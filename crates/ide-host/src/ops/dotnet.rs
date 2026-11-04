//! .NET solutions (what the Solution Explorer shows) and NuGet packages.
use crate::Host;
use crate::rpc::Dispatcher;

use ide_core::RequestId;
use ide_core::{IdeError, IdeResult, IpcPath};
use ide_dotnet::Solution;
use ide_dotnet::nuget::{self, InstalledPackages, PackageSearch};
use ide_dotnet::templates::{self, ProjectTemplate, TemplateValue};
use ide_dotnet::testing::{self, SessionRequest, TestApp, TestEvent, TestPhase, TestProject, TestSelection};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};

use super::Channel;

/// Solution files at the project root, `.slnx` before `.sln` before `.slnf`.
pub async fn dotnet_solutions(host: &Host, window: &str) -> IdeResult<Vec<IpcPath>> {
    let root = host.project(window)?.root.clone();
    host.blocking(move || Ok(ide_dotnet::find(&root).into_iter().map(IpcPath::from).collect())).await
}

/// The solution at `path`: its folders, projects and solution items.
pub async fn dotnet_solution(host: &Host, _window: &str, path: IpcPath) -> IdeResult<Solution> {
    host.blocking(move || ide_dotnet::read(&path)).await
}

/// The project root and the `dotnet` the project uses (Settings, `global.json`, else PATH).
fn dotnet_of(host: &Host, window: &str) -> IdeResult<(PathBuf, Option<PathBuf>)> {
    let project = host.project(window)?;
    let root = project.root.clone();
    let dotnet = ide_sdk::apply::dotnet_command(&root, &project.settings.effective());
    Ok((root, dotnet))
}

/// Searches the package sources of the project's `nuget.config` (`skip` / `take` page).
pub async fn nuget_search(
    host: &Host,
    window: &str,
    query: String,
    prerelease: bool,
    skip: u32,
    take: u32,
) -> IdeResult<PackageSearch> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let out = nuget::run(dotnet.as_deref(), &root, &nuget::search_args(&query, prerelease, skip, take))?;
        nuget::parse_search(&out)
    })
    .await
}

/// Every version of package `id`, newest first.
pub async fn nuget_versions(host: &Host, window: &str, id: String, prerelease: bool) -> IdeResult<Vec<String>> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        nuget::parse_versions(&nuget::run(dotnet.as_deref(), &root, &nuget::versions_args(&id, prerelease))?)
    })
    .await
}

/// The packages of every project of the solution at the root (else of the root's
/// project); `outdated` asks the sources for the latest versions too.
pub async fn nuget_installed(
    host: &Host,
    window: &str,
    outdated: bool,
    prerelease: bool,
) -> IdeResult<InstalledPackages> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let target = ide_dotnet::find(&root).into_iter().next().unwrap_or_else(|| root.clone());
        let out = nuget::run(dotnet.as_deref(), &root, &nuget::list_args(&target, outdated, prerelease))?;
        Ok(nuget::with_central_packages(nuget::parse_list(&out)?))
    })
    .await
}

/// Installs package `id` into `projects`, or moves them to `version` (else the latest
/// stable). Projects under central package management get the version in `Directory.Packages.props`.
pub async fn nuget_install(
    host: &Host,
    window: &str,
    projects: Vec<IpcPath>,
    id: String,
    version: Option<String>,
) -> IdeResult<()> {
    let (root, dotnet) = dotnet_of(host, window)?;
    if projects.is_empty() {
        return Err(IdeError::invalid("no project to install into"));
    }
    host.blocking(move || {
        for p in &projects {
            nuget::run(dotnet.as_deref(), &root, &nuget::add_args(p, &id, version.as_deref()))?;
        }
        Ok(())
    })
    .await
}

/// Removes package `id` from `projects`.
pub async fn nuget_remove(host: &Host, window: &str, projects: Vec<IpcPath>, id: String) -> IdeResult<()> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        for p in &projects {
            nuget::run(dotnet.as_deref(), &root, &nuget::remove_args(p, &id))?;
        }
        Ok(())
    })
    .await
}

