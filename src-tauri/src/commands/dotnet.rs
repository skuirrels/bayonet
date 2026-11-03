//! IPC commands of `ide_host::ops::dotnet` (the shared implementation).

use crate::state::AppState;
use ide_core::{IdeResult, IpcPath};
use ide_dotnet::Solution;
use tauri::{State, Window};

/// Solution files at the project root, `.slnx` before `.sln` before `.slnf`.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_solutions(window: Window, state: State<'_, AppState>) -> IdeResult<Vec<IpcPath>> {
    ide_host::ops::dotnet::dotnet_solutions(&state.host, window.label()).await
}

/// The solution at `path`: its folders, projects and solution items.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_solution(window: Window, state: State<'_, AppState>, path: IpcPath) -> IdeResult<Solution> {
    ide_host::ops::dotnet::dotnet_solution(&state.host, window.label(), path).await
}
