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
    coverage: bool,
    selection: Vec<TestSelection>,
    on_event: Channel<TestEvent>,
) -> IdeResult<()> {
    ide_host::ops::dotnet::dotnet_tests(
        &state.host,
        window.label(),
        request,
        run,
        coverage,
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

/// Converts a `.sln` to `.slnx` and returns the new file.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_migrate_solution(
    window: Window,
    state: State<'_, AppState>,
    solution: IpcPath,
) -> IdeResult<IpcPath> {
    ide_host::ops::dotnet::dotnet_migrate_solution(&state.host, window.label(), solution).await
}

/// Where the .NET SDK in use defines MSBuild target `name`.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_sdk_target(
    window: Window,
    state: State<'_, AppState>,
    name: String,
) -> IdeResult<Option<ide_dotnet::msbuild::FileLocation>> {
    ide_host::ops::dotnet::dotnet_sdk_target(&state.host, window.label(), name).await
}

/// Reads an MSBuild binary log for the binary log viewer.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_binlog_read(
    state: State<'_, AppState>,
    path: std::path::PathBuf,
) -> IdeResult<ide_dotnet::binlog::Binlog> {
    ide_host::ops::dotnet::dotnet_binlog_read(&state.host, path).await
}

/// The binary logs of this project's builds, newest first.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_binlogs(window: Window, state: State<'_, AppState>) -> IdeResult<Vec<IpcPath>> {
    ide_host::ops::dotnet::dotnet_binlogs(&state.host, window.label()).await
}

/// The solution's projects that use Entity Framework Core.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_ef_projects(window: Window, state: State<'_, AppState>) -> IdeResult<Vec<IpcPath>> {
    ide_host::ops::dotnet::dotnet_ef_projects(&state.host, window.label()).await
}

/// The EF Core migrations of `project`, oldest first.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_ef_migrations(
    window: Window,
    state: State<'_, AppState>,
    project: IpcPath,
) -> IdeResult<Vec<String>> {
    ide_host::ops::dotnet::dotnet_ef_migrations(&state.host, window.label(), project).await
}

/// The `dotnet ef` command line of `operation` on `project`.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_ef_command(
    window: Window,
    state: State<'_, AppState>,
    project: IpcPath,
    operation: ide_dotnet::ef::EfOperation,
) -> IdeResult<ide_dotnet::ef::EfCommand> {
    ide_host::ops::dotnet::dotnet_ef_command(&state.host, window.label(), project, operation).await
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