/// The solution at the project root, required by solution edits.
fn solution_of(root: &Path) -> IdeResult<PathBuf> {
    ide_dotnet::find(root).into_iter().next().ok_or_else(|| IdeError::not_found("the project has no solution file"))
}

/// Installed project and solution templates (`dotnet new`), custom ones included.
pub async fn dotnet_templates(host: &Host, window: &str) -> IdeResult<Vec<ProjectTemplate>> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let version = nuget::run(dotnet.as_deref(), &root, &["--version".to_owned()])?.trim().to_owned();
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .ok_or_else(|| IdeError::not_found("no home directory"))?;
        let file = templates::cache_file(&home, &version);
        if !file.is_file() {
            // The cache is written on first use of `dotnet new`.
            nuget::run(dotnet.as_deref(), &root, &["new".into(), "list".into()])?;
        }
        templates::parse_cache(&std::fs::read_to_string(&file)?)
    })
    .await
}

/// Creates a project from `template` in `directory` and, with `add_to_solution`, adds
/// it to the root's solution (in `solution_folder`). Returns the new project file.
#[allow(clippy::too_many_arguments)]
pub async fn dotnet_new_project(
    host: &Host,
    window: &str,
    template: String,
    name: String,
    directory: IpcPath,
    language: Option<String>,
    values: Vec<TemplateValue>,
    add_to_solution: bool,
    solution_folder: Option<String>,
) -> IdeResult<IpcPath> {
    let (root, dotnet) = dotnet_of(host, window)?;
    if name.trim().is_empty() || name.contains(['/', '\\']) {
        return Err(IdeError::invalid("enter a project name"));
    }
    host.blocking(move || {
        let args = templates::new_args(&template, &name, &directory, language.as_deref(), &values);
        nuget::run(dotnet.as_deref(), &root, &args)?;
        let project = templates::created_project(&directory, &name)
            .ok_or_else(|| IdeError::not_found(format!("{template} created no project in {}", directory.display())))?;
        if add_to_solution {
            let solution = solution_of(&root)?;
            nuget::run(
                dotnet.as_deref(),
                &root,
                &templates::sln_add_args(&solution, std::slice::from_ref(&project), solution_folder.as_deref()),
            )?;
        }
        Ok(IpcPath::from(project))
    })
    .await
}

/// Adds existing projects to the root's solution.
pub async fn dotnet_solution_add(
    host: &Host,
    window: &str,
    projects: Vec<IpcPath>,
    solution_folder: Option<String>,
) -> IdeResult<()> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let solution = solution_of(&root)?;
        let paths: Vec<PathBuf> = projects.into_iter().map(PathBuf::from).collect();
        nuget::run(dotnet.as_deref(), &root, &templates::sln_add_args(&solution, &paths, solution_folder.as_deref()))
            .map(|_| ())
    })
    .await
}

/// Removes projects from the root's solution (their files stay).
pub async fn dotnet_solution_remove(host: &Host, window: &str, projects: Vec<IpcPath>) -> IdeResult<()> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let solution = solution_of(&root)?;
        let paths: Vec<PathBuf> = projects.into_iter().map(PathBuf::from).collect();
        nuget::run(dotnet.as_deref(), &root, &templates::sln_remove_args(&solution, &paths)).map(|_| ())
    })
    .await
}

/// Converts a `.sln` to `.slnx` next to it (SLN-6); the `.sln` stays. Returns the new file.
pub async fn dotnet_migrate_solution(host: &Host, window: &str, solution: IpcPath) -> IdeResult<IpcPath> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let solution = PathBuf::from(solution);
        let target = templates::migrated_solution(&solution);
        if target.exists() {
            return Err(IdeError::new(
                ide_core::error::ErrorKind::AlreadyExists,
                format!("{} already exists", target.file_name().unwrap_or_default().to_string_lossy()),
            ));
        }
        nuget::run(dotnet.as_deref(), &root, &templates::sln_migrate_args(&solution))?;
        Ok(IpcPath::from(target))
    })
    .await
}

/// The projects of the root's solution (none without one).
fn solution_projects(root: &Path) -> Vec<PathBuf> {
    ide_dotnet::find(root)
        .into_iter()
        .next()
        .and_then(|s| ide_dotnet::read(&s).ok())
        .map(|s| s.projects())
        .unwrap_or_default()
}

