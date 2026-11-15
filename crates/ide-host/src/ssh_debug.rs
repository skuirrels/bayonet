//! Debugging .NET on an SSH host (DBG-8): netcoredbg runs there and speaks DAP over an
//! exec channel of the SSH configuration's shared connection, so every way that
//! configuration authenticates (password, key pair, OpenSSH config and agent) works.
//! The IDE installs a Linux netcoredbg under the remote home first.

use ide_core::remote_settings::SshConfiguration;
use ide_core::{IdeError, IdeResult};
use ide_dap::adapter::{AdapterIo, AdapterLauncher, AdapterSpec, LaunchFuture, ProcessLauncher, Transport};
use ide_dap::attach::AttachableProcess;
use ide_dap::container;
use ide_ssh::shell::quote;
use ide_ssh::{Session, SshManager, sftp};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};

/// Printed before a script's own output: what a login shell prints first is skipped.
pub const MARK: &str = "__openidea_debug__";

/// `uname -sm`, the home directory, then every installed debugger.
const PROBE_SCRIPT: &str =
    r#"uname -sm; printf '%s\n' "$HOME"; ls -d "$HOME"/.openidea/netcoredbg/*/netcoredbg 2>/dev/null"#;

/// The SSH configuration `key` names: its id, else the one configuration with that name.
pub fn configuration(ssh: &SshManager, key: &str) -> IdeResult<SshConfiguration> {
    let key = key.trim();
    let configs = ssh.configs();
    if let Some(c) = configs.iter().find(|c| c.id == key) {
        return Ok(c.clone());
    }
    let mut named = configs.into_iter().filter(|c| !key.is_empty() && c.name.trim() == key);
    match (named.next(), named.next()) {
        (Some(c), None) => Ok(c),
        (Some(_), Some(_)) => Err(IdeError::invalid(format!("several SSH configurations are named {key}"))),
        _ => Err(IdeError::not_found(format!("no SSH configuration {key}"))),
    }
}

/// Runs `script` with the host's POSIX `sh`; what it printed after [`MARK`].
async fn run_script(session: &Session, script: &str) -> IdeResult<String> {
    let out = session.exec(&format!("sh -c {}", quote(&format!("echo {MARK}; {script}"))), None).await?;
    script_output(session.label(), &out.stdout_str())
}

fn script_output(label: &str, out: &str) -> IdeResult<String> {
    match out.split_once(MARK) {
        Some((_, rest)) => Ok(rest.trim_start_matches(['\r', '\n']).to_owned()),
        None => Err(not_linux(label, "has no POSIX shell")),
    }
}

fn not_linux(label: &str, why: &str) -> IdeError {
    IdeError::unavailable(format!("Remote .NET debugging needs a Linux host (x64 or arm64); {label} {why}"))
}

/// netcoredbg's release name for the architecture of a `uname -sm` line (`Linux x86_64`).
fn linux_host_arch(label: &str, uname: &str) -> IdeResult<&'static str> {
    let mut parts = uname.split_whitespace();
    let os = parts.next().unwrap_or_default();
    if os != "Linux" {
        let os = match os {
            "" => "an unknown system".to_owned(),
            "Darwin" => "macOS".to_owned(),
            other => other.to_owned(),
        };
        return Err(not_linux(label, &format!("runs {os}")));
    }
    let machine = parts.next().unwrap_or_default();
    container::linux_arch(machine)
        .ok_or_else(|| not_linux(label, &format!("is {machine}, which netcoredbg has no build for")))
}

/// What [`PROBE_SCRIPT`] printed.
#[derive(Debug, PartialEq, Eq)]
struct Probe {
    arch: &'static str,
    /// Absolute path of netcoredbg for this host.
    debugger: String,
    installed: bool,
}

fn parse_probe(label: &str, out: &str) -> IdeResult<Probe> {
    let mut lines = out.lines().map(str::trim).filter(|l| !l.is_empty());
    let arch = linux_host_arch(label, lines.next().unwrap_or_default())?;
    let home = lines
        .next()
        .filter(|h| h.starts_with('/'))
        .ok_or_else(|| IdeError::unavailable(format!("{label} did not report a home directory")))?;
    let debugger = sftp::join(&sftp::join(home, &container::ssh_debugger_dir(arch)), "netcoredbg");
    let installed = lines.any(|l| l == debugger);
    Ok(Probe { arch, debugger, installed })
}

