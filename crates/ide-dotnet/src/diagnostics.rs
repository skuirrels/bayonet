//! .NET diagnostics (DIAG): the dotnet diagnostics tools the IDE drives, and the
//! parsers that turn their output into data for the Diagnostics tool window.
//!
//! - `dotnet-monitor` runs as a sidecar on loopback; its `/livemetrics` stream feeds
//!   the live counter charts (`dotnet-counters` only writes its file when it stops,
//!   so it records counters for a set time instead).
//! - `dotnet-stack report` gives every managed thread's stack.
//! - `dotnet-gcdump collect` / `report` gives the live objects by type.
//! - `dotnet-dump collect` writes a dump; `dotnet-dump analyze -c …` runs SOS commands
//!   on it (`clrthreads`, `dumpheap -stat`), and the interactive shells (`dotnet-dump
//!   analyze`, `dotnet-debug attach` / `open-dump`) run in a terminal.
//! - `dotnet-trace collect --format Speedscope` gives CPU samples, merged into a flame graph.
//! - `dotnet-symbol` downloads symbols for a dump, `dotnet-sos install` sets up LLDB.

use crate::ef::ToolInstall;
use ide_core::{IdeError, IdeResult, IpcPath};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DiagTool {
    Counters,
    Stack,
    Gcdump,
    Dump,
    Trace,
    Monitor,
    Symbol,
    Sos,
    Debug,
}

pub const ALL_TOOLS: [DiagTool; 9] = [
    DiagTool::Counters,
    DiagTool::Stack,
    DiagTool::Gcdump,
    DiagTool::Dump,
    DiagTool::Trace,
    DiagTool::Monitor,
    DiagTool::Symbol,
    DiagTool::Sos,
    DiagTool::Debug,
];

impl DiagTool {
    /// The executable and NuGet package name (they are the same).
    pub fn command(self) -> &'static str {
        match self {
            DiagTool::Counters => "dotnet-counters",
            DiagTool::Stack => "dotnet-stack",
            DiagTool::Gcdump => "dotnet-gcdump",
            DiagTool::Dump => "dotnet-dump",
            DiagTool::Trace => "dotnet-trace",
            DiagTool::Monitor => "dotnet-monitor",
            DiagTool::Symbol => "dotnet-symbol",
            DiagTool::Sos => "dotnet-sos",
            DiagTool::Debug => "dotnet-debug",
        }
    }

    /// What the tool is for, for its row in the tool list.
    pub fn purpose(self) -> &'static str {
        match self {
            DiagTool::Counters => "Records runtime and application metrics",
            DiagTool::Stack => "Prints the managed stacks of every thread",
            DiagTool::Gcdump => "Snapshots the GC heap: live objects by type",
            DiagTool::Dump => "Writes process dumps and analyses them with SOS",
            DiagTool::Trace => "Samples CPU stacks for a flame graph",
            DiagTool::Monitor => "Streams live metrics; collects artifacts over HTTP",
            DiagTool::Symbol => "Downloads symbols and modules for dumps",
            DiagTool::Sos => "Installs the SOS extension into LLDB",
            DiagTool::Debug => "Interactive SOS shell on a live process or dump",
        }
    }

    fn prerelease(self) -> bool {
        matches!(self, DiagTool::Debug)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiagToolStatus {
    pub tool: DiagTool,
    pub command: String,
    pub purpose: String,
    /// The executable, `None` when the tool is not installed.
    pub path: Option<IpcPath>,
    /// `dotnet tool install --global <tool>`.
    pub install: ToolInstall,
}

/// Where dotnet global tools live.
fn global_tools_dir() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(PathBuf::from(home).join(".dotnet").join("tools"))
}

/// The tool's executable: a global tool, else on the IDE's search path.
pub fn locate(tool: DiagTool) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{}.exe", tool.command()) } else { tool.command().to_owned() };
    global_tools_dir()
        .map(|d| d.join(&exe))
        .filter(|p| p.is_file())
        .or_else(|| ide_core::process::find_executable(tool.command(), &ide_core::tool_path::search_path()))
}

pub fn install_command(tool: DiagTool) -> ToolInstall {
    let mut args = vec!["tool".to_owned(), "install".to_owned(), "--global".to_owned(), tool.command().to_owned()];
    if tool.prerelease() {
        args.push("--prerelease".to_owned());
    }
    ToolInstall { command: "dotnet".to_owned(), args, cwd: None }
}

