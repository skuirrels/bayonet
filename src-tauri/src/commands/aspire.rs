//! IPC commands of `ide_host::ops::aspire` (the shared implementation).

use crate::state::AppState;
use ide_core::{IdeResult, IpcPath, RequestId};
use ide_dotnet::aspire::AspireLogLine;
use ide_host::ops::aspire::{self as ops, AspireCli, AspireEvent};
use tauri::State;
use tauri::ipc::Channel;

/// Where the Aspire CLI is, and how to install it.
#[tauri::command]
#[specta::specta]
pub async fn aspire_cli(state: State<'_, AppState>) -> IdeResult<AspireCli> {
    ops::aspire_cli(&state.host).await
}

/// Which of `projects` are Aspire AppHosts.
#[tauri::command]
#[specta::specta]
pub async fn aspire_apphosts(state: State<'_, AppState>, projects: Vec<IpcPath>) -> IdeResult<Vec<IpcPath>> {
    ops::aspire_apphosts(&state.host, projects).await
}

/// Watches whether `apphost` runs and streams its resources, until `cancel_request(request)`.
#[tauri::command]
#[specta::specta]
pub async fn aspire_watch(
    state: State<'_, AppState>,
    request: RequestId,
    apphost: IpcPath,
    on_event: Channel<AspireEvent>,
) -> IdeResult<()> {
    ops::aspire_watch(&state.host, request, apphost, crate::commands::sender(on_event)).await
}

/// Streams the console of `resource`, until `cancel_request(request)`.
#[tauri::command]
#[specta::specta]
pub async fn aspire_logs(
    state: State<'_, AppState>,
    request: RequestId,
    apphost: IpcPath,
    resource: String,
    on_lines: Channel<Vec<AspireLogLine>>,
) -> IdeResult<()> {
    ops::aspire_logs(&state.host, request, apphost, resource, crate::commands::sender(on_lines)).await
}

/// Runs a resource command (`start`, `stop`, `restart`, `rebuild`, …).
#[tauri::command]
#[specta::specta]
pub async fn aspire_command(
    state: State<'_, AppState>,
    apphost: IpcPath,
    resource: String,
    name: String,
) -> IdeResult<String> {
    ops::aspire_command(&state.host, apphost, resource, name).await
}

/// Stops the running `apphost`.
#[tauri::command]
#[specta::specta]
pub async fn aspire_stop(state: State<'_, AppState>, apphost: IpcPath) -> IdeResult<()> {
    ops::aspire_stop(&state.host, apphost).await
}
