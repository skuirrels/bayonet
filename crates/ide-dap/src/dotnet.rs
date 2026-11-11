//! `dotnet run` under netcoredbg (what the C# extension's `coreclr` launch does): a
//! launch request may carry `"dotnet": {"project": …, "launchProfile": …, "framework":
//! …, "configuration": …}` instead of `program`; we build the project, ask MSBuild for
//! its output and run settings, apply the launch profile as `dotnet run` would, and
//! launch the output assembly.

use crate::adapter::find_binary;
use ide_core::{IdeError, IdeResult};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct DotnetLaunch {
    /// A project file, or a directory holding one.
    pub project: PathBuf,
    pub launch_profile: Option<String>,
    pub framework: Option<String>,
    pub configuration: Option<String>,
}

impl DotnetLaunch {
    pub fn from_json(v: &Value) -> IdeResult<Self> {
        let get = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        let project = get("project").ok_or_else(|| IdeError::invalid("dotnet launch: `project` is required"))?;
        Ok(Self {
            project: project.into(),
            launch_profile: get("launchProfile"),
            framework: get("framework"),
            configuration: get("configuration"),
        })
    }
}

/// MSBuild's view of the project after a build: `-getProperty` results.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RunProperties {
    pub target_path: String,
    pub run_working_directory: String,
    pub run_arguments: String,
}

/// `dotnet msbuild -getProperty:A -getProperty:B` prints `{"Properties": {"A": …, "B": …}}`.
pub fn parse_properties(stdout: &str) -> Option<RunProperties> {
    let start = stdout.find('{')?;
    let v: Value = serde_json::from_str(stdout[start..].trim()).ok()?;
    let props = v.get("Properties")?;
    let get = |k: &str| props.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    Some(RunProperties {
        target_path: get("TargetPath"),
        run_working_directory: get("RunWorkingDirectory"),
        run_arguments: get("RunArguments"),
    })
}

/// The project file `launch.project` names: itself, or the one project in a directory.
pub fn project_file(project: &Path) -> IdeResult<PathBuf> {
    if project.is_file() {
        return Ok(project.to_path_buf());
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(project)
        .map_err(|e| IdeError::not_found(format!("{}: {e}", project.display())))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && ide_dotnet::is_project_file(p))
        .collect();
    found.sort();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(IdeError::not_found(format!("no project file in {}", project.display()))),
        _ => Err(IdeError::invalid(format!("{} holds several projects; choose one", project.display()))),
    }
}

/// Fills `arguments` (`program`, `args`, `cwd`, `env`) as `dotnet run` would start the
/// project: the launch profile's arguments unless the request has its own, its working
/// directory, else MSBuild's `RunWorkingDirectory`, else the project directory, and its
/// environment under the request's own.
pub fn apply(
    arguments: &mut Value,
    project: &Path,
    props: &RunProperties,
    profile: Option<&ide_dotnet::LaunchProfile>,
) -> IdeResult<()> {
    if props.target_path.is_empty() {
        return Err(IdeError::invalid(format!("{} has no output assembly to debug", project.display())));
    }
    let dir = project.parent().unwrap_or(Path::new("."));
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    let mut args: Vec<String> = Vec::new();
    let mut cwd = (!props.run_working_directory.is_empty()).then(|| dir.join(&props.run_working_directory));
    if let Some(p) = profile {
        env.extend(p.environment_variables.clone());
        env.insert("DOTNET_LAUNCH_PROFILE".into(), p.name.clone());
        if let Some(urls) = p.application_url.as_deref().filter(|u| !u.is_empty()) {
            env.entry("ASPNETCORE_URLS".into()).or_insert_with(|| urls.to_owned());
        }
        if let Some(a) = &p.command_line_args {
            args = shlex::split(a)
                .ok_or_else(|| IdeError::invalid(format!("launch profile `{}`: bad commandLineArgs", p.name)))?;
        }
        if let Some(wd) = p.working_directory.as_deref().filter(|w| !w.is_empty()) {
            cwd = Some(dir.join(wd));
        }
    }
    if args.is_empty() {
        args = shlex::split(&props.run_arguments).unwrap_or_default();
    }
    let obj = arguments.as_object_mut().ok_or_else(|| IdeError::invalid("launch arguments must be an object"))?;
    if let Some(Value::Object(own)) = obj.get("env") {
        for (k, v) in own {
            if let Some(v) = v.as_str() {
                env.insert(k.clone(), v.to_owned());
            }
        }
    }
    let own_args = obj.get("args").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
    if !own_args {
        obj.insert("args".into(), json!(args));
    }
    obj.insert("program".into(), json!(props.target_path));
    obj.insert("cwd".into(), json!(cwd.unwrap_or_else(|| dir.to_path_buf())));
    obj.insert("env".into(), json!(env));
    Ok(())
}

