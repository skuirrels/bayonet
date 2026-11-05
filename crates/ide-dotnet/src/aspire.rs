//! .NET Aspire AppHosts (BLD-5): which projects are AppHosts, and what a
//! running one reports through the Aspire CLI (`aspire ps`, `aspire describe`,
//! `aspire logs`, `aspire resource`), all as JSON. The CLI finds AppHosts
//! however they were started (`dotnet run` included) through their backchannel.
use crate::ProjectInfo;
use crate::ef::ToolInstall;
use ide_core::IpcPath;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use std::path::{Path, PathBuf};

/// An AppHost project: `Aspire.AppHost.Sdk`, alone or after `Microsoft.NET.Sdk`.
pub fn is_apphost(info: &ProjectInfo) -> bool {
    info.sdks.iter().any(|s| s.eq_ignore_ascii_case("Aspire.AppHost.Sdk"))
}

/// The Aspire CLI: its own install (`~/.aspire/bin`), a global tool, else the search path.
pub fn locate() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "aspire.exe" } else { "aspire" };
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    home.iter()
        .flat_map(|h| [h.join(".aspire").join("bin").join(exe), h.join(".dotnet").join("tools").join(exe)])
        .find(|p| p.is_file())
        .or_else(|| ide_core::process::find_executable("aspire", &ide_core::tool_path::search_path()))
}

/// How to install the CLI when it is missing.
pub fn install_command() -> ToolInstall {
    ToolInstall {
        command: "dotnet".to_owned(),
        args: ["tool", "install", "--global", "Aspire.Cli"].map(str::to_owned).to_vec(),
        cwd: None,
    }
}

/// A running AppHost (`aspire ps`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AspireAppHost {
    pub path: IpcPath,
    pub pid: Option<u32>,
    /// The dashboard's login URL (with its token).
    pub dashboard_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AspireUrl {
    pub name: String,
    pub url: String,
}

/// A command a resource offers (`start`, `stop`, `restart`, `rebuild`, custom ones).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AspireCommand {
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
}

/// One resource of a running AppHost (`aspire describe`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AspireResource {
    /// Unique (`api-wbqsvmeb`); replicas share a display name.
    pub name: String,
    pub display_name: String,
    /// `Project`, `Container`, `Executable`, `Parameter`, …
    pub resource_type: String,
    /// `Starting`, `Running`, `Exited`, `Finished`, `FailedToStart`, …
    pub state: Option<String>,
    /// `Healthy`, `Unhealthy`, `Degraded`.
    pub health: Option<String>,
    /// The project file, image or executable it runs.
    pub source: Option<String>,
    pub start_time: Option<String>,
    pub urls: Vec<AspireUrl>,
    pub commands: Vec<AspireCommand>,
    pub pid: Option<u32>,
    pub project_path: Option<IpcPath>,
    pub exit_code: Option<i32>,
    pub dashboard_url: Option<String>,
}

/// One console line of a resource (`aspire logs`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AspireLogLine {
    pub resource: String,
    /// As written, ANSI colours included.
    pub content: String,
    pub is_error: bool,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

/// The running AppHosts of an `aspire ps --format Json` snapshot (an array, or one object).
pub fn parse_apphosts(json: &str) -> Vec<AspireAppHost> {
    let Some(start) = json.find(['[', '{']) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<Value>(json[start..].trim()) else { return Vec::new() };
    let items = match v {
        Value::Array(a) => a,
        o @ Value::Object(_) => vec![o],
        _ => Vec::new(),
    };
    items
        .iter()
        .filter(|h| h.get("status").and_then(Value::as_str).is_none_or(|s| s.eq_ignore_ascii_case("running")))
        .filter_map(|h| {
            Some(AspireAppHost {
                path: PathBuf::from(text(h, "appHostPath")?).into(),
                pid: h.get("appHostPid").and_then(Value::as_u64).and_then(|p| u32::try_from(p).ok()),
                dashboard_url: text(h, "dashboardUrl"),
            })
        })
        .collect()
}

/// A resource object of `aspire describe --format Json` (one per line with `--follow`).
pub fn parse_resource(v: &Value) -> Option<AspireResource> {
    let name = text(v, "name")?;
    let props = v.get("properties");
    let prop = |k: &str| props.and_then(|p| p.get(k));
    let mut commands: Vec<AspireCommand> = v
        .get("commands")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(k, c)| AspireCommand {
            name: k.clone(),
            display_name: text(c, "displayName").unwrap_or_else(|| k.clone()),
            description: text(c, "description"),
        })
        .collect();
    commands.sort_by_key(|c| command_order(&c.name));
    Some(AspireResource {
        display_name: text(v, "displayName").unwrap_or_else(|| name.clone()),
        name,
        resource_type: text(v, "resourceType").unwrap_or_default(),
        state: text(v, "state"),
        health: text(v, "healthStatus"),
        source: text(v, "source"),
        start_time: text(v, "startTimestamp"),
        urls: v
            .get("urls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|u| Some(AspireUrl { name: text(u, "name").unwrap_or_default(), url: text(u, "url")? }))
            .collect(),
        commands,
        pid: prop("executable.pid").and_then(Value::as_u64).and_then(|p| u32::try_from(p).ok()),
        project_path: prop("project.path").and_then(Value::as_str).map(|p| PathBuf::from(p).into()),
        exit_code: v
            .get("exitCode")
            .or_else(|| prop("resource.exitCode"))
            .and_then(Value::as_i64)
            .and_then(|c| i32::try_from(c).ok()),
        dashboard_url: text(v, "dashboardUrl"),
    })
}

