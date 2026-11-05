//! .NET diagnostics (DIAG): drives the dotnet diagnostics tools for the Diagnostics
//! tool window. Collected files (dumps, GC dumps, traces, counter recordings, stacks)
//! go to the project's diagnostics folder, so the window lists what was collected
//! and opens it again later.
//!
//! Live counters come from a `dotnet-monitor` sidecar on loopback (connect mode: it
//! sees every local .NET process), started on first use and stopped with the IDE
//! (`--exit-on-stdin-disconnect`).
use crate::Host;
use crate::rpc::Dispatcher;

use ide_core::{IdeError, IdeResult, IpcPath, RequestId};
use ide_dap::attach::{self, AttachKind, AttachableProcess};
use ide_dotnet::diagnostics::{
    self as diag, DiagTool, DiagToolStatus, DumpAnalysis, DumpType, FlameNode, FrameSource, HeapReport, MetricSample,
    ThreadStack, TraceProfile,
};
use ide_dotnet::trace_source::{self, MemberKind};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use super::Channel;

/// Flame graph nodes under this share of all samples are left out.
const MIN_FLAME_FRACTION: f64 = 0.002;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactKind {
    Dump,
    Gcdump,
    Trace,
    Counters,
    Stacks,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiagArtifact {
    pub path: IpcPath,
    pub kind: ArtifactKind,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified_secs: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HeapSnapshot {
    pub path: IpcPath,
    pub report: HeapReport,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StackSnapshot {
    /// Where the report was saved.
    pub path: IpcPath,
    pub threads: Vec<ThreadStack>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CounterRecording {
    pub path: IpcPath,
    pub samples: Vec<MetricSample>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MonitorStatus {
    /// `http://127.0.0.1:<port>` while the sidecar runs.
    pub url: Option<String>,
}

fn diagnostics_dir(host: &Host, window: &str) -> IdeResult<PathBuf> {
    let dir = host.project(window)?.cache_dir.join("diagnostics");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn stamp() -> String {
    // Sortable and unique enough: milliseconds since the epoch.
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    format!("{ms}")
}

fn artifact_path(dir: &Path, pid: u32, ext: &str) -> PathBuf {
    dir.join(format!("{pid}-{}.{ext}", stamp()))
}

/// Runs a diagnostics tool to completion (or until `token` is cancelled) and returns
/// what it printed; a non-zero exit is an error carrying the output.
async fn run_tool(exe: &Path, args: &[String], token: &ide_core::CancellationToken) -> IdeResult<String> {
    let mut child = tokio::process::Command::new(exe)
        .args(args)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", exe.display())))?;
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    // Both pipes at once, so a tool filling one cannot block on the other.
    let read = async move {
        let (mut a, mut b) = (String::new(), String::new());
        let read_out = async {
            if let Some(o) = out.as_mut() {
                let _ = o.read_to_string(&mut a).await;
            }
        };
        let read_err = async {
            if let Some(e) = err.as_mut() {
                let _ = e.read_to_string(&mut b).await;
            }
        };
        tokio::join!(read_out, read_err);
        a + &b
    };
    let (status, text) = tokio::select! {
        r = async { let t = read.await; (child.wait().await, t) } => r,
        () = token.cancelled() => {
            let _ = child.kill().await;
            return Err(IdeError::cancelled());
        }
    };
    let status = status.map_err(IdeError::from)?;
    if !status.success() {
        let tail: String =
            text.lines().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(IdeError::new(ide_core::error::ErrorKind::Io, format!("{} failed:\n{tail}", exe.display())));
    }
    Ok(text)
}

async fn tool(tool: DiagTool) -> IdeResult<PathBuf> {
    tokio::task::spawn_blocking(move || diag::require(tool)).await.map_err(|e| IdeError::internal(e.to_string()))?
}

fn s<T: ToString>(v: T) -> String {
    v.to_string()
}

/// The diagnostics tools and whether each is installed.
pub async fn diag_tools(host: &Host) -> IdeResult<Vec<DiagToolStatus>> {
    host.blocking(|| Ok(diag::statuses())).await
}

/// The running .NET processes (not this IDE's own tooling).
pub async fn diag_processes(host: &Host) -> IdeResult<Vec<AttachableProcess>> {
    host.blocking(|| {
        let me = std::process::id();
        Ok(attach::attachable_processes()?
            .into_iter()
            .filter(|p| p.kind == AttachKind::Dotnet && p.pid != me && !p.command_line.contains("dotnet-monitor"))
            .collect())
    })
    .await
}

/// Every managed thread's stack (`dotnet-stack report`), saved next to the other artifacts.
pub async fn diag_stacks(host: &Host, window: &str, request: RequestId, pid: u32) -> IdeResult<StackSnapshot> {
    let dir = diagnostics_dir(host, window)?;
    let exe = tool(DiagTool::Stack).await?;
    let token = host.begin_request(request);
    let result = run_tool(&exe, &[s("report"), s("-p"), s(pid)], &token).await;
    host.end_request(request);
    let text = result?;
    let path = artifact_path(&dir, pid, "stacks.txt");
    std::fs::write(&path, &text)?;
    Ok(StackSnapshot { threads: diag::parse_stacks(&text), path: path.into() })
}

/// Re-reads a saved stack report.
pub async fn diag_stacks_read(host: &Host, path: PathBuf) -> IdeResult<StackSnapshot> {
    host.blocking(move || {
        let text = std::fs::read_to_string(&path)?;
        Ok(StackSnapshot { threads: diag::parse_stacks(&text), path: path.into() })
    })
    .await
}

/// Snapshots the GC heap of `pid` (`dotnet-gcdump collect`) and reports it by type.
pub async fn diag_gcdump(host: &Host, window: &str, request: RequestId, pid: u32) -> IdeResult<HeapSnapshot> {
    let dir = diagnostics_dir(host, window)?;
    let exe = tool(DiagTool::Gcdump).await?;
    let path = artifact_path(&dir, pid, "gcdump");
    let token = host.begin_request(request);
    let result = async {
        run_tool(&exe, &[s("collect"), s("-p"), s(pid), s("-o"), path.to_string_lossy().into_owned()], &token).await?;
        gcdump_report(&exe, &path, &token).await
    }
    .await;
    host.end_request(request);
    result
}

async fn gcdump_report(exe: &Path, path: &Path, token: &ide_core::CancellationToken) -> IdeResult<HeapSnapshot> {
    let text = run_tool(exe, &[s("report"), path.to_string_lossy().into_owned()], token).await?;
    Ok(HeapSnapshot { report: diag::parse_gcdump_report(&text), path: path.to_path_buf().into() })
}

/// Reports a `.gcdump` file by type.
pub async fn diag_gcdump_read(host: &Host, request: RequestId, path: PathBuf) -> IdeResult<HeapSnapshot> {
    let exe = tool(DiagTool::Gcdump).await?;
    let token = host.begin_request(request);
    let result = gcdump_report(&exe, &path, &token).await;
    host.end_request(request);
    result
}

/// Writes a dump of `pid` (`dotnet-dump collect`).
pub async fn diag_dump(host: &Host, window: &str, request: RequestId, pid: u32, kind: DumpType) -> IdeResult<IpcPath> {
    let dir = diagnostics_dir(host, window)?;
    let exe = tool(DiagTool::Dump).await?;
    let path = artifact_path(&dir, pid, "dmp");
    let token = host.begin_request(request);
    let args =
        [s("collect"), s("-p"), s(pid), s("--type"), s(kind.arg()), s("-o"), path.to_string_lossy().into_owned()];
    let result = run_tool(&exe, &args, &token).await;
    host.end_request(request);
    result?;
    Ok(path.into())
}

/// Runs SOS `clrthreads` and `dumpheap -stat` on a dump.
pub async fn diag_dump_analyze(host: &Host, request: RequestId, path: PathBuf) -> IdeResult<DumpAnalysis> {
    let exe = tool(DiagTool::Dump).await?;
    let token = host.begin_request(request);
    let args = [
        s("analyze"),
        path.to_string_lossy().into_owned(),
        s("-c"),
        s("clrthreads"),
        s("-c"),
        s("dumpheap -stat"),
        s("-c"),
        s("exit"),
    ];
    let result = run_tool(&exe, &args, &token).await;
    host.end_request(request);
    let output = result?;
    let (threads, heap) = diag::split_analysis(&output);
    let size = std::fs::metadata(&path).map_or(0, |m| m.len());
    Ok(DumpAnalysis {
        threads: diag::parse_clrthreads(&threads),
        heap: diag::parse_dumpheap_stat(&heap),
        size,
        path: path.into(),
        output,
    })
}

/// Downloads the symbols and modules a dump needs (`dotnet-symbol`) into a folder next to it.
pub async fn diag_symbols(host: &Host, request: RequestId, dump: PathBuf) -> IdeResult<String> {
    let exe = tool(DiagTool::Symbol).await?;
    let out = dump.with_extension("symbols");
    let token = host.begin_request(request);
    let args = [
        s("--symbols"),
        s("--modules"),
        s("-o"),
        out.to_string_lossy().into_owned(),
        dump.to_string_lossy().into_owned(),
    ];
    let result = run_tool(&exe, &args, &token).await;
    host.end_request(request);
    result
}

/// Samples CPU stacks of `pid` for `seconds` (`dotnet-trace`) and merges them into a flame graph.
pub async fn diag_trace(
    host: &Host,
    window: &str,
    request: RequestId,
    pid: u32,
    seconds: u32,
) -> IdeResult<TraceProfile> {
    let dir = diagnostics_dir(host, window)?;
    let exe = tool(DiagTool::Trace).await?;
    let base = artifact_path(&dir, pid, "nettrace");
    let token = host.begin_request(request);
    let seconds = seconds.clamp(1, 3600);
    let duration = format!("00:{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60);
    let args = [
        s("collect"),
        s("-p"),
        s(pid),
        s("--duration"),
        duration,
        s("--format"),
        s("Speedscope"),
        s("-o"),
        base.to_string_lossy().into_owned(),
    ];
    let result = run_tool(&exe, &args, &token).await;
    host.end_request(request);
    result?;
    let speedscope = base.with_extension("speedscope.json");
    diag_trace_read(host, window, speedscope).await
}

/// Reads a Speedscope trace into a flame graph, with the source of the frames that
/// come from the project's C# code.
pub async fn diag_trace_read(host: &Host, window: &str, path: PathBuf) -> IdeResult<TraceProfile> {
    let project = host.project(window).ok();
    host.blocking(move || {
        let mut trace = diag::flame_graph(&std::fs::read_to_string(&path)?, &path, MIN_FLAME_FRACTION)?;
        if let Some(project) = project {
            trace.sources = resolve_trace_sources(&project, &trace.root);
        }
        Ok(trace)
    })
    .await
}

/// C# files above this size are not parsed for trace sources.
const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024;

/// Finds the declarations of the managed frames under `root` in the project's C#
/// files: the symbol index gives the files that declare a member of that name in
/// that type, and the file's outline the declaration with the frame's namespace,
/// nesting and parameter count.
fn resolve_trace_sources(project: &crate::project::Project, root: &FlameNode) -> Vec<FrameSource> {
    fn names<'a>(n: &'a FlameNode, out: &mut std::collections::BTreeSet<&'a str>) {
        for c in &n.children {
            out.insert(&c.name);
            names(c, out);
        }
    }
    let mut frames = std::collections::BTreeSet::new();
    names(root, &mut frames);
    let syntax = project.syntax.syntax().clone();
    let csharp = ide_core::LanguageId::new("csharp");
    let mut outlines: std::collections::HashMap<PathBuf, Vec<ide_core::OutlineItem>> = std::collections::HashMap::new();
    let mut out = Vec::new();
    for name in frames {
        let Some(frame) = trace_source::parse_frame(name) else { continue };
        let type_name = frame.type_name();
        let mut paths = std::collections::BTreeSet::new();
        for m in &frame.members {
            for s in project.index.symbols_named(&m.name, None, 256) {
                let container = s.container.as_deref();
                let declared_here = container == Some(type_name)
                    || (m.kind == MemberKind::Constructor && s.name == type_name)
                    || (m.kind == MemberKind::LocalFunction
                        && frame.members.iter().any(|o| Some(o.name.as_str()) == container));
                if declared_here && s.path.extension().is_some_and(|e| e == "cs") {
                    paths.insert(PathBuf::from(&*s.path));
                }
            }
        }
        for path in paths {
            let outline = outlines.entry(path.clone()).or_insert_with(|| {
                let small = std::fs::metadata(&path).is_ok_and(|m| m.len() <= MAX_SOURCE_BYTES);
                match std::fs::read_to_string(&path) {
                    Ok(text) if small => syntax.outline(&csharp, &text),
                    _ => Vec::new(),
                }
            });
            if let Some(found) = trace_source::find_in_outline(outline, &frame) {
                out.push(FrameSource {
                    frame: name.to_owned(),
                    path: path.into(),
                    line: found.selection.start.line,
                    col: found.selection.start.col,
                    start_line: found.range.start.line,
                    end_line: found.range.end.line,
                });
                break;
            }
        }
    }
    out
}

/// Records the runtime counters of `pid` for `seconds` (`dotnet-counters collect`).
pub async fn diag_counters(
    host: &Host,
    window: &str,
    request: RequestId,
    pid: u32,
    seconds: u32,
) -> IdeResult<CounterRecording> {
    let dir = diagnostics_dir(host, window)?;
    let exe = tool(DiagTool::Counters).await?;
    let path = artifact_path(&dir, pid, "counters.csv");
    let seconds = seconds.clamp(2, 3600);
    let duration = format!("00:{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60);
    let token = host.begin_request(request);
    let args = [
        s("collect"),
        s("-p"),
        s(pid),
        s("--refresh-interval"),
        s(1),
        s("--duration"),
        duration,
        s("--format"),
        s("csv"),
        s("-o"),
        path.to_string_lossy().into_owned(),
    ];
    let result = run_tool(&exe, &args, &token).await;
    host.end_request(request);
    result?;
    diag_counters_read(host, path).await
}

pub async fn diag_counters_read(host: &Host, path: PathBuf) -> IdeResult<CounterRecording> {
    host.blocking(move || {
        Ok(CounterRecording { samples: diag::parse_counters_csv(&std::fs::read_to_string(&path)?), path: path.into() })
    })
    .await
}

fn artifact_kind(path: &Path) -> Option<ArtifactKind> {
    let name = path.file_name()?.to_str()?;
    Some(if name.ends_with(".dmp") {
        ArtifactKind::Dump
    } else if name.ends_with(".gcdump") {
        ArtifactKind::Gcdump
    } else if name.ends_with(".speedscope.json") {
        ArtifactKind::Trace
    } else if name.ends_with(".counters.csv") {
        ArtifactKind::Counters
    } else if name.ends_with(".stacks.txt") {
        ArtifactKind::Stacks
    } else {
        return None;
    })
}

/// What this project collected, newest first.
pub async fn diag_artifacts(host: &Host, window: &str) -> IdeResult<Vec<DiagArtifact>> {
    let dir = diagnostics_dir(host, window)?;
    host.blocking(move || {
        let mut out: Vec<DiagArtifact> = std::fs::read_dir(&dir)?
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let kind = artifact_kind(&path)?;
                let meta = e.metadata().ok()?;
                let secs = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                Some(DiagArtifact {
                    path: path.into(),
                    kind,
                    size: meta.len(),
                    modified_secs: u32::try_from(secs).unwrap_or(u32::MAX),
                })
            })
            .collect();
        out.sort_by_key(|a| std::cmp::Reverse(a.modified_secs));
        Ok(out)
    })
    .await
}

/// Deletes a collected file (and a trace's `.nettrace`, a dump's symbols folder).
pub async fn diag_artifact_delete(host: &Host, window: &str, path: PathBuf) -> IdeResult<()> {
    let dir = diagnostics_dir(host, window)?;
    host.blocking(move || {
        if path.parent() != Some(dir.as_path()) || artifact_kind(&path).is_none() {
            return Err(IdeError::invalid("not a collected diagnostics file"));
        }
        std::fs::remove_file(&path)?;
        let name = path.to_string_lossy();
        if let Some(base) = name.strip_suffix(".speedscope.json") {
            let _ = std::fs::remove_file(format!("{base}.nettrace"));
        }
        if name.ends_with(".dmp") {
            let _ = std::fs::remove_dir_all(path.with_extension("symbols"));
        }
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------- dotnet-monitor sidecar

struct Sidecar {
    child: tokio::process::Child,
    /// Held open: the monitor exits when it closes (with the IDE).
    _stdin: Option<tokio::process::ChildStdin>,
    port: u16,
}

static MONITOR: LazyLock<tokio::sync::Mutex<Option<Sidecar>>> = LazyLock::new(|| tokio::sync::Mutex::new(None));

fn free_port() -> IdeResult<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Sends `GET path` to the monitor and returns the response body reader (HTTP/1.0, so
/// the body is not chunked and ends when the connection closes).
async fn monitor_get(port: u16, path: &str) -> IdeResult<BufReader<tokio::net::TcpStream>> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
    stream
        .write_all(format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nAccept: */*\r\n\r\n").as_bytes())
        .await?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).await?;
    let code: u16 = status.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }
    if !(200..300).contains(&code) {
        let mut body = String::new();
        let _ = reader.read_to_string(&mut body).await;
        return Err(IdeError::unavailable(format!("dotnet-monitor answered {code}: {}", body.trim())));
    }
    Ok(reader)
}

async fn monitor_port() -> IdeResult<u16> {
    let mut guard = MONITOR.lock().await;
    if let Some(m) = guard.as_mut()
        && matches!(m.child.try_wait(), Ok(None))
    {
        return Ok(m.port);
    }
    let exe = tool(DiagTool::Monitor).await?;
    let port = free_port()?;
    let config = std::env::temp_dir().join(format!("bayonet-monitor-{}.json", std::process::id()));
    std::fs::write(&config, r#"{ "GlobalCounter": { "IntervalSeconds": 1 } }"#)?;
    let mut child = tokio::process::Command::new(&exe)
        .args(["collect", "--no-auth", "--metrics", "false", "--exit-on-stdin-disconnect", "--urls"])
        .arg(format!("http://127.0.0.1:{port}"))
        .arg("--configuration-file-path")
        .arg(&config)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", exe.display())))?;
    let stdin = child.stdin.take();
    // Ready once it answers.
    let mut ready = false;
    for _ in 0..100 {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        if monitor_get(port, "/processes").await.is_ok() {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    if !ready {
        let _ = child.kill().await;
        return Err(IdeError::unavailable("dotnet-monitor did not start"));
    }
    *guard = Some(Sidecar { child, _stdin: stdin, port });
    Ok(port)
}

pub async fn diag_monitor_status(_host: &Host) -> IdeResult<MonitorStatus> {
    let mut guard = MONITOR.lock().await;
    let url = match guard.as_mut() {
        Some(m) => matches!(m.child.try_wait(), Ok(None)).then(|| format!("http://127.0.0.1:{}", m.port)),
        None => None,
    };
    Ok(MonitorStatus { url })
}

/// Starts the monitor sidecar if needed and returns its URL.
pub async fn diag_monitor_start(_host: &Host) -> IdeResult<String> {
    Ok(format!("http://127.0.0.1:{}", monitor_port().await?))
}

pub async fn diag_monitor_stop(_host: &Host) -> IdeResult<()> {
    if let Some(mut m) = MONITOR.lock().await.take() {
        let _ = m.child.kill().await;
    }
    Ok(())
}

/// Streams the live metrics of `pid` (from the monitor sidecar), a batch per second,
/// until `cancel_request(request)` or the process exits.
pub async fn diag_live_metrics(
    host: &Host,
    request: RequestId,
    pid: u32,
    on_samples: Channel<Vec<MetricSample>>,
) -> IdeResult<()> {
    let token = host.begin_request(request);
    let result = async {
        let port = monitor_port().await?;
        let reader = monitor_get(port, &format!("/livemetrics?pid={pid}&durationSeconds=-1")).await?;
        let mut lines = reader.lines();
        let mut batch: Vec<MetricSample> = Vec::new();
        let mut batch_time = 0.0;
        loop {
            let line = tokio::select! {
                l = lines.next_line() => l?,
                () = token.cancelled() => break,
            };
            let Some(line) = line else { break };
            let Some(sample) = diag::parse_live_metric(&line) else { continue };
            // One batch per collection interval: samples of one interval share a second.
            if !batch.is_empty() && (sample.time - batch_time).abs() > 500.0 {
                let _ = on_samples.send(std::mem::take(&mut batch));
            }
            if batch.is_empty() {
                batch_time = sample.time;
            }
            batch.push(sample);
        }
        if !batch.is_empty() {
            let _ = on_samples.send(batch);
        }
        Ok(())
    }
    .await;
    host.end_request(request);
    result
}

/// A `.http` file of the monitor's endpoints for `pid`, to try them in the HTTP client.
pub async fn diag_monitor_http(host: &Host, window: &str, pid: u32) -> IdeResult<IpcPath> {
    let dir = diagnostics_dir(host, window)?;
    let url = format!("http://127.0.0.1:{}", monitor_port().await?);
    let path = dir.join("dotnet-monitor.http");
    std::fs::write(&path, monitor_http_file(&url, pid))?;
    Ok(path.into())
}

fn monitor_http_file(url: &str, pid: u32) -> String {
    let requests: [(&str, &str); 11] = [
        ("Processes dotnet-monitor sees", "/processes"),
        ("Process details", "/process?pid={pid}"),
        ("Environment variables", "/env?pid={pid}"),
        ("Live metrics for 10 seconds", "/livemetrics?pid={pid}&durationSeconds=10"),
        ("Managed stacks", "/stacks?pid={pid}"),
        ("Logs for 10 seconds", "/logs?pid={pid}&durationSeconds=10"),
        ("CPU trace for 10 seconds", "/trace?pid={pid}&profile=Cpu&durationSeconds=10"),
        ("GC dump", "/gcdump?pid={pid}"),
        ("Process dump (with heap)", "/dump?pid={pid}&type=WithHeap"),
        ("Collection rules", "/collectionrules?pid={pid}"),
        ("Operations in progress", "/operations"),
    ];
    let mut out = format!(
        "# dotnet-monitor HTTP API, for process {pid}.\n# The IDE's monitor runs on {url} without authentication, on loopback only.\n\n@monitor = {url}\n@pid = {pid}\n"
    );
    for (title, path) in requests {
        let path = path.replace("{pid}", "{{pid}}");
        out.push_str(&format!("\n### {title}\nGET {{{{monitor}}}}{path}\n"));
    }
    out
}

pub(crate) fn register(d: &mut Dispatcher) {
    d.add("diag_tools", |c| async move { diag_tools(&c.host).await });
    d.add("diag_processes", |c| async move { diag_processes(&c.host).await });
    d.add("diag_stacks", |c| async move { diag_stacks(&c.host, &c.window, c.arg("request")?, c.arg("pid")?).await });
    d.add("diag_stacks_read", |c| async move { diag_stacks_read(&c.host, c.arg("path")?).await });
    d.add("diag_gcdump", |c| async move { diag_gcdump(&c.host, &c.window, c.arg("request")?, c.arg("pid")?).await });
    d.add("diag_gcdump_read", |c| async move { diag_gcdump_read(&c.host, c.arg("request")?, c.arg("path")?).await });
    d.add("diag_dump", |c| async move {
        diag_dump(&c.host, &c.window, c.arg("request")?, c.arg("pid")?, c.arg("kind")?).await
    });
    d.add("diag_dump_analyze", |c| async move { diag_dump_analyze(&c.host, c.arg("request")?, c.arg("path")?).await });
    d.add("diag_symbols", |c| async move { diag_symbols(&c.host, c.arg("request")?, c.arg("dump")?).await });
    d.add("diag_trace", |c| async move {
        diag_trace(&c.host, &c.window, c.arg("request")?, c.arg("pid")?, c.arg("seconds")?).await
    });
    d.add("diag_trace_read", |c| async move { diag_trace_read(&c.host, &c.window, c.arg("path")?).await });
    d.add("diag_counters", |c| async move {
        diag_counters(&c.host, &c.window, c.arg("request")?, c.arg("pid")?, c.arg("seconds")?).await
    });
    d.add("diag_counters_read", |c| async move { diag_counters_read(&c.host, c.arg("path")?).await });
    d.add("diag_artifacts", |c| async move { diag_artifacts(&c.host, &c.window).await });
    d.add("diag_artifact_delete", |c| async move { diag_artifact_delete(&c.host, &c.window, c.arg("path")?).await });
    d.add("diag_monitor_status", |c| async move { diag_monitor_status(&c.host).await });
    d.add("diag_monitor_start", |c| async move { diag_monitor_start(&c.host).await });
    d.add("diag_monitor_stop", |c| async move { diag_monitor_stop(&c.host).await });
    d.add("diag_monitor_http", |c| async move { diag_monitor_http(&c.host, &c.window, c.arg("pid")?).await });
    d.add("diag_live_metrics", |c| async move {
        diag_live_metrics(&c.host, c.arg("request")?, c.arg("pid")?, c.sender("onSamples")?).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_artifacts_by_extension() {
        assert_eq!(artifact_kind(Path::new("/d/1-2.dmp")), Some(ArtifactKind::Dump));
        assert_eq!(artifact_kind(Path::new("/d/1-2.speedscope.json")), Some(ArtifactKind::Trace));
        assert_eq!(artifact_kind(Path::new("/d/1-2.counters.csv")), Some(ArtifactKind::Counters));
        assert_eq!(artifact_kind(Path::new("/d/1-2.nettrace")), None);
        assert_eq!(artifact_kind(Path::new("/d/dotnet-monitor.http")), None);
    }

    /// Frame names as `dotnet-trace` writes them, against the tree-sitter outline of the code.
    #[test]
    fn finds_trace_frames_in_csharp_sources() {
        let src = r#"using System.Text;

namespace Hot.Work;

public static class Program
{
    public static async Task Main()
    {
        Func<int, int> f = x => Fib(x);
        Local(18);
        static int Local(int x) => Fib(x);
    }

    static int Fib(int n) => n < 2 ? n : Fib(n - 1) + Fib(n - 2);
}

public sealed class Calculator(int seed)
{
    public long Total { get { return seed; } }
    public void Spin() { }
    public void Spin(int times) { }
    public sealed class Nested
    {
        public Nested() { }
        public void Deep() { }
    }
}

public static class Worker
{
    public static async Task RunAsync()
    {
        await Task.Yield();
    }
}

public sealed class Generic<T>
{
    public string Build(T item) => item?.ToString() ?? "";
}
"#;
        let outline = ide_language::Syntax::new().outline(&ide_core::LanguageId::new("csharp"), src);
        let line = |frame: &str| {
            let f = trace_source::parse_frame(frame).unwrap_or_else(|| panic!("{frame}"));
            trace_source::find_in_outline(&outline, &f)
                .map(|f| (f.selection.start.line, f.range.start.line, f.range.end.line))
        };
        assert_eq!(line("Hot!Hot.Work.Program+<Main>d__0.MoveNext()"), Some((6, 6, 11)));
        assert_eq!(line("Hot!Hot.Work.Program.<Main>()"), Some((6, 6, 11)));
        assert_eq!(line("Hot!Hot.Work.Program+<>c.<Main>b__0_0(int32)"), Some((6, 6, 11)));
        assert_eq!(line("Hot!Hot.Work.Program.<Main>g__Local|0_1(int32)").map(|l| l.0), Some(10));
        assert_eq!(line("Hot!Hot.Work.Program.Fib(int32)").map(|l| l.0), Some(13));
        assert_eq!(line("Hot!Hot.Work.Calculator.get_Total()").map(|l| l.0), Some(18));
        assert_eq!(line("Hot!Hot.Work.Calculator.Spin()").map(|l| l.0), Some(19));
        assert_eq!(line("Hot!Hot.Work.Calculator.Spin(int32)").map(|l| l.0), Some(20));
        assert_eq!(line("Hot!Hot.Work.Calculator+Nested..ctor()").map(|l| l.0), Some(23));
        assert_eq!(line("Hot!Hot.Work.Calculator+Nested.Deep()").map(|l| l.0), Some(24));
        assert_eq!(line("Hot!Hot.Work.Calculator..ctor(int32)").map(|l| l.0), Some(16));
        assert_eq!(line("Hot!Hot.Work.Worker+<RunAsync>d__0.MoveNext()").map(|l| l.0), Some(30));
        assert_eq!(line("Hot!Hot.Work.Generic`1[System.__Canon].Build(!0)").map(|l| l.0), Some(38));
        assert_eq!(line("System.Private.CoreLib!System.Text.StringBuilder.Append(int32)"), None);
    }

    /// `TRACE=<x.speedscope.json> SRC=<a .cs file>`: the frames of a real trace that the file declares.
    #[test]
    #[ignore]
    fn resolves_a_real_trace() {
        let trace = PathBuf::from(std::env::var("TRACE").expect("TRACE"));
        let src = std::fs::read_to_string(std::env::var("SRC").expect("SRC")).unwrap();
        let profile = diag::flame_graph(&std::fs::read_to_string(&trace).unwrap(), &trace, MIN_FLAME_FRACTION).unwrap();
        let outline = ide_language::Syntax::new().outline(&ide_core::LanguageId::new("csharp"), &src);
        let mut names = std::collections::BTreeSet::new();
        fn walk<'a>(n: &'a FlameNode, out: &mut std::collections::BTreeSet<&'a str>) {
            for c in &n.children {
                out.insert(c.name.as_str());
                walk(c, out);
            }
        }
        walk(&profile.root, &mut names);
        let mut found = 0;
        for name in names {
            let Some(f) = trace_source::parse_frame(name) else { continue };
            let hit = trace_source::find_in_outline(&outline, &f);
            if hit.is_some() {
                found += 1;
            }
            eprintln!("{:>5} {name}", hit.map_or("-".to_owned(), |h| (h.selection.start.line + 1).to_string()));
        }
        assert!(found > 0);
    }

    #[test]
    fn writes_the_monitor_endpoints_as_an_http_file() {
        let f = monitor_http_file("http://127.0.0.1:5000", 42);
        assert!(f.contains("@pid = 42"));
        assert!(f.contains("### GC dump\nGET {{monitor}}/gcdump?pid={{pid}}\n"));
    }

    /// `DIAG_PID=<pid of a .NET app>`: streams its live metrics from a real monitor.
    #[tokio::test]
    #[ignore]
    async fn streams_live_metrics() {
        let pid: u32 = std::env::var("DIAG_PID").expect("DIAG_PID").parse().unwrap();
        let port = monitor_port().await.unwrap();
        let reader = monitor_get(port, &format!("/livemetrics?pid={pid}&durationSeconds=3")).await.unwrap();
        let mut lines = reader.lines();
        let mut samples = Vec::new();
        while let Some(l) = lines.next_line().await.unwrap() {
            samples.extend(diag::parse_live_metric(&l));
        }
        assert!(samples.iter().any(|s| s.name == "cpu-usage"), "{samples:?}");
        diag_monitor_stop_inner().await;
    }

    async fn diag_monitor_stop_inner() {
        if let Some(mut m) = MONITOR.lock().await.take() {
            let _ = m.child.kill().await;
        }
    }
}