/// Builds `launch` and fills `arguments` to start its output. Build output is passed
/// to `on_output` line by line.
pub async fn prepare(
    launch: &DotnetLaunch,
    arguments: &mut Value,
    on_output: impl Fn(String) + Send + Sync + 'static,
) -> IdeResult<()> {
    let project = project_file(&launch.project)?;
    let dir = project.parent().unwrap_or(Path::new(".")).to_path_buf();
    let dotnet = find_binary("dotnet", &dir).ok_or_else(|| IdeError::unavailable("`dotnet` not found on PATH"))?;
    let configuration = launch.configuration.clone().unwrap_or_else(|| "Debug".into());
    // A multi-targeting project builds and runs its first framework unless one is chosen.
    let framework = launch.framework.clone().or_else(|| {
        let info = ide_dotnet::read_project(&project).ok()?;
        (info.target_frameworks.len() > 1).then(|| info.target_frameworks[0].clone())
    });
    let project_arg = project.to_string_lossy().into_owned();
    let mut build = vec!["build".to_owned(), project_arg.clone(), "-nologo".into(), "-tl:off".into()];
    build.extend(["-c".into(), configuration.clone()]);
    if let Some(f) = &framework {
        build.extend(["-f".into(), f.clone()]);
    }
    let on_output = std::sync::Arc::new(on_output);
    run(&dotnet, &build, &dir, Some(on_output.clone())).await?;
    let mut query = vec![
        "msbuild".to_owned(),
        project_arg,
        "-nologo".into(),
        "-getProperty:TargetPath".into(),
        "-getProperty:RunWorkingDirectory".into(),
        "-getProperty:RunArguments".into(),
        format!("-p:Configuration={configuration}"),
    ];
    if let Some(f) = &framework {
        query.push(format!("-p:TargetFramework={f}"));
    }
    let stdout = run(&dotnet, &query, &dir, None).await?;
    let props = parse_properties(&stdout)
        .ok_or_else(|| IdeError::invalid(format!("could not read the run settings of {}", project.display())))?;
    let profiles = ide_dotnet::launch_profiles(&dir);
    // `dotnet run` uses the first `Project` profile when none is named.
    let profile = match &launch.launch_profile {
        Some(name) => Some(
            profiles
                .iter()
                .find(|p| &p.name == name)
                .ok_or_else(|| IdeError::not_found(format!("launch profile `{name}` not found")))?,
        ),
        None => profiles.first(),
    };
    apply(arguments, &project, &props, profile)
}