pub fn statuses() -> Vec<DiagToolStatus> {
    ALL_TOOLS
        .iter()
        .map(|&tool| DiagToolStatus {
            tool,
            command: tool.command().to_owned(),
            purpose: tool.purpose().to_owned(),
            path: locate(tool).map(IpcPath::from),
            install: install_command(tool),
        })
        .collect()
}

/// The tool's executable, or a `not found` error naming how to install it.
pub fn require(tool: DiagTool) -> IdeResult<PathBuf> {
    locate(tool).ok_or_else(|| {
        let install = install_command(tool);
        IdeError::not_found(format!(
            "{} is not installed. Install it with: {} {}",
            tool.command(),
            install.command,
            install.args.join(" ")
        ))
    })
}

// ---------------------------------------------------------------- dotnet-stack

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManagedFrame {
    /// `System.Private.CoreLib`; empty for `[Native Frames]`.
    pub module: String,
    /// `System.Threading.Monitor.Wait(class System.Object,int32)`.
    pub method: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStack {
    /// OS thread id as printed (`0xA43E53E`).
    pub id: String,
    /// Innermost frame first.
    pub frames: Vec<ManagedFrame>,
}

/// Parses `dotnet-stack report`.
pub fn parse_stacks(text: &str) -> Vec<ThreadStack> {
    let mut out: Vec<ThreadStack> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(id) = trimmed.strip_prefix("Thread (").and_then(|r| r.strip_suffix("):")) {
            out.push(ThreadStack { id: id.to_owned(), frames: Vec::new() });
        } else if !trimmed.is_empty()
            && let Some(thread) = out.last_mut()
        {
            let frame = match trimmed.split_once('!') {
                Some((module, method)) if !module.contains(' ') => {
                    ManagedFrame { module: module.to_owned(), method: method.to_owned() }
                }
                _ => ManagedFrame { module: String::new(), method: trimmed.to_owned() },
            };
            thread.frames.push(frame);
        }
    }
    out
}