/// The solution's projects that use Entity Framework Core (PLT-2).
pub async fn dotnet_ef_projects(host: &Host, window: &str) -> IdeResult<Vec<IpcPath>> {
    let root = host.project(window)?.root.clone();
    host.blocking(move || {
        Ok(solution_projects(&root)
            .into_iter()
            .filter(|p| p.parent().is_some_and(ide_dotnet::ef::uses_ef_core))
            .map(IpcPath::from)
            .collect())
    })
    .await
}

/// The migrations of `project`, oldest first.
pub async fn dotnet_ef_migrations(host: &Host, _window: &str, project: IpcPath) -> IdeResult<Vec<String>> {
    host.blocking(move || Ok(ide_dotnet::ef::migrations(project.parent().unwrap_or(&project)))).await
}

/// The `dotnet ef` command line of `operation` on `project` (run in the Run tool window).
pub async fn dotnet_ef_command(
    host: &Host,
    window: &str,
    project: IpcPath,
    operation: ide_dotnet::ef::EfOperation,
) -> IdeResult<ide_dotnet::ef::EfCommand> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let dotnet = dotnet.unwrap_or_else(|| PathBuf::from("dotnet"));
        Ok(ide_dotnet::ef::command(&dotnet, &operation, &project, &solution_projects(&root)))
    })
    .await
}

/// Where the .NET SDK the project uses defines MSBuild target `name` (CI-18).
pub async fn dotnet_sdk_target(
    host: &Host,
    window: &str,
    name: String,
) -> IdeResult<Option<ide_dotnet::msbuild::FileLocation>> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let dotnet = dotnet.unwrap_or_else(|| PathBuf::from("dotnet"));
        let Some(sdk) = ide_dotnet::msbuild::sdk_dir(&dotnet, &root) else { return Ok(None) };
        Ok(ide_dotnet::msbuild::find_target(&ide_dotnet::msbuild::sdk_files(&sdk), &name))
    })
    .await
}

/// Reads the MSBuild binary log at `path` for the binary log viewer (BLD-3).
pub async fn dotnet_binlog_read(host: &Host, path: PathBuf) -> IdeResult<ide_dotnet::binlog::Binlog> {
    host.blocking(move || ide_dotnet::binlog::read(&path)).await
}

/// The binary logs builds of this project wrote (Settings › .NET › binary log), newest first.
pub async fn dotnet_binlogs(host: &Host, window: &str) -> IdeResult<Vec<IpcPath>> {
    let dir = host.project(window)?.build_cache_dir().join("binlogs");
    host.blocking(move || {
        let mut logs: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "binlog"))
            .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
            .collect();
        logs.sort_by_key(|l| std::cmp::Reverse(l.0));
        Ok(logs.into_iter().map(|(_, p)| IpcPath::from(p)).collect())
    })
    .await
}

/// Where the generated sources of each of `projects` go (CI-17), created
/// empty when missing so the tree can list them before the first compile.
pub async fn dotnet_generated_dirs(host: &Host, window: &str, projects: Vec<IpcPath>) -> IdeResult<Vec<IpcPath>> {
    let base = host.project(window)?.build_cache_dir().join("generated");
    host.blocking(move || {
        Ok(projects
            .into_iter()
            .map(|p| {
                let dir = ide_dotnet::generators::output_dir(&base, &PathBuf::from(p));
                let _ = std::fs::create_dir_all(&dir);
                IpcPath::from(dir)
            })
            .collect())
    })
    .await
}