/// Runs `dotnet args`, streaming its output to `on_output` (else collecting stdout).
async fn run(
    dotnet: &Path,
    args: &[String],
    cwd: &Path,
    on_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
) -> IdeResult<String> {
    let mut cmd = tokio::process::Command::new(dotnet);
    ide_core::process::prepare(cmd.as_std_mut());
    cmd.args(args)
        .current_dir(cwd)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_UI_LANGUAGE", "en")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(out) = &on_output {
        out(format!("> dotnet {}\n", args.join(" ")));
    }
    let mut child = cmd.spawn().map_err(|e| IdeError::unavailable(format!("failed to run dotnet: {e}")))?;
    let stderr = child.stderr.take().ok_or_else(|| IdeError::internal("dotnet stderr unavailable"))?;
    let stdout = child.stdout.take().ok_or_else(|| IdeError::internal("dotnet stdout unavailable"))?;
    let err_out = on_output.clone();
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut text = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            match &err_out {
                Some(out) => out(format!("{line}\n")),
                None => text.push_str(&format!("{line}\n")),
            }
        }
        text
    });
    let mut collected = String::new();
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        match &on_output {
            Some(out) => out(format!("{line}\n")),
            None => collected.push_str(&format!("{line}\n")),
        }
    }
    let err_text = err_task.await.unwrap_or_default();
    let status = child.wait().await?;
    if !status.success() {
        let detail = [collected.trim(), err_text.trim()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>();
        let sub = args.first().map_or("", String::as_str);
        return Err(IdeError::invalid(if detail.is_empty() {
            format!("dotnet {sub} failed ({status})")
        } else {
            format!("dotnet {sub} failed ({status}): {}", detail.join("\n"))
        }));
    }
    Ok(collected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_msbuild_properties() {
        let out = "  Determining projects to restore...\n{\n  \"Properties\": {\n    \"TargetPath\": \"/p/bin/Debug/net10.0/App.dll\",\n    \"RunWorkingDirectory\": \"\",\n    \"RunArguments\": \"--x 1\"\n  }\n}\n";
        let props = parse_properties(out).unwrap();
        assert_eq!(props.target_path, "/p/bin/Debug/net10.0/App.dll");
        assert_eq!(props.run_arguments, "--x 1");
        assert!(parse_properties("error").is_none());
    }

    #[test]
    fn launch_profile_shapes_the_request() {
        let profile = ide_dotnet::LaunchProfile {
            name: "https".into(),
            command_name: "Project".into(),
            command_line_args: Some("--seed \"a b\"".into()),
            working_directory: None,
            application_url: Some("https://localhost:7001".into()),
            environment_variables: BTreeMap::from([("ASPNETCORE_ENVIRONMENT".into(), "Development".into())]),
        };
        let props = RunProperties { target_path: "/p/App/bin/App.dll".into(), ..RunProperties::default() };
        let project = Path::new("/p/App/App.csproj");
        let mut args = json!({ "args": [], "env": { "ASPNETCORE_ENVIRONMENT": "Staging" }, "cwd": "/p" });
        apply(&mut args, project, &props, Some(&profile)).unwrap();
        assert_eq!(args["program"], "/p/App/bin/App.dll");
        assert_eq!(args["args"], json!(["--seed", "a b"]));
        assert_eq!(args["cwd"], "/p/App");
        assert_eq!(args["env"]["ASPNETCORE_ENVIRONMENT"], "Staging");
        assert_eq!(args["env"]["ASPNETCORE_URLS"], "https://localhost:7001");
        assert_eq!(args["env"]["DOTNET_LAUNCH_PROFILE"], "https");

        // The request's own arguments win; MSBuild's working directory applies without a profile.
        let props = RunProperties { run_working_directory: "data".into(), run_arguments: "--x".into(), ..props };
        let mut args = json!({ "args": ["--own"] });
        apply(&mut args, project, &props, None).unwrap();
        assert_eq!(
            (args["args"].clone(), args["cwd"].clone()),
            (json!(["--own"]), json!(Path::new("/p/App").join("data")))
        );
        let mut none = json!({});
        apply(&mut none, project, &props, None).unwrap();
        assert_eq!(none["args"], json!(["--x"]));
        assert!(apply(&mut json!({}), project, &RunProperties::default(), None).is_err());
    }

    #[test]
    fn a_directory_names_its_one_project() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(project_file(tmp.path()).is_err());
        std::fs::write(tmp.path().join("App.csproj"), "<Project />").unwrap();
        assert_eq!(project_file(tmp.path()).unwrap(), tmp.path().join("App.csproj"));
        std::fs::write(tmp.path().join("Other.fsproj"), "<Project />").unwrap();
        assert!(project_file(tmp.path()).is_err());
        let launch = DotnetLaunch::from_json(&json!({ "project": "/p/App.csproj", "framework": "" })).unwrap();
        assert_eq!((launch.project, launch.framework), (PathBuf::from("/p/App.csproj"), None));
        assert!(DotnetLaunch::from_json(&json!({})).is_err());
    }

    /// Against the machine's own .NET SDK, when there is one: the fixture's console app
    /// builds and resolves to its assembly.
    #[tokio::test]
    async fn prepares_the_fixture_cli() {
        if find_binary("dotnet", Path::new(".")).is_none() {
            return;
        }
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dotnet/src/Fixture.Cli");
        let launch = DotnetLaunch { project, ..DotnetLaunch::default() };
        let mut args = json!({ "args": [] });
        prepare(&launch, &mut args, |_| {}).await.unwrap();
        let program = args["program"].as_str().unwrap();
        assert!(program.ends_with("Fixture.Cli.dll"), "{program}");
        assert!(Path::new(program).is_file());
        assert!(args["cwd"].as_str().unwrap().ends_with("Fixture.Cli"));
    }
}
