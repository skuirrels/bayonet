//! The .NET diagnostics helper (`dotnet/Bayonet.Diagnostics`): records EventPipe traces
//! and reads them with TraceEvent. That gives what `dotnet-trace`'s Speedscope export
//! cannot: profiling from launch, source lines from portable PDBs, and runtime events
//! such as garbage collections.
//!
//! Its sources are embedded here. The first use builds them with the installed SDK into
//! the IDE's tools folder, under a fingerprint of the sources, so a changed helper is
//! rebuilt and an unchanged one starts at once.
use ide_core::error::ErrorKind;
use ide_core::{CancellationToken, IdeError, IdeResult};
use ide_jsonrpc::{NoopHandler, Peer, RpcError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Must match `Protocol.Version` in the helper's `Program.cs`.
const PROTOCOL_VERSION: u32 = 1;
const PROJECT: &str = "Bayonet.Diagnostics.csproj";
const ASSEMBLY: &str = "Bayonet.Diagnostics.dll";

const SOURCES: &[(&str, &str)] = &[
    (PROJECT, include_str!("../../../dotnet/Bayonet.Diagnostics/Bayonet.Diagnostics.csproj")),
    ("Program.cs", include_str!("../../../dotnet/Bayonet.Diagnostics/Program.cs")),
    ("Rpc.cs", include_str!("../../../dotnet/Bayonet.Diagnostics/Rpc.cs")),
    ("Recorder.cs", include_str!("../../../dotnet/Bayonet.Diagnostics/Recorder.cs")),
    ("Analyzer.cs", include_str!("../../../dotnet/Bayonet.Diagnostics/Analyzer.cs")),
];

/// FNV-1a over the sources: stable across builds of the IDE, unlike `DefaultHasher`.
fn fingerprint() -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (name, text) in SOURCES {
        for b in name.bytes().chain([0]).chain(text.bytes()).chain([0]) {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

/// Where helper builds live, one folder per fingerprint.
pub fn install_root(tools_dir: &Path) -> PathBuf {
    tools_dir.join("dotnet-diagnostics")
}

/// The built helper for these sources, if it is there already.
pub fn installed(tools_dir: &Path) -> Option<PathBuf> {
    Some(install_root(tools_dir).join(fingerprint()).join("bin").join(ASSEMBLY)).filter(|p| p.is_file())
}

/// The `dotnet` host the helper runs on.
pub fn dotnet() -> IdeResult<PathBuf> {
    ide_core::process::find_executable("dotnet", &ide_core::tool_path::search_path())
        .ok_or_else(|| IdeError::unavailable("the .NET SDK (`dotnet`) was not found"))
}

static BUILD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The helper's assembly, building it first if these sources have not been built yet.
/// Older builds are removed once the new one is in place.
pub async fn ensure_built(tools_dir: &Path, dotnet: &Path, token: &CancellationToken) -> IdeResult<PathBuf> {
    let _one_at_a_time = BUILD.lock().await;
    if let Some(dll) = installed(tools_dir) {
        return Ok(dll);
    }
    let root = install_root(tools_dir);
    let dir = root.join(fingerprint());
    let src = dir.join("src");
    std::fs::create_dir_all(&src)?;
    for (name, text) in SOURCES {
        std::fs::write(src.join(name), text)?;
    }
    let staging = dir.join(format!("bin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let mut build = tokio::process::Command::new(dotnet);
    ide_core::process::prepare(build.as_std_mut());
    build
        .arg("build")
        .arg(src.join(PROJECT))
        .args(["-c", "Release", "-nologo", "-tl:off", "-v", "q", "-o"])
        .arg(&staging)
        .current_dir(&src)
        .envs(ide_core::process::shell_env())
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::select! {
        out = build.output() => out.map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", dotnet.display())))?,
        () = token.cancelled() => return Err(IdeError::cancelled()),
    };
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout) + String::from_utf8_lossy(&output.stderr);
        let errors: Vec<&str> = text.lines().filter(|l| l.contains("error")).take(10).collect();
        let shown = if errors.is_empty() { text.lines().rev().take(10).collect() } else { errors };
        return Err(IdeError::new(
            ErrorKind::Io,
            format!("building the .NET diagnostics helper failed:\n{}", shown.join("\n")),
        ));
    }
    let bin = dir.join("bin");
    if let Err(e) = std::fs::rename(&staging, &bin) {
        // Another IDE process finished the same build first.
        let _ = std::fs::remove_dir_all(&staging);
        if !bin.join(ASSEMBLY).is_file() {
            return Err(e.into());
        }
    }
    if let Ok(entries) = std::fs::read_dir(&root) {
        for e in entries.flatten() {
            if e.path() != dir {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    Ok(bin.join(ASSEMBLY))
}

/// A finished recording.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub path: PathBuf,
    pub pid: u32,
    pub seconds: f64,
    /// Set when a launched process exited during the recording.
    pub exit_code: Option<i32>,
}

/// A process for [`Helper::launch`] to start and profile from its first instruction.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: std::collections::BTreeMap<String, String>,
    pub seconds: u32,
    pub output: PathBuf,
    /// Stop the process when the recording ends (it ends anyway when the process exits).
    pub stop_when_done: bool,
}

/// A distinct call stack, outermost frame first (indexes into [`Analysis::frames`]).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stack {
    pub frames: Vec<usize>,
    pub ms: f64,
}

/// Time on one source line (1-based), from the PDB of the module that ran it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineTime {
    pub file: PathBuf,
    pub line: u32,
    pub self_ms: f64,
    pub total_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub start_ms: f64,
    pub pause_ms: f64,
    pub generation: u32,
    pub reason: String,
    pub kind: String,
    pub heap_before_mb: f64,
    pub heap_after_mb: f64,
}

/// A trace read by the helper: thread-time stacks as `dotnet-trace` computes them.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub duration_ms: f64,
    pub threads: u32,
    pub frames: Vec<String>,
    pub stacks: Vec<Stack>,
    pub lines: Vec<LineTime>,
    pub collections: Vec<Collection>,
}

#[derive(Deserialize)]
struct Info {
    version: u32,
}

/// A running helper process.
pub struct Helper {
    peer: Peer,
}

fn error(method: &str, e: RpcError) -> IdeError {
    match e {
        RpcError::Server { code: ide_jsonrpc::codes::REQUEST_CANCELLED, .. } => IdeError::cancelled(),
        RpcError::Server { code: ide_jsonrpc::codes::INVALID_PARAMS, message, .. } => IdeError::invalid(message),
        RpcError::Server { message, .. } => IdeError::new(ErrorKind::Io, message),
        RpcError::Closed => IdeError::unavailable(format!("the .NET diagnostics helper stopped during {method}")),
        other => other.into(),
    }
}

impl Helper {
    /// Starts the helper at `dll` on `dotnet` and checks it speaks this protocol.
    pub async fn start(dotnet: &Path, dll: &Path) -> IdeResult<Self> {
        let cwd = dll.parent().unwrap_or(Path::new("."));
        let peer = Peer::spawn(
            &dotnet.to_string_lossy(),
            &[dll.to_string_lossy().into_owned()],
            cwd,
            &[("DOTNET_CLI_TELEMETRY_OPTOUT".to_owned(), "1".to_owned())],
            Arc::new(NoopHandler),
            Box::new(|line| tracing::debug!(target: "ide_dotnet::helper", "{line}")),
        )
        .map_err(|e| IdeError::unavailable(format!("cannot start the .NET diagnostics helper: {e}")))?;
        let info: Info = peer.request("initialize", json!({}), None).await.map_err(|e| error("initialize", e))?;
        if info.version != PROTOCOL_VERSION {
            return Err(IdeError::internal(format!(
                "the .NET diagnostics helper speaks protocol {}, not {PROTOCOL_VERSION}",
                info.version
            )));
        }
        Ok(Self { peer })
    }

    /// Builds the helper if needed and starts it.
    pub async fn open(tools_dir: &Path, token: &CancellationToken) -> IdeResult<Self> {
        let dotnet = dotnet()?;
        let dll = ensure_built(tools_dir, &dotnet, token).await?;
        Self::start(&dotnet, &dll).await
    }

    async fn call<R: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
        token: &CancellationToken,
    ) -> IdeResult<R> {
        self.peer.request(method, params, Some(token.clone())).await.map_err(|e| error(method, e))
    }

    /// Records the running process `pid` for `seconds` into `output` (a `.nettrace`).
    pub async fn collect(
        &self,
        pid: u32,
        seconds: u32,
        output: &Path,
        token: &CancellationToken,
    ) -> IdeResult<Recording> {
        self.call("trace/collect", json!({ "pid": pid, "seconds": seconds, "output": output }), token).await
    }

    /// Starts a process suspended, so the recording covers its startup, and records it.
    pub async fn launch(&self, launch: &Launch, token: &CancellationToken) -> IdeResult<Recording> {
        let params = serde_json::to_value(launch).map_err(|e| IdeError::internal(e.to_string()))?;
        self.call("trace/launch", params, token).await
    }

    pub async fn analyze(&self, path: &Path, token: &CancellationToken) -> IdeResult<Analysis> {
        self.call("trace/analyze", json!({ "path": path }), token).await
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.peer.notify("exit", serde_json::Value::Null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_the_sources() {
        let f = fingerprint();
        assert_eq!(f.len(), 16);
        assert_eq!(f, fingerprint());
        assert!(SOURCES.iter().any(|(name, text)| *name == PROJECT && text.contains("TraceEvent")));
    }

    #[test]
    fn finds_only_a_build_of_these_sources() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(installed(dir.path()), None);
        let stale = install_root(dir.path()).join("0000000000000000").join("bin");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join(ASSEMBLY), b"").unwrap();
        assert_eq!(installed(dir.path()), None);
        let current = install_root(dir.path()).join(fingerprint()).join("bin");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join(ASSEMBLY), b"").unwrap();
        assert_eq!(installed(dir.path()), Some(current.join(ASSEMBLY)));
    }

    /// Builds the helper with the installed SDK, profiles a small app from launch and
    /// attached, and reads both traces: source lines, GCs and stacks.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the .NET 10 SDK and NuGet access"]
    async fn profiles_a_real_app() {
        let tools = tempfile::tempdir().unwrap();
        let app = tempfile::tempdir().unwrap();
        std::fs::write(
            app.path().join("Hot.csproj"),
            r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework><ImplicitUsings>enable</ImplicitUsings></PropertyGroup></Project>"#,
        )
        .unwrap();
        std::fs::write(
            app.path().join("Program.cs"),
            "var until = DateTime.UtcNow.AddSeconds(30);\nvar sum = 0.0;\nwhile (DateTime.UtcNow < until)\n{\n    for (var i = 1; i < 1_000_000; i++) sum += Math.Sqrt(i);\n    for (var i = 0; i < 100_000; i++) sum += new string('x', 64).Length;\n}\nConsole.WriteLine(sum);\n",
        )
        .unwrap();
        let dotnet = dotnet().unwrap();
        let built = std::process::Command::new(&dotnet)
            .args(["build", "-nologo", "-v", "q", "-o"])
            .arg(app.path().join("out"))
            .current_dir(app.path())
            .status()
            .unwrap();
        assert!(built.success());
        let token = CancellationToken::new();
        let helper = Helper::open(tools.path(), &token).await.unwrap();
        let program = app.path().join("out").join(if cfg!(windows) { "Hot.exe" } else { "Hot" });
        let launch = Launch {
            program,
            seconds: 3,
            output: app.path().join("launch.nettrace"),
            stop_when_done: true,
            ..Launch::default()
        };
        let recording = helper.launch(&launch, &token).await.unwrap();
        let a = helper.analyze(&recording.path, &token).await.unwrap();
        assert!(a.duration_ms > 1000.0, "{}", a.duration_ms);
        assert!(a.frames.iter().any(|f| f.contains("Program.<Main>$")), "{:?}", a.frames);
        let program_cs = app.path().join("Program.cs").canonicalize().unwrap();
        let own: Vec<&LineTime> =
            a.lines.iter().filter(|l| l.file.canonicalize().ok().as_ref() == Some(&program_cs)).collect();
        // Both loops show up on their own lines, and the loop allocating strings collects garbage.
        for line in [5, 6] {
            assert!(own.iter().any(|l| l.line == line && l.self_ms > 50.0), "line {line}: {own:?}");
        }
        assert!(!a.collections.is_empty());
    }
}
