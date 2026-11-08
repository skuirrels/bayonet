//! NuGet restore outside a build (NUG-1): which projects need one, the `dotnet restore`
//! command line, and the errors and warnings in its output.

use crate::dependencies::assets_file;
use ide_core::IpcPath;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RestoreSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RestoreMessage {
    pub severity: RestoreSeverity,
    /// `NU1301`, `MSB3202`.
    pub code: Option<String>,
    pub text: String,
    /// The project (or solution) the message is about, when the output names one.
    pub project: Option<IpcPath>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub success: bool,
    pub messages: Vec<RestoreMessage>,
}

/// Files next to or above a project whose change can change what restore resolves.
const RESTORE_INPUTS: [&str; 5] =
    ["Directory.Packages.props", "Directory.Build.props", "Directory.Build.targets", "NuGet.config", "nuget.config"];

/// `dotnet restore <target>` for a project or solution, without the terminal logger
/// (its output is not line based) or a build server left running afterwards.
pub fn restore_args(target: &Path) -> Vec<String> {
    vec![
        "restore".into(),
        target.to_string_lossy().into_owned(),
        "-tl:off".into(),
        "--disable-build-servers".into(),
        "-nologo".into(),
    ]
}

/// When this process last restored each project without errors. A restore with nothing to
/// do ("All projects are up-to-date") leaves `project.assets.json` as it was, so its age
/// alone would keep a project with an edited project file stale for good.
static RESTORED: LazyLock<Mutex<HashMap<PathBuf, SystemTime>>> = LazyLock::new(Default::default);

/// Records that `projects` were restored without errors by a restore that started `at`.
pub fn mark_restored(projects: impl IntoIterator<Item = PathBuf>, at: SystemTime) {
    let mut restored = RESTORED.lock().unwrap_or_else(|e| e.into_inner());
    for p in projects {
        restored.insert(p, at);
    }
}

/// The projects a restore of `target` restores: the project, or a solution's projects.
pub fn restored_by(target: &Path) -> Vec<PathBuf> {
    if crate::format_of(target).is_some() {
        crate::read(target).map(|s| s.projects()).unwrap_or_default()
    } else {
        vec![target.to_path_buf()]
    }
}

