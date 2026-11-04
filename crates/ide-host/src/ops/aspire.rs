//! .NET Aspire (BLD-5): the Aspire tool window's view of a running AppHost,
//! through the Aspire CLI. Whether the AppHost runs comes from polling
//! `aspire ps` (fast, and it sees AppHosts however they were started);
//! while it runs, `aspire describe --follow` streams its resources. Console
//! logs stream from `aspire logs --follow`; resource commands run
//! `aspire resource <name> <command>`.
use crate::Host;
use crate::rpc::Dispatcher;

use ide_core::{IdeError, IdeResult, IpcPath, RequestId};
use ide_dotnet::aspire::{self, AspireAppHost, AspireLogLine, AspireResource};
use ide_dotnet::ef::ToolInstall;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::Channel;

/// How often `aspire ps` is asked whether the AppHost runs.
const POLL: Duration = Duration::from_secs(2);
/// Log lines are sent in batches at most this often.
const LOG_BATCH: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AspireCli {
    pub path: Option<IpcPath>,
    pub install: ToolInstall,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AspireEvent {
    /// The AppHost started (with its dashboard) or stopped (`None`).
    #[serde(rename_all = "camelCase")]
    AppHost { running: Option<AspireAppHost> },
    /// A resource was added or changed.
    #[serde(rename_all = "camelCase")]
    Resource { resource: Box<AspireResource> },
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

fn cli() -> IdeResult<PathBuf> {
    aspire::locate().ok_or_else(|| IdeError::unavailable("the Aspire CLI is not installed"))
}

fn command(cli: &Path, args: &[&str], cwd: Option<&Path>) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(cli);
    c.args(args)
        .args(["--nologo", "--non-interactive"])
        .env("ASPIRE_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    if let Some(cwd) = cwd {
        c.current_dir(cwd);
    }
    c
}

/// Where the Aspire CLI is, and how to install it.
pub async fn aspire_cli(_host: &Host) -> IdeResult<AspireCli> {
    Ok(AspireCli { path: aspire::locate().map(IpcPath::from), install: aspire::install_command() })
}

/// Which of `projects` are Aspire AppHosts.
pub async fn aspire_apphosts(host: &Host, projects: Vec<IpcPath>) -> IdeResult<Vec<IpcPath>> {
    host.blocking(move || {
        Ok(projects
            .into_iter()
            .filter(|p| ide_dotnet::read_project(p.as_ref()).is_ok_and(|i| aspire::is_apphost(&i)))
            .collect())
    })
    .await
}

async fn running_apphosts(cli: &Path) -> Vec<AspireAppHost> {
    match command(cli, &["ps", "--format", "Json"], None).stderr(Stdio::null()).output().await {
        Ok(out) => aspire::parse_apphosts(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => Vec::new(),
    }
}

/// Watches `apphost`: whether it runs (and its dashboard), then its resources
/// while it does, until `cancel_request(request)`.
pub async fn aspire_watch(
    host: &Host,
    request: RequestId,
    apphost: IpcPath,
    on_event: Channel<AspireEvent>,
) -> IdeResult<()> {
    let cli = cli()?;
    let token = host.begin_request(request);
    let project = PathBuf::from(apphost);
    let path = project.to_string_lossy().into_owned();
    let cwd = project.parent().map(Path::to_path_buf);
    let mut current: Option<AspireAppHost> = None;
    let mut describe: Option<(tokio::process::Child, tokio::io::Lines<BufReader<tokio::process::ChildStdout>>)> = None;
    let mut first = true;
    let mut tick = tokio::time::interval(POLL);
    loop {
        // Lines of the running `describe`, else the next poll (or cancellation).
        let line = async {
            match describe.as_mut() {
                Some((_, lines)) => lines.next_line().await.ok().flatten(),
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = token.cancelled() => break,
            line = line => match line {
                Some(line) => {
                    for resource in aspire::parse_resources(&line) {
                        let _ = on_event.send(AspireEvent::Resource { resource: Box::new(resource) });
                    }
                }
                // The stream ended (AppHost going away); the next poll says whether it still runs.
                None => describe = None,
            },
            _ = tick.tick() => {
                let hosts = running_apphosts(&cli).await;
                let now = aspire::running(&hosts, &project).cloned();
                if first || now != current {
                    first = false;
                    if now.is_none() {
                        describe = None;
                    }
                    current = now.clone();
                    let _ = on_event.send(AspireEvent::AppHost { running: now });
                }
                if current.is_some() && describe.is_none() {
                    let spawned = command(&cli, &["describe", "--apphost", &path, "--format", "Json", "--follow"], cwd.as_deref())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .spawn();
                    match spawned {
                        Ok(mut child) => {
                            if let Some(out) = child.stdout.take() {
                                describe = Some((child, BufReader::new(out).lines()));
                            }
                        }
                        Err(e) => {
                            let _ = on_event.send(AspireEvent::Error { message: format!("cannot run {}: {e}", cli.display()) });
                        }
                    }
                }
            }
        }
    }
    host.end_request(request);
    Ok(())
}

/// Streams the console of `resource` (its history first), until `cancel_request(request)`
/// or the AppHost stops.
pub async fn aspire_logs(
    host: &Host,
    request: RequestId,
    apphost: IpcPath,
    resource: String,
    on_lines: Channel<Vec<AspireLogLine>>,
) -> IdeResult<()> {
    let cli = cli()?;
    let token = host.begin_request(request);
    let project = PathBuf::from(apphost);
    let path = project.to_string_lossy().into_owned();
    let result = async {
        let mut child = command(
            &cli,
            // `--follow` replays the history first; adding `--tail` makes it replay it twice (CLI 13.4).
            &["logs", &resource, "--apphost", &path, "--format", "Json", "--follow"],
            project.parent(),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", cli.display())))?;
        let Some(out) = child.stdout.take() else { return Ok(()) };
        let mut lines = BufReader::new(out).lines();
        let mut batch: Vec<AspireLogLine> = Vec::new();
        let mut flush = tokio::time::interval(LOG_BATCH);
        loop {
            tokio::select! {
                () = token.cancelled() => break,
                line = lines.next_line() => match line? {
                    Some(line) => batch.extend(aspire::parse_log(&line)),
                    None => break,
                },
                _ = flush.tick() => {
                    if !batch.is_empty() {
                        let _ = on_lines.send(std::mem::take(&mut batch));
                    }
                }
            }
        }
        if !batch.is_empty() {
            let _ = on_lines.send(batch);
        }
        Ok(())
    }
    .await;
    host.end_request(request);
    result
}

/// Runs a resource command (`start`, `stop`, `restart`, `rebuild`, or a custom one).
pub async fn aspire_command(_host: &Host, apphost: IpcPath, resource: String, name: String) -> IdeResult<String> {
    let cli = cli()?;
    let project = PathBuf::from(apphost);
    let path = project.to_string_lossy().into_owned();
    let out = command(&cli, &["resource", &resource, &name, "--apphost", &path], project.parent())
        .output()
        .await
        .map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", cli.display())))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let text = text.trim().to_owned();
    if out.status.success() {
        Ok(text)
    } else {
        Err(IdeError::internal(if text.is_empty() {
            format!("`aspire resource {resource} {name}` failed")
        } else {
            text
        }))
    }
}

/// Stops the running `apphost` (however it was started).
pub async fn aspire_stop(_host: &Host, apphost: IpcPath) -> IdeResult<()> {
    let cli = cli()?;
    let project = PathBuf::from(apphost);
    let path = project.to_string_lossy().into_owned();
    let out = command(&cli, &["stop", "--apphost", &path], project.parent())
        .output()
        .await
        .map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", cli.display())))?;
    if out.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    Err(IdeError::internal(if text.is_empty() { "`aspire stop` failed".to_owned() } else { text }))
}

pub(crate) fn register(d: &mut Dispatcher) {
    d.add("aspire_stop", |c| async move { aspire_stop(&c.host, c.arg("apphost")?).await });
    d.add("aspire_cli", |c| async move { aspire_cli(&c.host).await });
    d.add("aspire_apphosts", |c| async move { aspire_apphosts(&c.host, c.arg("projects")?).await });
    d.add("aspire_watch", |c| async move {
        aspire_watch(&c.host, c.arg("request")?, c.arg("apphost")?, c.sender("onEvent")?).await
    });
    d.add("aspire_logs", |c| async move {
        aspire_logs(&c.host, c.arg("request")?, c.arg("apphost")?, c.arg("resource")?, c.sender("onLines")?).await
    });
    d.add("aspire_command", |c| async move {
        aspire_command(&c.host, c.arg("apphost")?, c.arg("resource")?, c.arg("name")?).await
    });
}
