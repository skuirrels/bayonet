//! Remote .NET debugging over SSH (DBG-8) against the in-process SSH / SFTP server, with
//! password authentication: listing a host's .NET processes, installing netcoredbg there
//! and starting it on the configuration's connection. The server answers commands from a
//! closure, so the remote shell, netcoredbg itself and a real attach are not exercised;
//! `real_host_attach` below does that against a host you name.

use ide_core::error::ErrorKind;
use ide_core::remote_settings::{SshAuth, SshConfiguration};
use ide_dap::adapter::{AdapterLauncher, AdapterSpec, Transport};
use ide_host::ssh_debug::{self, MARK, SshAdapterLauncher};
use ide_ssh::{SshFiles, SshManager};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::AsyncReadExt;

const DEBUGGER: &str = "/home/dev/.openidea/netcoredbg/linux-x64/netcoredbg";
const DAP_FRAME: &str = "Content-Length: 2\r\n\r\n{}";

struct Remote {
    _tmp: tempfile::TempDir,
    ssh: Arc<SshManager>,
    /// The folder SFTP serves as `/`.
    root: PathBuf,
    /// Every command the server was asked to run.
    commands: Arc<Mutex<Vec<String>>>,
    installed: Arc<AtomicBool>,
    /// A local netcoredbg folder to install from.
    local: PathBuf,
}

/// A host answering as `uname` (`Linux x86_64`), reached by configuration `box` ("Staging").
async fn remote(uname: &'static str) -> Remote {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("server");
    std::fs::create_dir_all(&root).unwrap();
    let local = tmp.path().join("netcoredbg");
    std::fs::create_dir_all(local.join("lib")).unwrap();
    std::fs::write(local.join("netcoredbg"), "debugger").unwrap();
    std::fs::write(local.join("lib/ManagedPart.dll"), "managed").unwrap();

    let commands = Arc::new(Mutex::new(Vec::new()));
    let installed = Arc::new(AtomicBool::new(false));
    let (seen, there) = (commands.clone(), installed.clone());
    let port = ide_ssh::testing::sshd(
        root.clone(),
        Arc::new(move |cmd| {
            seen.lock().push(cmd.to_owned());
            if cmd.contains("dotnet-diagnostic") {
                let list = "4021\tdotnet /srv/shop/Shop.Api.dll --urls http://+:5000 \n4022\t/srv/worker/Worker \n";
                (format!("motd\n{MARK}\n{uname}\n{list}"), 0)
            } else if cmd.contains("ls -d") {
                let found = if there.load(Ordering::SeqCst) { DEBUGGER } else { "" };
                (format!("{MARK}\n{uname}\n/home/dev\n{found}\n"), 0)
            } else if cmd.starts_with("chmod 755") {
                there.store(true, Ordering::SeqCst);
                (String::new(), 0)
            } else if cmd.starts_with("exec ") {
                (DAP_FRAME.to_owned(), 0)
            } else {
                (String::new(), 1)
            }
        }),
    )
    .await;
    let ssh = Arc::new(SshManager::with_files(
        tmp.path().join("secrets.json"),
        SshFiles { known_hosts: tmp.path().join("known_hosts"), config: None },
    ));
    ssh.set_configs(vec![SshConfiguration {
        id: "box".into(),
        name: "Staging".into(),
        host: "127.0.0.1".into(),
        port: Some(port),
        user: "dev".into(),
        auth: SshAuth::Password,
        ..Default::default()
    }]);
    ssh.set_secret("box", Some("secret".into()), false);
    // The first connection reports the unknown host key, as the UI sees it.
    let unknown = ssh.session("box").await.err().expect("unknown host key");
    assert!(unknown.message.starts_with("Unknown SSH host key"), "{}", unknown.message);
    ssh.trust_host("box").await.unwrap();
    Remote { _tmp: tmp, ssh, root, commands, installed, local }
}

