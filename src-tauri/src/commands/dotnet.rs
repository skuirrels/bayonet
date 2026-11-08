//! IPC commands of `ide_host::ops::dotnet` (the shared implementation).

use crate::state::AppState;
use ide_core::RequestId;
use ide_core::{IdeError, IdeResult, IpcPath};
use ide_dotnet::Solution;
use ide_dotnet::nuget::{InstalledPackages, PackageAudit, PackageSearch};
use ide_dotnet::templates::{ProjectTemplate, TemplateValue};
use ide_dotnet::testing::{TestEvent, TestProject, TestSelection};
use std::path::{Path, PathBuf};
use tauri::ipc::Channel;
use tauri::{AppHandle, State, Window};

/// Solution files at the project root: the one opened last first, then `.slnx` before `.sln` before `.slnf`.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_solutions(window: Window, state: State<'_, AppState>) -> IdeResult<Vec<IpcPath>> {
    ide_host::ops::dotnet::dotnet_solutions(&state.host, window.label()).await
}

/// The folder that opens as the project for the solution file `path` (File › Open Solution…,
/// a drop on the Welcome screen); None when `path` is not one (a folder named `App.sln`).
#[tauri::command]
#[specta::specta]
pub async fn dotnet_solution_folder(path: PathBuf) -> IdeResult<Option<IpcPath>> {
    blocking(move || {
        let file = canonical(&path)?;
        Ok(ide_dotnet::solution_folder(&file).map(IpcPath::from))
    })
    .await
}

/// Makes the solution file `path` its folder's solution, once that folder is open as the
/// project; the window that has it open reloads it.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_choose_solution(app: AppHandle, path: PathBuf) -> IdeResult<()> {
    blocking(move || crate::windows::choose_solution(&app, &canonical(&path)?)).await
}

fn canonical(path: &Path) -> IdeResult<PathBuf> {
    dunce::canonicalize(path).map_err(|e| IdeError::not_found(format!("{}: {e}", path.display())))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> IdeResult<T> + Send + 'static) -> IdeResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| IdeError::internal(e.to_string()))?
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

/// The publish profiles of `project` (BLD-6).
#[tauri::command]
#[specta::specta]
pub async fn dotnet_publish_profiles(
    window: Window,
    state: State<'_, AppState>,
    project: IpcPath,
) -> IdeResult<Vec<ide_dotnet::publish::PublishProfile>> {
    ide_host::ops::dotnet::dotnet_publish_profiles(&state.host, window.label(), project).await
}

/// The `dotnet publish` command line for publishing `project` to `target`.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_publish_command(
    window: Window,
    state: State<'_, AppState>,
    project: IpcPath,
    target: ide_dotnet::publish::PublishTarget,
) -> IdeResult<ide_dotnet::publish::PublishCommand> {
    ide_host::ops::dotnet::dotnet_publish_command(&state.host, window.label(), project, target).await
}

/// The shell line that publishes `project` as a container image and runs it (PLT-5).
#[tauri::command]
#[specta::specta]
pub async fn dotnet_container_run(
    window: Window,
    state: State<'_, AppState>,
    project: IpcPath,
    repository: String,
) -> IdeResult<ide_dotnet::publish::PublishCommand> {
    ide_host::ops::dotnet::dotnet_container_run(&state.host, window.label(), project, repository).await
}

/// Opens the way to a project's user secrets: its `secrets.json`, created if needed (PLT-4).
#[tauri::command]
#[specta::specta]
pub async fn dotnet_user_secrets(window: Window, state: State<'_, AppState>, project: IpcPath) -> IdeResult<IpcPath> {
    ide_host::ops::dotnet::dotnet_user_secrets(&state.host, window.label(), project).await
}

/// Vulnerable and deprecated packages of the solution's projects (NUG-4).
#[tauri::command]
#[specta::specta]
pub async fn nuget_audit(window: Window, state: State<'_, AppState>) -> IdeResult<PackageAudit> {
    ide_host::ops::dotnet::nuget_audit(&state.host, window.label()).await
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
/// A `debug` run starts each test app waiting for a debugger and reports its
/// process (`TestEvent::Debug`) for the client to attach to.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn dotnet_tests(
    window: Window,
    state: State<'_, AppState>,
    request: RequestId,
    run: bool,
    coverage: bool,
    debug: bool,
    selection: Vec<TestSelection>,
    on_event: Channel<TestEvent>,
) -> IdeResult<()> {
    ide_host::ops::dotnet::dotnet_tests(
        &state.host,
        window.label(),
        request,
        run,
        coverage,
        debug,
        selection,
        crate::commands::sender(on_event),
    )
    .await
}

/// Avalonia previewer for a XAML file (CI-16); runs until `cancel_request(request)`.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn dotnet_xaml_preview(
    window: Window,
    state: State<'_, AppState>,
    request: RequestId,
    path: std::path::PathBuf,
    text: String,
    scale: f64,
    on_event: Channel<ide_dotnet::avalonia::PreviewEvent>,
) -> IdeResult<()> {
    ide_host::ops::dotnet::dotnet_xaml_preview(
        &state.host,
        window.label(),
        request,
        path,
        text,
        scale,
        crate::commands::sender(on_event),
    )
    .await
}

/// New XAML text for a running preview; false when it has stopped.
#[tauri::command]
#[specta::specta]
pub fn dotnet_xaml_preview_update(request: RequestId, text: String) -> IdeResult<bool> {
    Ok(ide_host::ops::dotnet::dotnet_xaml_preview_update(request, text))
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

/// Where the side compiles of CI-17 write each project's generated sources.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_generated_dirs(
    window: Window,
    state: State<'_, AppState>,
    projects: Vec<IpcPath>,
) -> IdeResult<Vec<IpcPath>> {
    ide_host::ops::dotnet::dotnet_generated_dirs(&state.host, window.label(), projects).await
}

/// `project`'s analysers and the files its source generators write.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_generated_sources(
    window: Window,
    state: State<'_, AppState>,
    project: IpcPath,
) -> IdeResult<ide_dotnet::generators::GeneratedSources> {
    ide_host::ops::dotnet::dotnet_generated_sources(&state.host, window.label(), project).await
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

/// Each project's frameworks, packages and project references for the Solution view.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_dependencies(
    window: Window,
    state: State<'_, AppState>,
    projects: Vec<IpcPath>,
) -> IdeResult<Vec<Option<ide_dotnet::dependencies::ProjectDependencies>>> {
    ide_host::ops::dotnet::dotnet_dependencies(&state.host, window.label(), projects).await
}

/// Of `projects`, those whose NuGet restore is missing or out of date.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_restore_needed(
    window: Window,
    state: State<'_, AppState>,
    projects: Vec<IpcPath>,
) -> IdeResult<Vec<IpcPath>> {
    ide_host::ops::dotnet::dotnet_restore_needed(&state.host, window.label(), projects).await
}

/// Restores the NuGet packages of `targets` and publishes their errors as `nuget` diagnostics.
#[tauri::command]
#[specta::specta]
pub async fn dotnet_restore(
    window: Window,
    state: State<'_, AppState>,
    targets: Vec<IpcPath>,
    clear: Vec<IpcPath>,
    request: RequestId,
) -> IdeResult<ide_dotnet::restore::RestoreResult> {
    ide_host::ops::dotnet::dotnet_restore(&state.host, window.label(), targets, clear, request).await
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