/// The project has no `project.assets.json`, or was last restored before a change to the
/// project file or a `Directory.*.props` / `nuget.config` above it.
pub fn needs_restore(project: &Path) -> bool {
    let Some(assets) = assets_file(project) else { return true };
    let Some(written) = modified(&assets) else { return true };
    let marked = RESTORED.lock().unwrap_or_else(|e| e.into_inner()).get(project).copied();
    let restored = marked.map_or(written, |m| m.max(written));
    let dir = project.parent().unwrap_or(project);
    let inputs = std::iter::once(project.to_path_buf())
        .chain(dir.ancestors().flat_map(|a| RESTORE_INPUTS.iter().map(move |n| a.join(n))));
    inputs.filter_map(|p| modified(&p)).any(|m| m > restored)
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Errors and warnings in `dotnet restore` output, each once (the console logger repeats
/// them in its summary). MSBuild's format: `origin(pos): error CODE: text [project]`.
pub fn parse_output(text: &str) -> Vec<RestoreMessage> {
    let mut out: Vec<RestoreMessage> = Vec::new();
    for line in text.lines() {
        if let Some(m) = message(line)
            && !out.contains(&m)
        {
            out.push(m);
        }
    }
    out
}

fn message(line: &str) -> Option<RestoreMessage> {
    let line = line.trim();
    let (severity, at) = [(RestoreSeverity::Error, " error "), (RestoreSeverity::Warning, " warning ")]
        .into_iter()
        .chain([(RestoreSeverity::Error, "error "), (RestoreSeverity::Warning, "warning ")])
        .find_map(|(s, marker)| {
            if marker.starts_with(' ') {
                line.find(&format!(":{marker}")).map(|i| (s, i + 1))
            } else {
                line.starts_with(marker).then_some((s, 0))
            }
        })?;
    let origin = line[..at].trim_end_matches(':').trim();
    let rest = line[at..].trim_start();
    let rest = rest.split_once(' ').map(|(_, r)| r)?;
    let (code, text) = match rest.split_once(':') {
        Some((code, text)) if !code.trim().is_empty() && !code.trim().contains(' ') => (Some(code.trim()), text),
        _ => (None, rest.strip_prefix(':').unwrap_or(rest)),
    };
    let (text, bracket) = match text.trim().rsplit_once(" [") {
        Some((t, p)) if p.ends_with(']') => (t.trim(), Some(p.trim_end_matches(']'))),
        _ => (text.trim(), None),
    };
    // The message's project: the origin when it is a project file, else the bracketed one.
    let origin = origin.strip_suffix(')').and_then(|o| o.rsplit_once('(')).map_or(origin, |(o, _)| o.trim());
    let project = [Some(origin), bracket]
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .find(|p| p.is_absolute() && (crate::is_project_file(p) || crate::format_of(p).is_some()))
        .map(IpcPath::from);
    Some(RestoreMessage { severity, code: code.map(str::to_owned), text: text.to_owned(), project })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "  Determining projects to restore...
/s/App/App.csproj : error NU1301: Unable to load the service index for source https://feed.example/v3/index.json. [/s/App.slnx]
/s/App/App.csproj : warning NU1603: App depends on Serilog (>= 4.0.0) but Serilog 4.0.0 was not found. [/s/App.slnx]
/usr/local/share/dotnet/sdk/10.0.403/NuGet.targets(521,5): error MSB3202: The project file \"/s/Gone/Gone.csproj\" was not found. [/s/App.slnx]
  Failed to restore /s/App/App.csproj (in 1.2 sec).

Build FAILED.

/s/App/App.csproj : error NU1301: Unable to load the service index for source https://feed.example/v3/index.json. [/s/App.slnx]
    1 Warning(s)
    2 Error(s)
";

    #[test]
    fn parses_nuget_and_msbuild_messages_once() {
        let messages = parse_output(OUTPUT);
        let summary: Vec<_> =
            messages.iter().map(|m| (m.severity, m.code.as_deref(), m.project.clone().map(PathBuf::from))).collect();
        assert_eq!(
            summary,
            [
                (RestoreSeverity::Error, Some("NU1301"), Some(PathBuf::from("/s/App/App.csproj"))),
                (RestoreSeverity::Warning, Some("NU1603"), Some(PathBuf::from("/s/App/App.csproj"))),
                // From an SDK targets file: about the solution in brackets.
                (RestoreSeverity::Error, Some("MSB3202"), Some(PathBuf::from("/s/App.slnx"))),
            ]
        );
        assert_eq!(messages[0].text, "Unable to load the service index for source https://feed.example/v3/index.json.");
    }

    #[test]
    fn reads_messages_without_an_origin() {
        let messages =
            parse_output("error NU1100: Unable to resolve 'Foo (>= 1.0.0)' for 'net10.0'.\nwarning : something odd");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].code.as_deref(), Some("NU1100"));
        assert_eq!(messages[0].project, None);
        assert_eq!(messages[1].severity, RestoreSeverity::Warning);
        assert_eq!(messages[1].text, "something odd");
        assert!(
            parse_output("  Restored /s/App/App.csproj (in 80 ms).\n  All projects are up-to-date for restore.")
                .is_empty()
        );
    }

    #[test]
    fn a_project_needs_restore_until_its_assets_are_newer_than_its_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("App");
        std::fs::create_dir_all(app.join("obj")).unwrap();
        let project = app.join("App.csproj");
        std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk\" />").unwrap();
        assert!(needs_restore(&project), "never restored");

        let assets = app.join("obj/project.assets.json");
        std::fs::write(&assets, "{}").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(60);
        std::fs::File::options().write(true).open(&assets).unwrap().set_modified(later).unwrap();
        assert!(!needs_restore(&project), "restored after the last change");

        let props = dir.path().join("Directory.Packages.props");
        std::fs::write(&props, "<Project />").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&props)
            .unwrap()
            .set_modified(later + std::time::Duration::from_secs(60))
            .unwrap();
        assert!(needs_restore(&project), "central versions changed since");

        // A restore with nothing to do leaves the assets file alone.
        mark_restored([project.clone()], later + std::time::Duration::from_secs(120));
        assert!(!needs_restore(&project), "restored since, without changes");
    }

    #[test]
    fn restores_without_the_terminal_logger() {
        assert_eq!(
            restore_args(Path::new("/s/App.slnx")),
            ["restore", "/s/App.slnx", "-tl:off", "--disable-build-servers", "-nologo"]
        );
    }
}