fn uploaded(root: &Path) -> Vec<PathBuf> {
    let dir = root.join("home/dev/.openidea/netcoredbg");
    std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_processes_installs_the_debugger_and_starts_it_over_ssh() {
    let r = remote("Linux x86_64").await;

    let list = ssh_debug::processes(&r.ssh, "box").await.unwrap();
    assert_eq!(
        list.iter().map(|p| (p.pid, p.name.as_str())).collect::<Vec<_>>(),
        [(4021, "Shop.Api"), (4022, "Worker")]
    );
    assert!(list.iter().all(|p| p.ssh.as_deref() == Some("box") && p.adapter == "netcoredbg"));

    // The first attach uploads the debugger (found by the configuration's name too).
    let local = r.local.clone();
    let pipe = ssh_debug::prepare(&r.ssh, "Staging", move |arch| {
        assert_eq!(arch, "x64");
        Ok(local)
    })
    .await
    .unwrap();
    let pipe: serde_json::Value = serde_json::from_str(&pipe).unwrap();
    assert_eq!(pipe, serde_json::json!({"sshConfiguration": "box", "debuggerPath": DEBUGGER}));
    // The fake server runs no `mv`: the upload is still where it was staged.
    let staged = uploaded(&r.root);
    assert_eq!(staged.len(), 1, "{staged:?}");
    let name = staged[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("linux-x64.") && name.ends_with(".part"), "{name}");
    assert_eq!(std::fs::read_to_string(staged[0].join("netcoredbg")).unwrap(), "debugger");
    assert_eq!(std::fs::read_to_string(staged[0].join("lib/ManagedPart.dll")).unwrap(), "managed");
    let install = r.commands.lock().iter().find(|c| c.starts_with("chmod 755")).cloned().unwrap();
    let tmp = format!("/home/dev/.openidea/netcoredbg/{name}");
    assert_eq!(
        install,
        format!(
            "chmod 755 {tmp}/netcoredbg && {tmp}/netcoredbg --version >/dev/null && rm -rf {dir} && mv {tmp} {dir}",
            dir = "/home/dev/.openidea/netcoredbg/linux-x64"
        )
    );

    // Installed: nothing is uploaded again.
    assert!(r.installed.load(Ordering::SeqCst));
    let again = ssh_debug::prepare(&r.ssh, "box", |_| panic!("already installed")).await.unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&again).unwrap(), pipe);
    assert_eq!(uploaded(&r.root).len(), 1);

    // The adapter's DAP stream is the remote command's stdin / stdout.
    let spec = AdapterSpec {
        adapter: "netcoredbg".into(),
        command: DEBUGGER.into(),
        args: vec!["--interpreter=vscode".into()],
        transport: Transport::Stdio,
        install_hint: None,
        cwd: std::env::temp_dir(),
        env: vec![("LOCAL_ONLY".into(), "1".into())],
        server_host: None,
        connect: None,
        pinned: None,
        ssh: Some("box".into()),
    };
    let launcher = SshAdapterLauncher::new(r.ssh.clone());
    let mut io = launcher.launch(spec.clone(), None).await.unwrap();
    assert!(io.child.is_none());
    let mut out = String::new();
    io.reader.read_to_string(&mut out).await.unwrap();
    assert_eq!(out, DAP_FRAME);
    assert_eq!(r.commands.lock().last().unwrap(), &format!("exec {DEBUGGER} --interpreter=vscode"));

    // Only stdio adapters, and only known configurations.
    let tcp = AdapterSpec { transport: Transport::Tcp, ..spec.clone() };
    assert_eq!(launcher.launch(tcp, None).await.err().unwrap().kind, ErrorKind::InvalidInput);
    let gone = AdapterSpec { ssh: Some("nowhere".into()), ..spec };
    assert_eq!(launcher.launch(gone, None).await.err().unwrap().kind, ErrorKind::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_that_is_not_linux_is_refused_before_anything_is_installed() {
    let r = remote("Darwin arm64").await;
    for error in [
        ssh_debug::processes(&r.ssh, "box").await.unwrap_err(),
        ssh_debug::prepare(&r.ssh, "box", |_| panic!("nothing to install")).await.unwrap_err(),
    ] {
        assert_eq!(error.kind, ErrorKind::Unavailable);
        assert!(
            error.message.contains("needs a Linux host") && error.message.ends_with("runs macOS"),
            "{}",
            error.message
        );
    }
    assert!(uploaded(&r.root).is_empty());
    assert_eq!(ssh_debug::processes(&r.ssh, "missing").await.unwrap_err().kind, ErrorKind::NotFound);
}

/// End to end against a real Linux host: lists its .NET processes, installs netcoredbg
/// (downloading the Linux build on first use) and attaches to one of them.
///
/// Needs a host of `~/.ssh/config` reachable with key or agent authentication whose key is
/// already in `known_hosts`, and a .NET app running there:
/// `OPENIDEA_SSH_DEBUG_HOST=<alias> cargo test -p ide-host --test ssh_debug -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a real Linux SSH host running a .NET app (OPENIDEA_SSH_DEBUG_HOST) and network access"]
async fn real_host_attach() {
    use ide_dap::{DebugManager, LaunchConfig, SessionOwner};

    let Ok(alias) = std::env::var("OPENIDEA_SSH_DEBUG_HOST") else {
        eprintln!("OPENIDEA_SSH_DEBUG_HOST is not set");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let ssh = Arc::new(SshManager::new(tmp.path().join("secrets.json")));
    ssh.set_configs(vec![SshConfiguration {
        id: "real".into(),
        name: alias.clone(),
        host: alias,
        auth: SshAuth::OpenSsh,
        ..Default::default()
    }]);
    let list = ssh_debug::processes(&ssh, "real").await.unwrap();
    eprintln!("processes: {list:?}");
    let process = list.first().expect("a .NET process on the host");

    let data = dirs::data_local_dir().unwrap().join("openidea-ssh-debug-test");
    let pipe = ssh_debug::prepare(&ssh, "real", move |arch| ide_host::ops::debug::linux_netcoredbg(&data, arch))
        .await
        .unwrap();
    eprintln!("pipeTransport: {pipe}");
    let pipe: serde_json::Value = serde_json::from_str(&pipe).unwrap();

    let languages = Arc::new(ide_language::LanguageRegistry::from_configs(Vec::new()));
    let manager = DebugManager::with_launcher(languages, Arc::new(SshAdapterLauncher::new(ssh.clone())));
    let sink: ide_core::SharedSink = Arc::new(|_| {});
    let config = LaunchConfig {
        name: process.name.clone(),
        adapter: "netcoredbg".into(),
        request: "attach".into(),
        program: None,
        args: vec![],
        cwd: None,
        env: Default::default(),
        extra_json: Some(serde_json::json!({"processId": process.pid, "pipeTransport": pipe}).to_string()),
    };
    let session = manager.start(config, SessionOwner::new("test", sink)).await.unwrap();
    let threads = manager.threads(session).await.unwrap();
    eprintln!("threads: {threads:?}");
    assert!(!threads.is_empty());
    manager.stop(session).await.unwrap();
}
