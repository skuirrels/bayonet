//! Publishing (BLD-6): the project's publish profiles (`Properties/PublishProfiles/*.pubxml`),
//! a folder, or a container image through the SDK's container support, each as a
//! `dotnet publish` command line for the Run tool window.

use ide_core::IpcPath;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;

/// A `.pubxml` of the project.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PublishProfile {
    /// File name without `.pubxml`: what `-p:PublishProfile` takes.
    pub name: String,
    pub path: IpcPath,
    /// `FileSystem`, `Container`, `MSDeploy`, `ZipDeploy`, …
    pub method: Option<String>,
    /// Where it publishes: a folder, a registry, a site.
    pub target: Option<String>,
    pub configuration: Option<String>,
    pub runtime: Option<String>,
}

/// What to publish to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PublishTarget {
    #[serde(rename_all = "camelCase")]
    Profile { name: String, configuration: Option<String> },
    #[serde(rename_all = "camelCase")]
    Folder { output: String, configuration: String, runtime: Option<String>, self_contained: bool },
    /// An image for the local container runtime (`PublishContainer`).
    #[serde(rename_all = "camelCase")]
    Container { configuration: String, repository: Option<String>, tag: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PublishCommand {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: IpcPath,
}

/// The text of the first `<name>` element, trimmed; unset when empty.
fn element(text: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let rest = &text[text.find(&open)? + open.len()..];
    let value = rest[..rest.find(&format!("</{name}>"))?].trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Reads one publish profile.
pub fn parse_profile(path: &Path, text: &str) -> PublishProfile {
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let method = element(text, "WebPublishMethod").or_else(|| element(text, "PublishProvider"));
    let target =
        ["PublishUrl", "PublishDir", "ContainerRegistry", "MSDeployServiceURL"].iter().find_map(|e| element(text, e));
    PublishProfile {
        name,
        path: IpcPath::from(path.to_path_buf()),
        method,
        target,
        configuration: element(text, "LastUsedBuildConfiguration").or_else(|| element(text, "Configuration")),
        runtime: element(text, "RuntimeIdentifier"),
    }
}

/// The publish profiles of the project file `project`, by name.
pub fn profiles(project: &Path) -> Vec<PublishProfile> {
    let Some(dir) = project.parent() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(dir.join("Properties").join("PublishProfiles")) else { return Vec::new() };
    let mut out: Vec<PublishProfile> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pubxml")))
        .filter_map(|p| Some(parse_profile(&p, &std::fs::read_to_string(&p).ok()?)))
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// `dotnet publish` of `project` to `target`.
pub fn command(dotnet: &Path, project: &Path, target: &PublishTarget) -> PublishCommand {
    let mut args = vec!["publish".to_owned(), project.to_string_lossy().into_owned()];
    match target {
        PublishTarget::Profile { name, configuration } => {
            if let Some(c) = configuration {
                args.extend(["-c".into(), c.clone()]);
            }
            args.push(format!("-p:PublishProfile={name}"));
        }
        PublishTarget::Folder { output, configuration, runtime, self_contained } => {
            args.extend(["-c".into(), configuration.clone(), "-o".into(), output.clone()]);
            if let Some(r) = runtime.as_deref().filter(|r| !r.is_empty()) {
                args.extend(["-r".into(), r.to_owned()]);
            }
            if runtime.as_deref().is_some_and(|r| !r.is_empty()) || *self_contained {
                args.push(if *self_contained { "--self-contained" } else { "--no-self-contained" }.into());
            }
        }
        PublishTarget::Container { configuration, repository, tag } => {
            args.extend(["-c".into(), configuration.clone(), "-t:PublishContainer".into()]);
            if let Some(r) = repository.as_deref().filter(|r| !r.is_empty()) {
                args.push(format!("-p:ContainerRepository={r}"));
            }
            if let Some(t) = tag.as_deref().filter(|t| !t.is_empty()) {
                args.push(format!("-p:ContainerImageTag={t}"));
            }
        }
    }
    let cwd = project.parent().unwrap_or(project).to_path_buf();
    PublishCommand { command: dotnet.to_string_lossy().into_owned(), args, cwd: IpcPath::from(cwd) }
}

/// The port ASP.NET Core listens on in its container images (`ASPNETCORE_HTTP_PORTS`).
pub const CONTAINER_HTTP_PORT: u16 = 8080;

/// For a web project, the host port to reach it on: the first `http://` port of its
/// launch profiles, else 8080. `None` for other projects.
pub fn web_host_port(project: &Path) -> Option<u16> {
    let text = std::fs::read_to_string(project).ok()?;
    if !text.contains("Microsoft.NET.Sdk.Web") {
        return None;
    }
    let settings = project.parent()?.join("Properties").join("launchSettings.json");
    let port = std::fs::read_to_string(settings).ok().and_then(|t| {
        t.split("\"applicationUrl\"").skip(1).find_map(|rest| {
            let value = rest.split('"').nth(1)?;
            value
                .split(';')
                .find_map(|u| u.strip_prefix("http://")?.rsplit(':').next()?.trim_end_matches('/').parse().ok())
        })
    });
    Some(port.unwrap_or(CONTAINER_HTTP_PORT))
}

fn quote(arg: &str) -> String {
    if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@+".contains(c)) {
        arg.to_owned()
    } else {
        format!("\"{arg}\"")
    }
}

/// Run in Container (PLT-5): one shell line that publishes `project` as image
/// `repository` (the SDK's container support, no Dockerfile) and then runs it, with
/// `host_port` mapped to the app's port. `&&` works in `sh` and `cmd` alike.
pub fn container_run(dotnet: &Path, project: &Path, repository: &str, host_port: Option<u16>) -> PublishCommand {
    let publish = command(
        dotnet,
        project,
        &PublishTarget::Container {
            configuration: "Release".into(),
            repository: Some(repository.to_owned()),
            tag: Some("dev".into()),
        },
    );
    let mut line: Vec<String> = std::iter::once(&publish.command).chain(&publish.args).map(|a| quote(a)).collect();
    line.extend(["&&", "docker", "run", "--rm", "-i"].map(String::from));
    if let Some(port) = host_port {
        line.extend(["-p".into(), format!("{port}:{CONTAINER_HTTP_PORT}")]);
    }
    line.push(format!("{repository}:dev"));
    PublishCommand { command: line.join(" "), args: Vec::new(), cwd: publish.cwd }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLDER: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<Project>
  <PropertyGroup>
    <Configuration>Release</Configuration>
    <Platform>Any CPU</Platform>
    <PublishDir>bin\Release\net10.0\publish\</PublishDir>
    <PublishProtocol>FileSystem</PublishProtocol>
    <RuntimeIdentifier>linux-x64</RuntimeIdentifier>
  </PropertyGroup>
</Project>"#;
    const WEB: &str = r#"<Project><PropertyGroup>
    <WebPublishMethod>FileSystem</WebPublishMethod>
    <LastUsedBuildConfiguration>Debug</LastUsedBuildConfiguration>
    <PublishUrl>  </PublishUrl>
    <PublishDir>out</PublishDir>
  </PropertyGroup></Project>"#;

    #[test]
    fn profiles_say_where_they_publish() {
        let p = parse_profile(Path::new("/s/Web/Properties/PublishProfiles/FolderProfile.pubxml"), FOLDER);
        assert_eq!(p.name, "FolderProfile");
        assert_eq!(p.target.as_deref(), Some("bin\\Release\\net10.0\\publish\\"));
        assert_eq!((p.configuration.as_deref(), p.runtime.as_deref()), (Some("Release"), Some("linux-x64")));
        let w = parse_profile(Path::new("/s/Web/Properties/PublishProfiles/IIS.pubxml"), WEB);
        assert_eq!(w.method.as_deref(), Some("FileSystem"));
        assert_eq!(w.target.as_deref(), Some("out"), "an empty PublishUrl is no target");
        assert_eq!(w.configuration.as_deref(), Some("Debug"));
    }

    #[test]
    fn lists_the_pubxml_files_of_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let profiles_dir = dir.path().join("Properties/PublishProfiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();
        std::fs::write(profiles_dir.join("b.pubxml"), FOLDER).unwrap();
        std::fs::write(profiles_dir.join("A.pubxml"), WEB).unwrap();
        std::fs::write(profiles_dir.join("A.pubxml.user"), WEB).unwrap();
        let names: Vec<String> = profiles(&dir.path().join("Web.csproj")).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["A", "b"]);
        assert!(profiles(&dir.path().join("none/X.csproj")).is_empty());
    }

    #[test]
    fn publish_command_lines() {
        let (dotnet, project) = (Path::new("dotnet"), Path::new("/s/Web/Web.csproj"));
        let args = |t: PublishTarget| command(dotnet, project, &t).args[2..].join(" ");
        assert_eq!(
            args(PublishTarget::Profile { name: "FolderProfile".into(), configuration: Some("Release".into()) }),
            "-c Release -p:PublishProfile=FolderProfile"
        );
        assert_eq!(
            args(PublishTarget::Folder {
                output: "/out".into(),
                configuration: "Release".into(),
                runtime: Some("osx-arm64".into()),
                self_contained: true
            }),
            "-c Release -o /out -r osx-arm64 --self-contained"
        );
        assert_eq!(
            args(PublishTarget::Folder {
                output: "/out".into(),
                configuration: "Debug".into(),
                runtime: None,
                self_contained: false
            }),
            "-c Debug -o /out"
        );
        assert_eq!(
            args(PublishTarget::Container {
                configuration: "Release".into(),
                repository: Some("shop/web".into()),
                tag: Some("1.2".into())
            }),
            "-c Release -t:PublishContainer -p:ContainerRepository=shop/web -p:ContainerImageTag=1.2"
        );
        assert_eq!(
            command(dotnet, project, &PublishTarget::Profile { name: "x".into(), configuration: None }).cwd,
            IpcPath::from("/s/Web")
        );
    }

    #[test]
    fn run_in_container_publishes_then_runs_the_image() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("My Web").join("Web.csproj");
        std::fs::create_dir_all(project.parent().unwrap().join("Properties")).unwrap();
        std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk.Web\"></Project>").unwrap();
        assert_eq!(web_host_port(&project), Some(8080), "no launch profiles");
        std::fs::write(
            project.parent().unwrap().join("Properties/launchSettings.json"),
            r#"{"profiles":{"https":{"applicationUrl":"https://localhost:7001;http://localhost:5123"}}}"#,
        )
        .unwrap();
        assert_eq!(web_host_port(&project), Some(5123));
        let cmd = container_run(Path::new("/usr/local/share/dotnet/dotnet"), &project, "web", Some(5123));
        assert!(cmd.args.is_empty(), "one shell line");
        let line = cmd.command;
        assert!(line.starts_with("/usr/local/share/dotnet/dotnet publish \""), "{line}");
        let tail = "\" -c Release -t:PublishContainer -p:ContainerRepository=web -p:ContainerImageTag=dev";
        assert!(line.contains(&format!("{}{tail}", Path::new("My Web").join("Web.csproj").display())), "{line}");
        assert!(line.ends_with("&& docker run --rm -i -p 5123:8080 web:dev"), "{line}");

        let console = dir.path().join("Tool.csproj");
        std::fs::write(&console, "<Project Sdk=\"Microsoft.NET.Sdk\"></Project>").unwrap();
        assert_eq!(web_host_port(&console), None);
        assert!(
            container_run(Path::new("dotnet"), &console, "tool", None).command.ends_with("docker run --rm -i tool:dev")
        );
    }
}