/// One side compile at a time: they share the compiler server and the cache.
static GENERATING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Compiles `project` once more with `EmitCompilerGeneratedFiles` into the
/// IDE's cache and lists its analysers and generated files (CI-17). The
/// project's own `obj` and `bin` are left alone; see `ide_dotnet::generators`.
pub async fn dotnet_generated_sources(
    host: &Host,
    window: &str,
    project: IpcPath,
) -> IdeResult<ide_dotnet::generators::GeneratedSources> {
    use ide_dotnet::generators;
    let (_, dotnet) = dotnet_of(host, window)?;
    let dotnet = dotnet.unwrap_or_else(|| PathBuf::from("dotnet"));
    let base = host.project(window)?.build_cache_dir().join("generated");
    let project = PathBuf::from(project);
    let dir = generators::output_dir(&base, &project);
    let empty = |errors: Vec<String>| generators::GeneratedSources {
        dir: dir.clone().into(),
        analyzers: Vec::new(),
        files: Vec::new(),
        errors,
    };
    // F# has no source generators (and no Roslyn analysers).
    if !project.extension().is_some_and(|e| e.eq_ignore_ascii_case("csproj") || e.eq_ignore_ascii_case("vbproj")) {
        return Ok(empty(Vec::new()));
    }
    let _guard = GENERATING.lock().await;
    let info = {
        let p = project.clone();
        tokio::task::spawn_blocking(move || ide_dotnet::read_project(&p))
            .await
            .map_err(|e| IdeError::internal(e.to_string()))??
    };
    let work = base.join("work");
    let (obj, next, targets) = (work.join("obj"), work.join("out"), work.join("generated.targets"));
    generators::remove_listing(&work);
    std::fs::create_dir_all(&next).map_err(|e| IdeError::internal(format!("cannot create {}: {e}", next.display())))?;
    std::fs::write(&targets, generators::targets_file(&project, &obj, &next))
        .map_err(|e| IdeError::internal(format!("cannot write {}: {e}", targets.display())))?;
    let mut args: Vec<String> = vec![
        "msbuild".into(),
        project.to_string_lossy().into_owned(),
        "-nologo".into(),
        "-v:q".into(),
        "-t:Compile".into(),
        "-p:BuildProjectReferences=false".into(),
        format!("-p:CustomAfterMicrosoftCommonTargets={}", targets.display()),
        "-getItem:Analyzer".into(),
    ];
    if info.target_frameworks.len() > 1
        && let Some(first) = info.target_frameworks.first()
    {
        args.push(format!("-p:TargetFramework={first}"));
    }
    let out = tokio::process::Command::new(&dotnet)
        .args(&args)
        .current_dir(project.parent().unwrap_or(&project))
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_UI_LANGUAGE", "en")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", dotnet.display())))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let errors = generators::errors(&format!("{stdout}\n{}", String::from_utf8_lossy(&out.stderr)));
    let analyzers = generators::parse_analyzers(&stdout);
    if analyzers.is_empty() && !errors.is_empty() {
        return Ok(empty(errors));
    }
    host.blocking(move || {
        let names = generators::add_analyzer_folders(&next, &analyzers)
            .map_err(|e| IdeError::internal(format!("cannot write {}: {e}", next.display())))?;
        generators::make_read_only(&generators::generated_files(&next));
        // Swap the new listing in whole, so the tree never shows half of it.
        generators::remove_listing(&dir);
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent).map_err(|e| IdeError::internal(e.to_string()))?;
        }
        std::fs::rename(&next, &dir).map_err(|e| IdeError::internal(format!("cannot move {}: {e}", dir.display())))?;
        generators::remove_listing(&work);
        let files = generators::generated_files(&dir).into_iter().map(IpcPath::from).collect();
        Ok(generators::GeneratedSources { dir: dir.into(), analyzers: names, files, errors })
    })
    .await
}

/// The projects `project` references.
pub async fn dotnet_project_references(host: &Host, _window: &str, project: IpcPath) -> IdeResult<Vec<IpcPath>> {
    host.blocking(move || Ok(templates::project_references(&project)?.into_iter().map(IpcPath::from).collect())).await
}

/// Adds and removes project references of `project`.
pub async fn dotnet_edit_references(
    host: &Host,
    window: &str,
    project: IpcPath,
    add: Vec<IpcPath>,
    remove: Vec<IpcPath>,
) -> IdeResult<()> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        for (adding, list) in [(false, remove), (true, add)] {
            if !list.is_empty() {
                let paths: Vec<PathBuf> = list.into_iter().map(PathBuf::from).collect();
                nuget::run(dotnet.as_deref(), &root, &templates::reference_args(adding, &project, &paths))?;
            }
        }
        Ok(())
    })
    .await
}