/// The .NET processes on the host of SSH configuration `key` (those with a diagnostics
/// socket in `/tmp`).
pub async fn processes(ssh: &SshManager, key: &str) -> IdeResult<Vec<AttachableProcess>> {
    let config = configuration(ssh, key)?;
    let session = ssh.session(&config.id).await?;
    let out = run_script(&session, &format!("uname -sm; {}", container::LIST_SCRIPT)).await?;
    let (uname, list) = out.split_once('\n').unwrap_or((out.as_str(), ""));
    linux_host_arch(session.label(), uname)?;
    Ok(container::parse_ssh_processes(&config.id, list))
}

/// Makes sure netcoredbg is on the host of SSH configuration `key` (`local` gives the
/// folder of the Linux build for an architecture on this machine, installing it when
/// needed); the `pipeTransport` JSON of an attach request that runs it there.
pub async fn prepare<F>(ssh: &SshManager, key: &str, local: F) -> IdeResult<String>
where
    F: FnOnce(&'static str) -> IdeResult<PathBuf> + Send + 'static,
{
    let config = configuration(ssh, key)?;
    let session = ssh.session(&config.id).await?;
    let probe = parse_probe(session.label(), &run_script(&session, PROBE_SCRIPT).await?)?;
    if !probe.installed {
        let arch = probe.arch;
        let files = tokio::task::spawn_blocking(move || {
            let dir = local(arch)?;
            let mut files = Vec::new();
            collect_files(&dir, "", &mut files)?;
            Ok::<_, IdeError>(files)
        })
        .await
        .map_err(|e| IdeError::internal(e.to_string()))??;
        let dir = sftp::parent(&probe.debugger).unwrap_or("/").to_owned();
        install(&session, &files, &dir).await?;
    }
    Ok(container::ssh_pipe_transport(&config.id, &probe.debugger).to_string())
}

/// Every file under `dir` with its `/`-separated path relative to it.
fn collect_files(dir: &Path, prefix: &str, out: &mut Vec<(PathBuf, String)>) -> IdeResult<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_files(&entry.path(), &rel, out)?;
        } else if kind.is_file() {
            out.push((entry.path(), rel));
        }
    }
    Ok(())
}

/// Uploads `files` next to `dir`, checks that the debugger starts there, then puts the
/// folder in place (a broken or half-uploaded copy never becomes `dir`).
async fn install(session: &Session, files: &[(PathBuf, String)], dir: &str) -> IdeResult<()> {
    // Per install: another window may be installing on the same host right now.
    let tmp = format!(
        "{dir}.{}-{}.part",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
    );
    let started = std::time::Instant::now();
    let fs = ide_ssh::RemoteFs::new(session.sftp().await?);
    let mut result = Ok(());
    for (local, rel) in files {
        result = fs.upload(local, &sftp::join(&tmp, rel), false).await;
        if result.is_err() {
            break;
        }
    }
    fs.close().await;
    if result.is_ok() {
        let command = format!(
            "chmod 755 {tmp}/netcoredbg && {tmp}/netcoredbg --version >/dev/null && rm -rf {dir} && mv {tmp} {dir}",
            tmp = quote(&tmp),
            dir = quote(dir),
        );
        result = match session.exec(&command, None).await {
            Ok(out) => out.into_stdout("starting netcoredbg").map(drop),
            Err(e) => Err(e),
        };
    }
    match result {
        Ok(()) => {
            tracing::info!("installed netcoredbg on {} in {:?}", session.label(), started.elapsed());
            Ok(())
        }
        Err(e) => {
            let _ = session.exec(&format!("rm -rf {}", quote(&tmp)), None).await;
            Err(IdeError::new(e.kind, format!("Cannot install netcoredbg on {}: {}", session.label(), e.message)))
        }
    }
}

/// Starts adapters whose [`AdapterSpec::ssh`] names an SSH configuration on that
/// configuration's connection; every other adapter as a local process.
pub struct SshAdapterLauncher {
    ssh: Arc<SshManager>,
    local: ProcessLauncher,
}

impl SshAdapterLauncher {
    pub fn new(ssh: Arc<SshManager>) -> Self {
        Self { ssh, local: ProcessLauncher }
    }
}