/// Lifecycle commands first, in the dashboard's order; custom ones after.
fn command_order(name: &str) -> (usize, String) {
    let known = ["start", "stop", "restart", "rebuild"];
    (known.iter().position(|k| *k == name).unwrap_or(known.len()), name.to_owned())
}

/// The resources of one `aspire describe` line: `{ "resources": [...] }` or a single resource.
pub fn parse_resources(line: &str) -> Vec<AspireResource> {
    let Some(start) = line.find('{') else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<Value>(line[start..].trim()) else { return Vec::new() };
    match v.get("resources").and_then(Value::as_array) {
        Some(all) => all.iter().filter_map(parse_resource).collect(),
        None => parse_resource(&v).into_iter().collect(),
    }
}

/// One `aspire logs --format Json` line.
pub fn parse_log(line: &str) -> Option<AspireLogLine> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    Some(AspireLogLine {
        resource: text(&v, "resourceName")?,
        content: v.get("content").and_then(Value::as_str).unwrap_or_default().to_owned(),
        is_error: v.get("isError").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// The running AppHost for `project`, if any (paths compared as written).
pub fn running<'a>(hosts: &'a [AspireAppHost], project: &Path) -> Option<&'a AspireAppHost> {
    hosts.iter().find(|h| AsRef::<Path>::as_ref(&h.path) == project)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESCRIBE: &str = r#"{"name":"api-wbqsvmeb","displayName":"api","resourceType":"Project","state":"Running","startTimestamp":"2026-11-04T11:46:25.462+00:00","source":"Fixture.Api.csproj","healthStatus":"Healthy","dashboardUrl":"http://localhost:15178/?resource=api-wbqsvmeb","urls":[{"name":"http","url":"http://localhost:5152"}],"properties":{"executable.pid":47244,"project.path":"/s/Fixture.Api/Fixture.Api.csproj"},"commands":{"stop":{"displayName":"Stop","description":"Stop resource"},"rebuild":{"displayName":"Rebuild"},"restart":{"displayName":"Restart","description":"Restart resource."}}}"#;

    #[test]
    fn reads_running_apphosts() {
        let one = r#"{"appHostPath":"/s/AppHost/AppHost.csproj","appHostPid":47193,"status":"running","dashboardUrl":"http://localhost:15178/login?t=8f83"}"#;
        let hosts = parse_apphosts(one);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].pid, Some(47193));
        assert_eq!(hosts[0].dashboard_url.as_deref(), Some("http://localhost:15178/login?t=8f83"));
        assert!(running(&hosts, Path::new("/s/AppHost/AppHost.csproj")).is_some());
        let many = format!("[{one}, {{\"appHostPath\":\"/t/A.csproj\",\"status\":\"stopping\"}}]");
        assert_eq!(parse_apphosts(&many).len(), 1);
        assert!(parse_apphosts("[]").is_empty());
        assert!(parse_apphosts("No running AppHosts found.").is_empty());
    }

    #[test]
    fn reads_a_resource() {
        let r = parse_resources(DESCRIBE).remove(0);
        assert_eq!((r.name.as_str(), r.display_name.as_str()), ("api-wbqsvmeb", "api"));
        assert_eq!(r.state.as_deref(), Some("Running"));
        assert_eq!(r.health.as_deref(), Some("Healthy"));
        assert_eq!(r.urls, vec![AspireUrl { name: "http".into(), url: "http://localhost:5152".into() }]);
        assert_eq!(r.pid, Some(47244));
        assert_eq!(r.project_path, Some(PathBuf::from("/s/Fixture.Api/Fixture.Api.csproj").into()));
        assert_eq!(r.commands.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["stop", "restart", "rebuild"]);
        let all = parse_resources(&format!("{{\"resources\":[{DESCRIBE}]}}"));
        assert_eq!(all.len(), 1);
        assert!(parse_resources("Connecting to AppHost...").is_empty());
    }

    #[test]
    fn reads_log_lines() {
        let l = parse_log(r#"{"resourceName":"api","content":"\u001B[32minfo\u001B[39m: started","isError":false}"#)
            .unwrap();
        assert_eq!(l.resource, "api");
        assert!(l.content.contains("info"));
        assert!(!l.is_error);
        assert!(parse_log("Getting logs...").is_none());
    }

    #[test]
    fn recognises_apphosts_by_sdk() {
        let info = |sdks: &[&str]| ProjectInfo {
            sdks: sdks.iter().map(|s| (*s).to_owned()).collect(),
            output_type: Some("Exe".into()),
            target_frameworks: vec!["net10.0".into()],
            is_test: false,
            project_references: Vec::new(),
        };
        assert!(is_apphost(&info(&["Aspire.AppHost.Sdk"])));
        assert!(is_apphost(&info(&["Microsoft.NET.Sdk", "Aspire.AppHost.Sdk"])));
        assert!(!is_apphost(&info(&["Microsoft.NET.Sdk.Web"])));
    }
}