/// The test projects of the solution at the root.
pub async fn dotnet_test_projects(host: &Host, window: &str) -> IdeResult<Vec<TestProject>> {
    let root = host.project(window)?.root.clone();
    host.blocking(move || {
        let solution = ide_dotnet::find(&root).into_iter().next();
        Ok(testing::test_projects(solution.as_deref(), &root))
    })
    .await
}

/// Builds, then discovers (`run` false) or runs the tests of `selection` (every
/// test project when empty), streaming progress and results on `on_event`.
/// Cancellable with `cancel_request(request)`.
pub async fn dotnet_tests(
    host: &Host,
    window: &str,
    request: RequestId,
    run: bool,
    coverage: bool,
    selection: Vec<TestSelection>,
    on_event: Channel<TestEvent>,
) -> IdeResult<()> {
    let (root, dotnet) = dotnet_of(host, window)?;
    let token = host.begin_request(request);
    let coverage = (run && coverage).then(|| coverage_dir(request));
    let result = tests_inner(&root, dotnet, run, coverage.as_deref(), selection, &token, &on_event).await;
    host.end_request(request);
    if let Err(e) = &result {
        let _ = on_event.send(TestEvent::Phase {
            project: None,
            phase: TestPhase::Failed,
            message: Some(e.message.clone()),
        });
    }
    result
}

/// Where a coverage run writes its reports: a fresh folder per run.
fn coverage_dir(request: RequestId) -> PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    std::env::temp_dir().join("dotnetidea-coverage").join(format!("{}-{}-{stamp}", std::process::id(), request.0))
}

async fn tests_inner(
    root: &Path,
    dotnet: Option<PathBuf>,
    run: bool,
    coverage: Option<&Path>,
    selection: Vec<TestSelection>,
    token: &ide_core::CancellationToken,
    on_event: &Channel<TestEvent>,
) -> IdeResult<()> {
    let dotnet = dotnet.unwrap_or_else(|| PathBuf::from("dotnet"));
    let solution = ide_dotnet::find(root).into_iter().next();
    let selection = if selection.is_empty() {
        let (s, r) = (solution.clone(), root.to_path_buf());
        tokio::task::spawn_blocking(move || testing::test_projects(s.as_deref(), &r))
            .await
            .map_err(|e| IdeError::internal(e.to_string()))?
            .into_iter()
            .map(|p| TestSelection { project: p.path, uids: Vec::new() })
            .collect()
    } else {
        selection
    };
    if selection.is_empty() {
        return Err(IdeError::not_found("no test projects in the solution"));
    }
    let send = |e: TestEvent| {
        let _ = on_event.send(e);
    };
    send(TestEvent::Phase { project: None, phase: TestPhase::Building, message: None });
    // One build of the solution when every test project is wanted, else of each project.
    let targets: Vec<PathBuf> = match &solution {
        Some(s) if selection.iter().all(|t| t.uids.is_empty()) && selection.len() > 1 => vec![s.clone()],
        _ => selection.iter().map(|t| t.project.to_path_buf()).collect(),
    };
    for target in &targets {
        let mut args: Vec<String> = vec!["build".into(), target.to_string_lossy().into_owned()];
        args.extend(["-nologo", "-tl:off", "-v:minimal", "-clp:NoSummary"].map(String::from));
        let out = on_event.clone();
        let on_line = move |line| {
            let _ = out.send(TestEvent::Output { project: None, line });
        };
        let code = stream_dotnet(&dotnet, root, &args, token, on_line).await?;
        if code != 0 {
            return Err(IdeError::new(ide_core::error::ErrorKind::Io, "build failed; see the output"));
        }
    }
    for t in &selection {
        if token.is_cancelled() {
            return Err(IdeError::cancelled());
        }
        let project = t.project.clone();
        let phase =
            |phase, message: Option<String>| TestEvent::Phase { project: Some(project.clone()), phase, message };
        send(phase(if run { TestPhase::Running } else { TestPhase::Discovering }, None));
        let mut app = match test_app(&dotnet, &t.project).await {
            Ok(app) => app,
            Err(e) => {
                send(phase(TestPhase::Failed, Some(e.message)));
                continue;
            }
        };
        let request = if run { SessionRequest::Run(t.uids.clone()) } else { SessionRequest::Discover };
        let dir = t.project.parent().map(Path::to_path_buf).unwrap_or_default();
        // Projects without the coverage extension run without coverage (the window says so).
        let report = coverage.filter(|_| testing::has_coverage_extension(&dir)).map(|out| {
            let name = format!(
                "{}.cobertura.xml",
                t.project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
            );
            app.args = testing::coverage_args(out, &name);
            out.join(name)
        });
        let out = on_event.clone();
        let out_project = project.clone();
        let result = testing::session(
            &app,
            request,
            token,
            |mut tests| {
                if !run {
                    testing::resolve_locations(&dir, &mut tests);
                }
                send(TestEvent::Tests { project: project.clone(), tests });
            },
            move |line| {
                let _ = out.send(TestEvent::Output { project: Some(out_project.clone()), line });
            },
        )
        .await;
        if let Some(report) = report.filter(|r| r.is_file()) {
            send(TestEvent::Coverage { project: project.clone(), report: report.into() });
        }
        match result {
            Ok(()) => send(phase(TestPhase::Finished, None)),
            Err(e) if e.kind == ide_core::error::ErrorKind::Cancelled => return Err(e),
            Err(e) => send(phase(TestPhase::Failed, Some(e.message))),
        }
    }
    send(TestEvent::Phase { project: None, phase: TestPhase::Finished, message: None });
    Ok(())
}

