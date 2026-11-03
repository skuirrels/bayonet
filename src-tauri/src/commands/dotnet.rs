//! IPC commands of `ide_host::ops::dotnet` (the shared implementation).

use crate::state::AppState;
use ide_core::RequestId;
use ide_core::{IdeResult, IpcPath};
use ide_dotnet::Solution;
use ide_dotnet::nuget::{InstalledPackages, PackageSearch};
use ide_dotnet::templates::{ProjectTemplate, TemplateValue};
use ide_dotnet::testing::{TestEvent, TestProject, TestSelection};
use tauri::ipc::Channel;
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

/// Searches the package sources of the project's `nuget.config`.
#[tauri::command]
#[specta::specta]
pub async fn nuget_search(
    window: Window,
    state: State<'_, AppState>,
    query: String,
    prerelease: bool,
    skip: u32,
    take: u32,
) -> IdeResult<PackageSearch> {
    ide_host::ops::dotnet::nuget_search(&state.host, window.label(), query, prerelease, skip, take).await
}

/// Every version of package `id`, newest first.
#[tauri::command]
#[specta::specta]
pub async fn nuget_versions(
    window: Window,
    state: State<'_, AppState>,
    id: String,
    prerelease: bool,
) -> IdeResult<Vec<String>> {
    ide_host::ops::dotnet::nuget_versions(&state.host, window.label(), id, prerelease).await
}

/// The packages of every project of the solution; `outdated` adds the latest versions.
#[tauri::command]
#[specta::specta]
pub async fn nuget_installed(
    window: Window,
    state: State<'_, AppState>,
    outdated: bool,
    prerelease: bool,
) -> IdeResult<InstalledPackages> {
    ide_host::ops::dotnet::nuget_installed(&state.host, window.label(), outdated, prerelease).await
}

/// Installs or updates package `id` in `projects`.
#[tauri::command]
#[specta::specta]
pub async fn nuget_install(
    window: Window,
    state: State<'_, AppState>,
    projects: Vec<IpcPath>,
    id: String,
    version: Option<String>,
) -> IdeResult<()> {
    ide_host::ops::dotnet::nuget_install(&state.host, window.label(), projects, id, version).await
}

/// Removes package `id` from `projects`.
#[tauri::command]
#[specta::specta]
pub async fn nuget_remove(
    window: Window,
    state: State<'_, AppState>,
    projects: Vec<IpcPath>,
    id: String,
) -> IdeResult<()> {
    ide_host::ops::dotnet::nuget_remove(&state.host, window.label(), projects, id).await
}

/// The test projects of the solution.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_test_projects(window: Window, state: State<'_, AppState>) -> IdeResult<Vec<TestProject>> {
    ide_host::ops::dotnet::dotnet_test_projects(&state.host, window.label()).await
}

/// Builds, then discovers or runs the tests of `selection` (every test project when
/// empty), streaming events on `on_event`. Cancel with `cancel_request(request)`.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_tests(
    window: Window,
    state: State<'_, AppState>,
    request: RequestId,
    run: bool,
    selection: Vec<TestSelection>,
    on_event: Channel<TestEvent>,
) -> IdeResult<()> {
    ide_host::ops::dotnet::dotnet_tests(
        &state.host,
        window.label(),
        request,
        run,
        selection,
        crate::commands::sender(on_event),
    )
    .await
}

/// Installed project and solution templates, custom ones included.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_templates(window: Window, state: State<'_, AppState>) -> IdeResult<Vec<ProjectTemplate>> {
    ide_host::ops::dotnet::dotnet_templates(&state.host, window.label()).await
}

/// Creates a project from a template, optionally adding it to the solution.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn dotnet_new_project(
    window: Window,
    state: State<'_, AppState>,
    template: String,
    name: String,
    directory: IpcPath,
    language: Option<String>,
    values: Vec<TemplateValue>,
    add_to_solution: bool,
    solution_folder: Option<String>,
) -> IdeResult<IpcPath> {
    ide_host::ops::dotnet::dotnet_new_project(
        &state.host,
        window.label(),
        template,
        name,
        directory,
        language,
        values,
        add_to_solution,
        solution_folder,
    )
    .await
}

/// Adds existing projects to the solution.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_solution_add(
    window: Window,
    state: State<'_, AppState>,
    projects: Vec<IpcPath>,
    solution_folder: Option<String>,
) -> IdeResult<()> {
    ide_host::ops::dotnet::dotnet_solution_add(&state.host, window.label(), projects, solution_folder).await
}

/// Removes projects from the solution.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_solution_remove(
    window: Window,
    state: State<'_, AppState>,
    projects: Vec<IpcPath>,
) -> IdeResult<()> {
    ide_host::ops::dotnet::dotnet_solution_remove(&state.host, window.label(), projects).await
}

/// The projects a project references.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_project_references(
    window: Window,
    state: State<'_, AppState>,
    project: IpcPath,
) -> IdeResult<Vec<IpcPath>> {
    ide_host::ops::dotnet::dotnet_project_references(&state.host, window.label(), project).await
}

/// Adds and removes project references.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_edit_references(
    window: Window,
    state: State<'_, AppState>,
    project: IpcPath,
    add: Vec<IpcPath>,
    remove: Vec<IpcPath>,
) -> IdeResult<()> {
    ide_host::ops::dotnet::dotnet_edit_references(&state.host, window.label(), project, add, remove).await
}