// ---------------------------------------------------------------- heap statistics

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HeapType {
    /// `System.String`, `Entry<System.String,System.String>[]`.
    pub name: String,
    /// `System.Private.CoreLib.dll`; empty when unknown.
    pub module: String,
    pub bytes: u64,
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HeapReport {
    pub total_bytes: u64,
    pub total_objects: u64,
    /// Largest first.
    pub types: Vec<HeapType>,
}

fn number(s: &str) -> Option<u64> {
    let digits: String = s.chars().filter(|c| *c != ',' && *c != '.').collect();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn sorted(types: HashMap<(String, String), (u64, u64)>) -> Vec<HeapType> {
    let mut out: Vec<HeapType> =
        types.into_iter().map(|((name, module), (bytes, count))| HeapType { name, module, bytes, count }).collect();
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Parses `dotnet-gcdump report` (heapstat): rows of bytes, count, type `[module]`.
/// Size buckets (`(Bytes > 10K)`) of one type are added together.
pub fn parse_gcdump_report(text: &str) -> HeapReport {
    let mut total_bytes = 0;
    let mut total_objects = 0;
    let mut types: HashMap<(String, String), (u64, u64)> = HashMap::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(n) = t.strip_suffix("GC Heap bytes") {
            total_bytes = number(n.trim()).unwrap_or(0);
            continue;
        }
        if let Some(n) = t.strip_suffix("GC Heap objects") {
            total_objects = number(n.trim()).unwrap_or(0);
            continue;
        }
        let mut parts = t.splitn(3, char::is_whitespace).filter(|s| !s.is_empty());
        let (Some(bytes), Some(rest)) = (parts.next().and_then(number), t.split_once(char::is_whitespace)) else {
            continue;
        };
        let rest = rest.1.trim_start();
        let Some((count, name)) = rest.split_once(char::is_whitespace) else { continue };
        let Some(count) = number(count) else { continue };
        let name = name.trim();
        let (name, module) = match name.rsplit_once("  [") {
            Some((n, m)) => (n.trim(), m.trim_end_matches(']')),
            None => (name, ""),
        };
        // `System.String (Bytes > 10K)` → `System.String`.
        let name = match name.rfind(" (Bytes > ") {
            Some(i) if name.ends_with(')') => &name[..i],
            _ => name,
        };
        let e = types.entry((name.to_owned(), module.to_owned())).or_default();
        e.0 += bytes;
        e.1 += count;
    }
    HeapReport { total_bytes, total_objects, types: sorted(types) }
}

/// Parses SOS `dumpheap -stat`: `MT Count TotalSize Class Name` rows and the `Total` line.
pub fn parse_dumpheap_stat(text: &str) -> HeapReport {
    let mut types: HashMap<(String, String), (u64, u64)> = HashMap::new();
    let mut total_bytes = 0;
    let mut total_objects = 0;
    let mut in_table = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("MT ") && t.contains("Count") {
            in_table = true;
            continue;
        }
        if let Some(rest) = t.strip_prefix("Total ") {
            // `Total 20,918 objects, 1,673,268 bytes`
            let parts: Vec<u64> =
                rest.split(", ").map(|w| w.split_whitespace().next().and_then(number).unwrap_or(0)).collect();
            total_objects = parts.first().copied().unwrap_or(0);
            total_bytes = parts.get(1).copied().unwrap_or(0);
            in_table = false;
            continue;
        }
        if !in_table {
            continue;
        }
        let mut it = t.split_whitespace();
        let (Some(mt), Some(count), Some(size)) = (it.next(), it.next(), it.next()) else { continue };
        if !mt.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let (Some(count), Some(size)) = (number(count), number(size)) else { continue };
        let name = it.collect::<Vec<_>>().join(" ");
        if name.is_empty() {
            continue;
        }
        let e = types.entry((name, String::new())).or_default();
        e.0 += size;
        e.1 += count;
    }
    if total_bytes == 0 {
        total_bytes = types.values().map(|v| v.0).sum();
        total_objects = types.values().map(|v| v.1).sum();
    }
    HeapReport { total_bytes, total_objects, types: sorted(types) }
}

// ---------------------------------------------------------------- dumps

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManagedThread {
    /// The debugger's thread number (`XXXX` for a dead thread).
    pub debugger_id: String,
    pub managed_id: u32,
    pub os_id: String,
    pub gc_mode: String,
    pub lock_count: i32,
    /// `Finalizer`, `Threadpool Worker`, …
    pub role: Option<String>,
    /// The exception the thread is throwing, if any.
    pub exception: Option<String>,
    pub dead: bool,
}

/// Parses SOS `clrthreads`.
pub fn parse_clrthreads(text: &str) -> Vec<ManagedThread> {
    let mut out = Vec::new();
    let mut in_table = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("DBG") && t.contains("OSID") {
            in_table = true;
            continue;
        }
        if !in_table || t.is_empty() {
            continue;
        }
        let cols: Vec<&str> = t.split_whitespace().collect();
        // DBG ID OSID ThreadOBJ State GCMode AllocContext Domain LockCount Apt [Exception…]
        if cols.len() < 10 {
            in_table = false;
            continue;
        }
        let Ok(managed_id) = cols[1].parse::<u32>() else {
            in_table = false;
            continue;
        };
        let tail = cols[10..].join(" ");
        let (role, exception) = match tail.strip_prefix('(').and_then(|r| r.split_once(')')) {
            Some((role, rest)) => (Some(role.to_owned()), Some(rest.trim().to_owned()).filter(|s| !s.is_empty())),
            None => (None, Some(tail).filter(|s| !s.is_empty())),
        };
        out.push(ManagedThread {
            debugger_id: cols[0].to_owned(),
            managed_id,
            os_id: cols[2].to_owned(),
            gc_mode: cols[5].to_owned(),
            lock_count: cols[8].parse().unwrap_or(-1),
            role,
            exception,
            dead: cols[0] == "XXXX",
        });
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DumpType {
    /// Everything, including module images.
    Full,
    /// Memory with the GC heap (what managed analysis needs).
    Heap,
    /// Small: stacks and module list.
    Mini,
    /// Mini with personal data removed.
    Triage,
}

impl DumpType {
    pub fn arg(self) -> &'static str {
        match self {
            DumpType::Full => "Full",
            DumpType::Heap => "Heap",
            DumpType::Mini => "Mini",
            DumpType::Triage => "Triage",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DumpAnalysis {
    pub path: IpcPath,
    pub size: u64,
    pub threads: Vec<ManagedThread>,
    pub heap: HeapReport,
    /// SOS output as printed, for the console view.
    pub output: String,
}

/// Splits `dotnet-dump analyze -c a -c b` output into the parts of each command.
/// Each command's output starts after the prompt echo SOS prints, so it is split on
/// the known first lines of `clrthreads` (`ThreadCount:`) and `dumpheap -stat`
/// (`Statistics:`).
pub fn split_analysis(text: &str) -> (String, String) {
    let heap_at = text.find("Statistics:").unwrap_or(text.len());
    (text[..heap_at].to_owned(), text[heap_at..].to_owned())
}

// ---------------------------------------------------------------- traces

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FlameNode {
    pub name: String,
    /// Inclusive time in milliseconds (whole, for the bindings).
    pub total_ms: u32,
    pub self_ms: u32,
    pub children: Vec<FlameNode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TraceProfile {
    pub path: IpcPath,
    pub duration_ms: u32,
    pub threads: u32,
    /// All threads merged; children by time, longest first.
    pub root: FlameNode,
    /// Where the frames of the solution's own code are declared (filled in by the host).
    pub sources: Vec<FrameSource>,
}

/// The declaration a trace frame's method comes from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FrameSource {
    /// The frame name, as in [`FlameNode::name`].
    pub frame: String,
    pub path: IpcPath,
    /// 0-based line and column of the declaration's name.
    pub line: u32,
    pub col: u32,
    /// 0-based first and last line of the whole declaration.
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Deserialize)]
struct Speedscope {
    shared: SpeedscopeShared,
    profiles: Vec<SpeedscopeProfile>,
}

#[derive(Deserialize)]
struct SpeedscopeShared {
    frames: Vec<SpeedscopeFrame>,
}

#[derive(Deserialize)]
struct SpeedscopeFrame {
    name: String,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum SpeedscopeProfile {
    #[serde(rename = "evented")]
    Evented {
        #[serde(rename = "startValue")]
        start_value: f64,
        #[serde(rename = "endValue")]
        end_value: f64,
        events: Vec<SpeedscopeEvent>,
    },
    #[serde(rename = "sampled")]
    Sampled { samples: Vec<Vec<usize>>, weights: Vec<f64> },
}

#[derive(Deserialize)]
struct SpeedscopeEvent {
    #[serde(rename = "type")]
    kind: String,
    frame: usize,
    at: f64,
}

/// A call tree under construction: children by frame index.
#[derive(Default)]
struct Tree {
    total: f64,
    children: HashMap<usize, Tree>,
}

impl Tree {
    fn add(&mut self, stack: &[usize], weight: f64) {
        self.total += weight;
        if let Some((first, rest)) = stack.split_first() {
            self.children.entry(*first).or_default().add(rest, weight);
        }
    }

    fn into_node(self, name: String, frames: &[SpeedscopeFrame], min: f64) -> FlameNode {
        let child_total: f64 = self.children.values().map(|c| c.total).sum();
        let mut children: Vec<FlameNode> = self
            .children
            .into_iter()
            .filter(|(_, c)| c.total >= min)
            .map(|(f, c)| {
                let name = frames.get(f).map_or_else(|| format!("frame {f}"), |fr| fr.name.clone());
                c.into_node(name, frames, min)
            })
            .collect();
        children.sort_by(|a, b| b.total_ms.cmp(&a.total_ms).then_with(|| a.name.cmp(&b.name)));
        FlameNode {
            name,
            total_ms: self.total.round() as u32,
            self_ms: (self.total - child_total).max(0.0).round() as u32,
            children,
        }
    }
}

/// Frames dotnet-trace puts at the root of every stack (process, thread grouping).
/// The pseudo-frame dotnet-trace puts under the method that was running on a CPU.
const CPU_TIME_FRAME: &str = "CPU_TIME";

fn is_grouping_frame(name: &str) -> bool {
    name.starts_with("Process64 ")
        || name.starts_with("Process32 ")
        || name == "(Non-Activities)"
        || name == "Threads"
        || name.starts_with("Thread (")
}

/// Merges the per-thread profiles of a Speedscope file into one flame graph. Nodes
/// under `min_fraction` of the total are left out.
pub fn flame_graph(json: &str, path: &Path, min_fraction: f64) -> IdeResult<TraceProfile> {
    let s: Speedscope =
        serde_json::from_str(json).map_err(|e| IdeError::invalid(format!("not a Speedscope file: {e}")))?;
    let mut tree = Tree::default();
    let mut duration: f64 = 0.0;
    let threads = s.profiles.len() as u32;
    let name = |f: usize| s.shared.frames.get(f).map(|fr| fr.name.as_str());
    let strip = |stack: &[usize]| -> Vec<usize> {
        let mut frames: Vec<usize> =
            stack.iter().copied().skip_while(|f| name(*f).is_some_and(is_grouping_frame)).collect();
        // dotnet-trace marks on-CPU samples with a `CPU_TIME` leaf: that time is the caller's own.
        if frames.len() > 1 && frames.last().and_then(|f| name(*f)) == Some(CPU_TIME_FRAME) {
            frames.pop();
        }
        frames
    };
    for profile in &s.profiles {
        match profile {
            SpeedscopeProfile::Evented { start_value, end_value, events } => {
                duration = duration.max(end_value - start_value);
                let mut stack: Vec<usize> = Vec::new();
                let mut last = *start_value;
                for e in events {
                    let span = e.at - last;
                    if span > 0.0 && !stack.is_empty() {
                        let frames = strip(&stack);
                        if !frames.is_empty() {
                            tree.add(&frames, span);
                        }
                    }
                    last = e.at;
                    match e.kind.as_str() {
                        "O" => stack.push(e.frame),
                        "C" => {
                            if let Some(i) = stack.iter().rposition(|f| *f == e.frame) {
                                stack.truncate(i);
                            }
                        }
                        _ => {}
                    }
                }
            }
            SpeedscopeProfile::Sampled { samples, weights } => {
                for (sample, w) in samples.iter().zip(weights) {
                    let frames = strip(sample);
                    if !frames.is_empty() {
                        tree.add(&frames, *w);
                    }
                }
                duration = duration.max(weights.iter().sum());
            }
        }
    }
    let min = tree.total * min_fraction;
    let root = tree.into_node("All threads".to_owned(), &s.shared.frames, min);
    Ok(TraceProfile {
        path: path.to_path_buf().into(),
        duration_ms: duration.round() as u32,
        threads,
        root,
        sources: Vec::new(),
    })
}

// ---------------------------------------------------------------- metrics

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MetricSample {
    /// Milliseconds since the Unix epoch.
    pub time: f64,
    pub provider: String,
    /// `cpu-usage`, `dotnet.gc.collections`.
    pub name: String,
    pub display_name: String,
    pub unit: String,
    /// `Metric` (a gauge) or `Rate` (per interval).
    pub counter_type: String,
    /// `gc.heap.generation=gen0`; empty without tags.
    pub tags: String,
    pub value: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveMetric {
    timestamp: String,
    provider: String,
    name: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    counter_type: Option<String>,
    #[serde(default)]
    tags: Option<String>,
    value: f64,
}

/// Days from 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `2026-11-04T08:44:50.1984412+00:00` → ms since the epoch (UTC offsets applied).
pub fn parse_iso_time(s: &str) -> Option<f64> {
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-');
    let (y, mo, da) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    let (clock, offset_ms) = match rest.find(['+', 'Z']).or_else(|| rest.rfind('-')) {
        Some(i) => {
            let (c, o) = rest.split_at(i);
            let off = if o == "Z" {
                0.0
            } else {
                let sign = if o.starts_with('-') { -1.0 } else { 1.0 };
                let (h, m) = o[1..].split_once(':').unwrap_or((&o[1..], "0"));
                sign * (h.parse::<f64>().ok()? * 3_600_000.0 + m.parse::<f64>().ok()? * 60_000.0)
            };
            (c, off)
        }
        None => (rest, 0.0),
    };
    let mut c = clock.split(':');
    let (h, mi): (f64, f64) = (c.next()?.parse().ok()?, c.next()?.parse().ok()?);
    let sec: f64 = c.next()?.parse().ok()?;
    let days = days_from_civil(y, mo, da) as f64;
    Some(days * 86_400_000.0 + h * 3_600_000.0 + mi * 60_000.0 + sec * 1000.0 - offset_ms)
}

/// One line of dotnet-monitor's `/livemetrics` stream (JSON per line, possibly
/// prefixed by the record separator of `application/json-seq`).
pub fn parse_live_metric(line: &str) -> Option<MetricSample> {
    let line = line.trim_matches(|c: char| c == '\u{1e}' || c.is_whitespace());
    if line.is_empty() {
        return None;
    }
    let m: LiveMetric = serde_json::from_str(line).ok()?;
    Some(MetricSample {
        time: parse_iso_time(&m.timestamp)?,
        provider: m.provider,
        display_name: m.display_name.filter(|d| !d.is_empty()).unwrap_or_else(|| m.name.clone()),
        name: m.name,
        unit: m.unit.unwrap_or_default(),
        counter_type: m.counter_type.unwrap_or_else(|| "Metric".to_owned()),
        tags: m.tags.unwrap_or_default(),
        value: m.value,
    })
}

/// Parses a `dotnet-counters collect --format csv` recording:
/// `Timestamp,Provider,Counter Name,Counter Type,Mean/Increment` with names like
/// `dotnet.gc.collections ({collection} / 1 sec)[gc.heap.generation=gen0]`.
pub fn parse_counters_csv(text: &str) -> Vec<MetricSample> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        // The name may hold commas inside its tags; the other columns do not.
        let mut head = line.splitn(3, ',');
        let (Some(time), Some(provider), Some(rest)) = (head.next(), head.next(), head.next()) else { continue };
        let Some((rest, value)) = rest.rsplit_once(',') else { continue };
        let Some((counter, kind)) = rest.rsplit_once(',') else { continue };
        let Ok(value) = value.trim().parse::<f64>() else { continue };
        let Some(time) = parse_us_time(time) else { continue };
        let (counter, tags) = match counter.split_once('[') {
            Some((c, t)) if counter.ends_with(']') => (c, t.trim_end_matches(']')),
            _ => (counter, ""),
        };
        let (name, unit) = match counter.rsplit_once(" (") {
            Some((n, u)) if counter.ends_with(')') => (n.trim(), u.trim_end_matches(')')),
            _ => (counter.trim(), ""),
        };
        out.push(MetricSample {
            time,
            provider: provider.to_owned(),
            name: name.to_owned(),
            display_name: name.to_owned(),
            unit: unit.to_owned(),
            counter_type: kind.to_owned(),
            tags: tags.to_owned(),
            value,
        });
    }
    out
}

/// `11/04/2026 08:43:26` (local time as dotnet-counters writes it) → ms since the epoch, as if UTC.
fn parse_us_time(s: &str) -> Option<f64> {
    let (date, clock) = s.trim().split_once(' ')?;
    let mut d = date.split('/');
    let (mo, da, y): (u32, u32, i32) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    parse_iso_time(&format!("{y:04}-{mo:02}-{da:02}T{clock}Z"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stack_reports() {
        let text = "Thread (0xA43E53E):\n  [Native Frames]\n  System.Private.CoreLib!System.Threading.Monitor.Wait(class System.Object,int32)\n  Fixture.Api!Program.<Main>$(class System.String[])\n\nThread (0xA43E56F):\n  [Native Frames]\n";
        let threads = parse_stacks(text);
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, "0xA43E53E");
        assert_eq!(threads[0].frames[0], ManagedFrame { module: String::new(), method: "[Native Frames]".into() });
        assert_eq!(threads[0].frames[2].module, "Fixture.Api");
        assert_eq!(threads[0].frames[2].method, "Program.<Main>$(class System.String[])");
        assert_eq!(threads[1].frames.len(), 1);
    }

    #[test]
    fn parses_gcdump_reports_and_merges_size_buckets() {
        let text = "      1,085,491  GC Heap bytes\n         14,438  GC Heap objects\n\n   Object Bytes     Count  Type\n         62,158         1  System.String (Bytes > 10K)  [System.Private.CoreLib.dll]\n          2,054         2  System.String (Bytes > 1K)  [System.Private.CoreLib.dll]\n            912         1  Entry<System.Type,System.Text.Json.Serialization.JsonConverter>[]  [System.Private.CoreLib.dll]\n";
        let r = parse_gcdump_report(text);
        assert_eq!((r.total_bytes, r.total_objects), (1_085_491, 14_438));
        assert_eq!(
            r.types[0],
            HeapType {
                name: "System.String".into(),
                module: "System.Private.CoreLib.dll".into(),
                bytes: 64_212,
                count: 3
            }
        );
        assert_eq!(r.types[1].name, "Entry<System.Type,System.Text.Json.Serialization.JsonConverter>[]");
    }

    #[test]
    fn parses_sos_output() {
        let heap = "Statistics:\n          MT Count TotalSize Class Name\n00010ca54a50     1        24 Microsoft.Extensions.Primitives.StringSegment[]\n000109d889f0 4,643   348,828 System.String\nTotal 20,918 objects, 1,673,268 bytes\n";
        let r = parse_dumpheap_stat(heap);
        assert_eq!((r.total_objects, r.total_bytes), (20_918, 1_673_268));
        assert_eq!((r.types[0].name.as_str(), r.types[0].count, r.types[0].bytes), ("System.String", 4643, 348_828));
        let threads = " DBG   ID     OSID ThreadOBJ           State GC Mode     GC Alloc Context                  Domain           Count Apt Exception\n   0    1  a43e53e 000000010311AB30  2020020 Preemptive  0000000000000000:0000000000000000 000000010310CBF0 -00001 Ukn \n  20    2  a43e554 000000010311C350    21220 Preemptive  00000073948482D8:0000007394849BB8 000000010310CBF0 -00001 Ukn (Finalizer) \nXXXX   19        0 0000007C84E42D80  1031820 Preemptive  0000000000000000:0000000000000000 000000010310CBF0 -00001 Ukn (Threadpool Worker) System.InvalidOperationException 0000007394aa0010\n";
        let t = parse_clrthreads(threads);
        assert_eq!(t.len(), 3);
        assert_eq!((t[0].managed_id, t[0].role.as_deref(), t[0].lock_count), (1, None, -1));
        assert_eq!(t[1].role.as_deref(), Some("Finalizer"));
        assert!(t[2].dead);
        assert_eq!(t[2].exception.as_deref(), Some("System.InvalidOperationException 0000007394aa0010"));
    }

    #[test]
    fn merges_speedscope_threads_into_a_flame_graph() {
        let json = r#"{"shared":{"frames":[{"name":"Process64 dotnet (1)"},{"name":"Threads"},{"name":"Thread (7)"},{"name":"Main"},{"name":"Work"},{"name":"Idle"}]},
          "profiles":[
            {"type":"evented","startValue":0,"endValue":10,"events":[
              {"type":"O","frame":0,"at":0},{"type":"O","frame":1,"at":0},{"type":"O","frame":2,"at":0},{"type":"O","frame":3,"at":0},
              {"type":"O","frame":4,"at":2},{"type":"C","frame":4,"at":8},{"type":"C","frame":3,"at":10}]},
            {"type":"evented","startValue":0,"endValue":4,"events":[{"type":"O","frame":5,"at":0},{"type":"C","frame":5,"at":4}]}
          ]}"#;
        let p = flame_graph(json, Path::new("/t.json"), 0.0).unwrap();
        assert_eq!((p.threads, p.duration_ms, p.root.total_ms), (2, 10, 14));
        let main = &p.root.children[0];
        assert_eq!((main.name.as_str(), main.total_ms, main.self_ms), ("Main", 10, 4));
        assert_eq!(main.children[0].name, "Work");
        assert_eq!(p.root.children[1].name, "Idle");
        // Nodes under the threshold go.
        assert_eq!(flame_graph(json, Path::new("/t.json"), 0.5).unwrap().root.children.len(), 1);
    }

    #[test]
    fn counts_cpu_time_as_the_callers_own_time() {
        let json = r#"{"shared":{"frames":[{"name":"Main"},{"name":"Spin"},{"name":"CPU_TIME"},{"name":"UNMANAGED_CODE_TIME"}]},
          "profiles":[{"type":"evented","startValue":0,"endValue":10,"events":[
            {"type":"O","frame":0,"at":0},{"type":"O","frame":1,"at":0},{"type":"O","frame":2,"at":0},{"type":"C","frame":2,"at":6},
            {"type":"O","frame":3,"at":6},{"type":"C","frame":3,"at":8},{"type":"C","frame":1,"at":8},
            {"type":"O","frame":2,"at":8},{"type":"C","frame":2,"at":10},{"type":"C","frame":0,"at":10}]}]}"#;
        let p = flame_graph(json, Path::new("/t.json"), 0.0).unwrap();
        let main = &p.root.children[0];
        assert_eq!((main.name.as_str(), main.total_ms, main.self_ms), ("Main", 10, 2));
        let spin = &main.children[0];
        assert_eq!((spin.name.as_str(), spin.total_ms, spin.self_ms), ("Spin", 8, 6));
        // Native time stays a frame of its own.
        assert_eq!(spin.children.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["UNMANAGED_CODE_TIME"]);
    }

    #[test]
    fn parses_live_metrics_and_counter_recordings() {
        let line = "\u{1e}{\"timestamp\":\"2026-11-04T08:44:50.5+01:00\",\"provider\":\"System.Runtime\",\"name\":\"cpu-usage\",\"displayName\":\"CPU Usage\",\"unit\":\"%\",\"counterType\":\"Metric\",\"tags\":\"\",\"value\":0.5}";
        let m = parse_live_metric(line).unwrap();
        assert_eq!(
            (m.name.as_str(), m.display_name.as_str(), m.unit.as_str(), m.value),
            ("cpu-usage", "CPU Usage", "%", 0.5)
        );
        assert_eq!(m.time, parse_iso_time("2026-11-04T07:44:50.5Z").unwrap());
        assert_eq!(parse_iso_time("1970-01-02T00:00:01Z"), Some(86_401_000.0));
        assert!(parse_live_metric("").is_none());
        let csv = "Timestamp,Provider,Counter Name,Counter Type,Mean/Increment\n11/04/2026 08:43:27,System.Runtime,dotnet.gc.collections ({collection} / 1 sec)[gc.heap.generation=gen0],Rate,2\n11/04/2026 08:43:27,System.Runtime,dotnet.process.memory.working_set (By),Metric,68665344\n";
        let s = parse_counters_csv(csv);
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].name.as_str(), s[0].unit.as_str(), s[0].tags.as_str()),
            ("dotnet.gc.collections", "{collection} / 1 sec", "gc.heap.generation=gen0")
        );
        assert_eq!((s[1].counter_type.as_str(), s[1].value), ("Metric", 68_665_344.0));
        assert_eq!(s[1].time, parse_iso_time("2026-11-04T08:43:27Z").unwrap());
    }

    /// `DIAG=<dir>` with `stack.txt`, `gcreport.txt`, `analyze.txt`, `counters.csv`,
    /// `api.speedscope.json` captured from a real process.
    #[test]
    #[ignore]
    fn parses_real_tool_output() {
        let dir = PathBuf::from(std::env::var("DIAG").expect("DIAG"));
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
        let stacks = parse_stacks(&read("stack.txt"));
        assert!(stacks.len() > 3 && stacks.iter().all(|t| !t.frames.is_empty()));
        let gc = parse_gcdump_report(&read("gcreport.txt"));
        assert!(gc.total_bytes > 0 && gc.types.len() > 100);
        let (threads, heap) = split_analysis(&read("analyze.txt"));
        let threads = parse_clrthreads(&threads);
        let heap = parse_dumpheap_stat(&heap);
        assert!(!threads.is_empty() && heap.total_objects > 0 && heap.types.len() > 100);
        let counters = parse_counters_csv(&read("counters.csv"));
        assert!(!counters.is_empty());
        let trace = flame_graph(&read("api.speedscope.json"), &dir, 0.001).unwrap();
        assert!(trace.threads > 0 && !trace.root.children.is_empty());
        eprintln!(
            "stacks {} gc {} types {} bytes; threads {} heap {} types; counters {}; trace {} ms, top {:?}",
            stacks.len(),
            gc.types.len(),
            gc.total_bytes,
            threads.len(),
            heap.types.len(),
            counters.len(),
            trace.duration_ms,
            trace.root.children.iter().take(3).map(|c| (&c.name, c.total_ms)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn installs_missing_tools_globally() {
        assert_eq!(install_command(DiagTool::Gcdump).args, ["tool", "install", "--global", "dotnet-gcdump"]);
        assert_eq!(install_command(DiagTool::Debug).args.last().map(String::as_str), Some("--prerelease"));
        assert_eq!(statuses().len(), 9);
    }
}
