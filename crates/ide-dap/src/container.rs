//! Debugging .NET inside a container (DBG-8): netcoredbg runs in the container, started
//! through `docker exec -i`, and speaks DAP over that pipe (the C# extension's
//! `pipeTransport`). The IDE copies a Linux netcoredbg into the container first.

use crate::adapter::AdapterSpec;
use crate::attach::{AttachKind, AttachableProcess, dotnet_app, file_name};
use ide_core::{IdeError, IdeResult};
use serde_json::{Value, json};

/// Where netcoredbg is copied in a container.
pub const DEBUGGER_DIR: &str = "/tmp/openidea-netcoredbg";

/// Lists the container's .NET processes: those with a diagnostics socket in `/tmp`, as
/// `<pid>\t<command line>` lines. POSIX `sh` only (base images have no `ps`).
pub const LIST_SCRIPT: &str = r#"for s in /tmp/dotnet-diagnostic-*-socket; do [ -S "$s" ] || continue; p=${s#/tmp/dotnet-diagnostic-}; p=${p%%-*}; [ -r /proc/$p/cmdline ] || continue; printf '%s\t' "$p"; tr '\0' ' ' < /proc/$p/cmdline; echo; done"#;

/// The processes [`LIST_SCRIPT`] printed for `container` (its name).
pub fn parse_processes(container: &str, out: &str) -> Vec<AttachableProcess> {
    let mut seen = std::collections::HashSet::new();
    out.lines()
        .filter_map(|line| {
            let (pid, cmd) = line.split_once('\t')?;
            let pid: u32 = pid.trim().parse().ok()?;
            if !seen.insert(pid) {
                return None;
            }
            let cmd = cmd.trim().to_owned();
            let exe = cmd.split_whitespace().next().unwrap_or_default();
            let name = dotnet_app(&cmd).unwrap_or_else(|| file_name(exe).to_owned());
            Some(AttachableProcess {
                pid,
                name,
                command_line: cmd,
                kind: AttachKind::Dotnet,
                adapter: "netcoredbg".into(),
                host: None,
                port: None,
                container: Some(container.to_owned()),
            })
        })
        .collect()
}

/// netcoredbg's release name for the container's `uname -m`.
pub fn linux_arch(uname: &str) -> Option<&'static str> {
    match uname.trim() {
        "aarch64" | "arm64" => Some("arm64"),
        "x86_64" | "amd64" => Some("x64"),
        _ => None,
    }
}

/// The `pipeTransport` that runs the copied netcoredbg in `container` through `docker`.
pub fn pipe_transport(docker: &str, container: &str) -> Value {
    json!({
        "pipeProgram": docker,
        "pipeArgs": ["exec", "-i", container],
        "debuggerPath": format!("{DEBUGGER_DIR}/netcoredbg"),
    })
}

/// Starts the adapter through `pipeProgram pipeArgs… debuggerPath debuggerArgs…` instead
/// of locally (`docker exec -i <container>`, `ssh <host>`). The adapter's own arguments
/// (`--interpreter=vscode`) follow `debuggerPath` unless `debuggerArgs` replaces them.
pub(crate) fn apply_pipe_transport(spec: &mut AdapterSpec, pipe: &Value) -> IdeResult<()> {
    let strings = |key: &str| -> Option<Vec<String>> {
        pipe.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
    };
    let program = pipe
        .get("pipeProgram")
        .and_then(Value::as_str)
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| IdeError::invalid("pipeTransport: `pipeProgram` is required"))?;
    let debugger = pipe.get("debuggerPath").and_then(Value::as_str).unwrap_or(&spec.command).to_owned();
    let mut args = strings("pipeArgs").unwrap_or_default();
    args.push(debugger);
    args.extend(strings("debuggerArgs").unwrap_or_else(|| spec.args.clone()));
    spec.command = program.to_owned();
    spec.args = args;
    spec.pinned = None;
    if let Some(cwd) = pipe.get("pipeCwd").and_then(Value::as_str) {
        spec.cwd = cwd.into();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_dotnet_processes_by_their_app() {
        let out = "1\tdotnet /app/Api.dll --urls http://+:8080 \n57\t/app/Worker \n1\tdotnet /app/Api.dll\nbad line\n";
        let list = parse_processes("shop-api", out);
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].pid, list[0].name.as_str()), (1, "Api"));
        assert_eq!(list[0].command_line, "dotnet /app/Api.dll --urls http://+:8080");
        assert_eq!((list[1].pid, list[1].name.as_str()), (57, "Worker"));
        assert!(list.iter().all(|p| p.container.as_deref() == Some("shop-api") && p.adapter == "netcoredbg"));
    }

    #[test]
    fn maps_the_container_architecture() {
        assert_eq!(linux_arch("aarch64\n"), Some("arm64"));
        assert_eq!(linux_arch("x86_64"), Some("x64"));
        assert_eq!(linux_arch("riscv64"), None);
    }

    #[test]
    fn a_pipe_transport_wraps_the_adapter_command() {
        let config = crate::adapter::builtin("netcoredbg").unwrap();
        let mut spec = crate::adapter::resolve(&config, std::env::temp_dir());
        apply_pipe_transport(&mut spec, &pipe_transport("/usr/local/bin/docker", "shop-api")).unwrap();
        assert_eq!(spec.command, "/usr/local/bin/docker");
        assert_eq!(
            spec.args,
            ["exec", "-i", "shop-api", "/tmp/openidea-netcoredbg/netcoredbg", "--interpreter=vscode"]
        );

        let mut spec = crate::adapter::resolve(&config, std::env::temp_dir());
        let ssh = json!({"pipeProgram": "ssh", "pipeArgs": ["box"], "debuggerArgs": ["--interpreter=vscode", "--log"]});
        apply_pipe_transport(&mut spec, &ssh).unwrap();
        assert_eq!(spec.args, ["box", "netcoredbg", "--interpreter=vscode", "--log"]);
        assert!(apply_pipe_transport(&mut spec, &json!({})).is_err());
    }
}