/// The built test app of `project`: its `TargetPath` (first target framework when it has several).
async fn test_app(dotnet: &Path, project: &Path) -> IdeResult<TestApp> {
    let names = ["TargetPath", "IsTestingPlatformApplication"];
    let mut args: Vec<String> = vec!["msbuild".into(), project.to_string_lossy().into_owned()];
    args.extend(names.iter().map(|n| format!("-getProperty:{n}")));
    let info = {
        let p = project.to_path_buf();
        tokio::task::spawn_blocking(move || ide_dotnet::read_project(&p))
            .await
            .map_err(|e| IdeError::internal(e.to_string()))??
    };
    if info.target_frameworks.len() > 1
        && let Some(first) = info.target_frameworks.first()
    {
        args.push(format!("-p:TargetFramework={first}"));
    }
    let dir = project.parent().unwrap_or(project);
    let out = tokio::process::Command::new(dotnet)
        .args(&args)
        .current_dir(dir)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_UI_LANGUAGE", "en")
        .output()
        .await
        .map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", dotnet.display())))?;
    let props = testing::parse_msbuild_properties(&String::from_utf8_lossy(&out.stdout), &names);
    let get = |k: &str| props.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let assembly = get("TargetPath");
    if assembly.is_empty() || !Path::new(&assembly).is_file() {
        return Err(IdeError::not_found(format!("{} has no built test assembly", project.display())));
    }
    if !get("IsTestingPlatformApplication").eq_ignore_ascii_case("true") {
        return Err(IdeError::new(
            ide_core::error::ErrorKind::Unavailable,
            "this project runs on VSTest; only Microsoft.Testing.Platform test projects are supported so far",
        ));
    }
    Ok(TestApp {
        dotnet: dotnet.to_path_buf(),
        assembly: PathBuf::from(assembly),
        cwd: dir.to_path_buf(),
        env: Vec::new(),
        args: Vec::new(),
    })
}

/// Runs `dotnet <args>`, calling `on_line` per output line; the exit code (cancellation kills it).
async fn stream_dotnet(
    dotnet: &Path,
    cwd: &Path,
    args: &[String],
    token: &ide_core::CancellationToken,
    on_line: impl Fn(String) + Send + Sync + Clone + 'static,
) -> IdeResult<i32> {
    let mut child = tokio::process::Command::new(dotnet)
        .args(args)
        .current_dir(cwd)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_UI_LANGUAGE", "en")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", dotnet.display())))?;
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let f = on_line.clone();
        readers.push(tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                f(l);
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let f = on_line.clone();
        readers.push(tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                f(l);
            }
        }));
    }
    let status = tokio::select! {
        s = child.wait() => s.map_err(IdeError::from)?,
        () = token.cancelled() => {
            let _ = child.kill().await;
            return Err(IdeError::cancelled());
        }
    };
    for r in readers {
        let _ = r.await;
    }
    Ok(status.code().unwrap_or(-1))
}

