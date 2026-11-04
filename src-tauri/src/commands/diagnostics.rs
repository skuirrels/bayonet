//! IPC commands of `ide_host::ops::diagnostics` (the shared implementation).

use crate::state::AppState;
use ide_core::{IdeResult, IpcPath, RequestId};
use ide_dap::attach::AttachableProcess;
use ide_dotnet::diagnostics::{DiagToolStatus, DumpAnalysis, DumpType, MetricSample, TraceProfile};
use ide_host::ops::diagnostics::{
    self as ops, CounterRecording, DiagArtifact, HeapSnapshot, MonitorStatus, StackSnapshot,
};
use std::path::PathBuf;
use tauri::ipc::Channel;
use tauri::{State, Window};

/// The diagnostics tools and whether each is installed.
#[tauri::command]
#[specta::specta]
pub async fn diag_tools(_window: Window, state: State<'_, AppState>) -> IdeResult<Vec<DiagToolStatus>> {
    ops::diag_tools(&state.host).await
}

/// The running .NET processes.
#[tauri::command]
#[specta::specta]
pub async fn diag_processes(_window: Window, state: State<'_, AppState>) -> IdeResult<Vec<AttachableProcess>> {
    ops::diag_processes(&state.host).await
}

/// Every managed thread's stack of `pid` (`dotnet-stack`).
#[tauri::command]
#[specta::specta]
pub async fn diag_stacks(
    window: Window,
    state: State<'_, AppState>,
    request: RequestId,
    pid: u32,
) -> IdeResult<StackSnapshot> {
    ops::diag_stacks(&state.host, window.label(), request, pid).await
}

/// Re-reads a saved stack report.
#[tauri::command]
#[specta::specta]
pub async fn diag_stacks_read(_window: Window, state: State<'_, AppState>, path: PathBuf) -> IdeResult<StackSnapshot> {
    ops::diag_stacks_read(&state.host, path).await
}

/// Snapshots the GC heap of `pid` (`dotnet-gcdump`).
#[tauri::command]
#[specta::specta]
pub async fn diag_gcdump(
    window: Window,
    state: State<'_, AppState>,
    request: RequestId,
    pid: u32,
) -> IdeResult<HeapSnapshot> {
    ops::diag_gcdump(&state.host, window.label(), request, pid).await
}

/// Reports a `.gcdump` file by type.
#[tauri::command]
#[specta::specta]
pub async fn diag_gcdump_read(
    _window: Window,
    state: State<'_, AppState>,
    request: RequestId,
    path: PathBuf,
) -> IdeResult<HeapSnapshot> {
    ops::diag_gcdump_read(&state.host, request, path).await
}

/// Writes a dump of `pid` (`dotnet-dump collect`).
#[tauri::command]
#[specta::specta]
pub async fn diag_dump(
    window: Window,
    state: State<'_, AppState>,
    request: RequestId,
    pid: u32,
    kind: DumpType,
) -> IdeResult<IpcPath> {
    ops::diag_dump(&state.host, window.label(), request, pid, kind).await
}

/// Runs SOS `clrthreads` and `dumpheap -stat` on a dump.
#[tauri::command]
#[specta::specta]
pub async fn diag_dump_analyze(
    _window: Window,
    state: State<'_, AppState>,
    request: RequestId,
    path: PathBuf,
) -> IdeResult<DumpAnalysis> {
    ops::diag_dump_analyze(&state.host, request, path).await
}

/// Downloads symbols and modules for a dump (`dotnet-symbol`).
#[tauri::command]
#[specta::specta]
pub async fn diag_symbols(
    _window: Window,
    state: State<'_, AppState>,
    request: RequestId,
    dump: PathBuf,
) -> IdeResult<String> {
    ops::diag_symbols(&state.host, request, dump).await
}

/// Samples CPU stacks of `pid` for `seconds` into a flame graph (`dotnet-trace`).
#[tauri::command]
#[specta::specta]
pub async fn diag_trace(
    window: Window,
    state: State<'_, AppState>,
    request: RequestId,
    pid: u32,
    seconds: u32,
) -> IdeResult<TraceProfile> {
    ops::diag_trace(&state.host, window.label(), request, pid, seconds).await
}

/// Reads a Speedscope trace into a flame graph.
#[tauri::command]
#[specta::specta]
pub async fn diag_trace_read(window: Window, state: State<'_, AppState>, path: PathBuf) -> IdeResult<TraceProfile> {
    ops::diag_trace_read(&state.host, window.label(), path).await
}

/// Records the counters of `pid` for `seconds` (`dotnet-counters`).
#[tauri::command]
#[specta::specta]
pub async fn diag_counters(
    window: Window,
    state: State<'_, AppState>,
    request: RequestId,
    pid: u32,
    seconds: u32,
) -> IdeResult<CounterRecording> {
    ops::diag_counters(&state.host, window.label(), request, pid, seconds).await
}

/// Reads a counter recording.
#[tauri::command]
#[specta::specta]
pub async fn diag_counters_read(
    _window: Window,
    state: State<'_, AppState>,
    path: PathBuf,
) -> IdeResult<CounterRecording> {
    ops::diag_counters_read(&state.host, path).await
}

/// The diagnostics files this project collected, newest first.
#[tauri::command]
#[specta::specta]
pub async fn diag_artifacts(window: Window, state: State<'_, AppState>) -> IdeResult<Vec<DiagArtifact>> {
    ops::diag_artifacts(&state.host, window.label()).await
}

/// Deletes a collected diagnostics file.
#[tauri::command]
#[specta::specta]
pub async fn diag_artifact_delete(window: Window, state: State<'_, AppState>, path: PathBuf) -> IdeResult<()> {
    ops::diag_artifact_delete(&state.host, window.label(), path).await
}

/// Whether the dotnet-monitor sidecar runs, and its URL.
#[tauri::command]
#[specta::specta]
pub async fn diag_monitor_status(_window: Window, state: State<'_, AppState>) -> IdeResult<MonitorStatus> {
    ops::diag_monitor_status(&state.host).await
}

/// Starts the dotnet-monitor sidecar and returns its URL.
#[tauri::command]
#[specta::specta]
pub async fn diag_monitor_start(_window: Window, state: State<'_, AppState>) -> IdeResult<String> {
    ops::diag_monitor_start(&state.host).await
}

/// Stops the dotnet-monitor sidecar.
#[tauri::command]
#[specta::specta]
pub async fn diag_monitor_stop(_window: Window, state: State<'_, AppState>) -> IdeResult<()> {
    ops::diag_monitor_stop(&state.host).await
}

/// Writes a `.http` file of dotnet-monitor's endpoints for `pid`.
#[tauri::command]
#[specta::specta]
pub async fn diag_monitor_http(window: Window, state: State<'_, AppState>, pid: u32) -> IdeResult<IpcPath> {
    ops::diag_monitor_http(&state.host, window.label(), pid).await
}

/// Streams the live metrics of `pid` until `cancel_request(request)`.
#[tauri::command]
#[specta::specta]
pub async fn diag_live_metrics(
    state: State<'_, AppState>,
    request: RequestId,
    pid: u32,
    on_samples: Channel<Vec<MetricSample>>,
) -> IdeResult<()> {
    ops::diag_live_metrics(&state.host, request, pid, crate::commands::sender(on_samples)).await
}