impl AdapterLauncher for SshAdapterLauncher {
    fn launch(&self, spec: AdapterSpec, reuse_port: Option<u16>) -> LaunchFuture {
        let Some(key) = spec.ssh.clone() else { return self.local.launch(spec, reuse_port) };
        let ssh = self.ssh.clone();
        Box::pin(async move {
            if spec.transport != Transport::Stdio || reuse_port.is_some() {
                return Err(IdeError::invalid(format!(
                    "debug adapter `{}` cannot run over SSH: only adapters speaking DAP on their standard streams can",
                    spec.adapter
                )));
            }
            let config = configuration(&ssh, &key)?;
            let session = ssh.session(&config.id).await?;
            let command = ide_ssh::shell::command_line(None, &BTreeMap::new(), &spec.command, &spec.args);
            let stream = session.exec_stream(&command).await.map_err(|e| {
                IdeError::unavailable(format!("cannot start {} on {}: {e}", spec.command, session.label()))
            })?;
            let (reader, writer) = tokio::io::split(stream);
            Ok(AdapterIo {
                reader: Box::new(OnSession { reader, _session: session }),
                writer: Box::new(writer),
                child: None,
                port: None,
                stderr: Arc::default(),
                program_output: None,
            })
        })
    }
}

/// The adapter's output; keeps the connection it arrives on open while the debug session lasts.
struct OnSession<R> {
    reader: R,
    _session: Arc<Session>,
}

impl<R: AsyncRead + Unpin> AsyncRead for OnSession<R> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ide_core::error::ErrorKind;

    #[test]
    fn skips_what_the_login_shell_prints_before_the_script() {
        let out = format!("Welcome to box\n{MARK}\nLinux x86_64\n");
        assert_eq!(script_output("dev@box:22", &out).unwrap(), "Linux x86_64\n");
        // Windows' default shell has no `sh`.
        let e = script_output("dev@box:22", "'sh' is not recognized").unwrap_err();
        assert_eq!(e.kind, ErrorKind::Unavailable);
        assert!(e.message.contains("dev@box:22") && e.message.contains("Linux host"), "{}", e.message);
    }

    #[test]
    fn reads_the_host_and_whether_the_debugger_is_there() {
        let probe = parse_probe("dev@box:22", "Linux aarch64\n/home/dev\n").unwrap();
        assert_eq!(
            probe,
            Probe {
                arch: "arm64",
                debugger: "/home/dev/.openidea/netcoredbg/linux-arm64/netcoredbg".into(),
                installed: false
            }
        );
        // Another architecture's copy (a shared home) does not count.
        let other = "Linux x86_64\n/home/dev\n/home/dev/.openidea/netcoredbg/linux-arm64/netcoredbg\n";
        assert!(!parse_probe("box", other).unwrap().installed);
        let both = format!("{other}/home/dev/.openidea/netcoredbg/linux-x64/netcoredbg\n");
        let probe = parse_probe("box", &both).unwrap();
        assert!(probe.installed && probe.arch == "x64");
        assert!(parse_probe("box", "Linux x86_64\n").is_err());
    }

    #[test]
    fn only_linux_hosts_with_a_netcoredbg_build_qualify() {
        for (uname, why) in [
            ("Darwin arm64", "box runs macOS"),
            ("FreeBSD amd64", "box runs FreeBSD"),
            ("MINGW64_NT-10.0-19045 x86_64", "box runs MINGW64_NT-10.0-19045"),
            ("", "box runs an unknown system"),
            ("Linux riscv64", "box is riscv64, which netcoredbg has no build for"),
        ] {
            let e = linux_host_arch("box", uname).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Unavailable, "{uname}");
            assert!(e.message.ends_with(why), "{uname}: {}", e.message);
        }
        assert_eq!(linux_host_arch("box", "Linux x86_64").unwrap(), "x64");
        assert_eq!(linux_host_arch("box", "Linux aarch64").unwrap(), "arm64");
    }

    #[test]
    fn collects_a_debugger_folder_recursively() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("lib/net")).unwrap();
        std::fs::write(tmp.path().join("netcoredbg"), "bin").unwrap();
        std::fs::write(tmp.path().join("lib/net/ManagedPart.dll"), "dll").unwrap();
        let mut files = Vec::new();
        collect_files(tmp.path(), "", &mut files).unwrap();
        let mut rel: Vec<&str> = files.iter().map(|(_, r)| r.as_str()).collect();
        rel.sort_unstable();
        assert_eq!(rel, ["lib/net/ManagedPart.dll", "netcoredbg"]);
    }
}