pub(crate) fn register(d: &mut Dispatcher) {
    d.add("dotnet_templates", |c| async move { dotnet_templates(&c.host, &c.window).await });
    d.add("dotnet_new_project", |c| async move {
        dotnet_new_project(
            &c.host,
            &c.window,
            c.arg("template")?,
            c.arg("name")?,
            c.arg("directory")?,
            c.arg("language")?,
            c.arg("values")?,
            c.arg("addToSolution")?,
            c.arg("solutionFolder")?,
        )
        .await
    });
    d.add("dotnet_solution_add", |c| async move {
        dotnet_solution_add(&c.host, &c.window, c.arg("projects")?, c.arg("solutionFolder")?).await
    });
    d.add(
        "dotnet_solution_remove",
        |c| async move { dotnet_solution_remove(&c.host, &c.window, c.arg("projects")?).await },
    );
    d.add("dotnet_migrate_solution", |c| async move {
        dotnet_migrate_solution(&c.host, &c.window, c.arg("solution")?).await
    });
    d.add("dotnet_sdk_target", |c| async move { dotnet_sdk_target(&c.host, &c.window, c.arg("name")?).await });
    d.add("dotnet_binlog_read", |c| async move { dotnet_binlog_read(&c.host, c.arg("path")?).await });
    d.add(
        "dotnet_generated_dirs",
        |c| async move { dotnet_generated_dirs(&c.host, &c.window, c.arg("projects")?).await },
    );
    d.add("dotnet_generated_sources", |c| async move {
        dotnet_generated_sources(&c.host, &c.window, c.arg("project")?).await
    });
    d.add("dotnet_binlogs", |c| async move { dotnet_binlogs(&c.host, &c.window).await });
    d.add("dotnet_ef_projects", |c| async move { dotnet_ef_projects(&c.host, &c.window).await });
    d.add("dotnet_ef_migrations", |c| async move { dotnet_ef_migrations(&c.host, &c.window, c.arg("project")?).await });
    d.add("dotnet_ef_command", |c| async move {
        dotnet_ef_command(&c.host, &c.window, c.arg("project")?, c.arg("operation")?).await
    });
    d.add("dotnet_project_references", |c| async move {
        dotnet_project_references(&c.host, &c.window, c.arg("project")?).await
    });
    d.add("dotnet_edit_references", |c| async move {
        dotnet_edit_references(&c.host, &c.window, c.arg("project")?, c.arg("add")?, c.arg("remove")?).await
    });
    d.add("dotnet_test_projects", |c| async move { dotnet_test_projects(&c.host, &c.window).await });
    d.add("dotnet_tests", |c| async move {
        dotnet_tests(
            &c.host,
            &c.window,
            c.arg("request")?,
            c.arg("run")?,
            c.arg::<Option<bool>>("coverage")?.unwrap_or(false),
            c.arg("selection")?,
            c.sender("onEvent")?,
        )
        .await
    });
    d.add("nuget_search", |c| async move {
        nuget_search(&c.host, &c.window, c.arg("query")?, c.arg("prerelease")?, c.arg("skip")?, c.arg("take")?).await
    });
    d.add(
        "nuget_versions",
        |c| async move { nuget_versions(&c.host, &c.window, c.arg("id")?, c.arg("prerelease")?).await },
    );
    d.add("nuget_installed", |c| async move {
        nuget_installed(&c.host, &c.window, c.arg("outdated")?, c.arg("prerelease")?).await
    });
    d.add("nuget_install", |c| async move {
        nuget_install(&c.host, &c.window, c.arg("projects")?, c.arg("id")?, c.arg("version")?).await
    });
    d.add("nuget_remove", |c| async move { nuget_remove(&c.host, &c.window, c.arg("projects")?, c.arg("id")?).await });
    d.add("dotnet_solutions", |c| async move { dotnet_solutions(&c.host, &c.window).await });
    d.add("dotnet_solution", |c| async move { dotnet_solution(&c.host, &c.window, c.arg("path")?).await });
}
