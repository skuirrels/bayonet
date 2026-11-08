//! .NET solutions (what the Solution Explorer shows) and NuGet packages.
use crate::Host;
use crate::rpc::Dispatcher;

use ide_core::RequestId;
use ide_core::{IdeError, IdeResult, IpcPath};
use ide_dotnet::Solution;
use ide_dotnet::nuget::{self, InstalledPackages, PackageAudit, PackageSearch};
use ide_dotnet::templates::{self, ProjectTemplate, TemplateValue};
use ide_dotnet::testing::{self, SessionRequest, TestApp, TestEvent, TestPhase, TestProject, TestSelection};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};

use super::Channel;

/// Solution files at the project root: the one opened last first, then `.slnx` before `.sln` before `.slnf`.
pub async fn dotnet_solutions(host: &Host, window: &str) -> IdeResult<Vec<IpcPath>> {
    let root = host.project(window)?.root.clone();
    host.blocking(move || Ok(ide_dotnet::find(&root).into_iter().map(IpcPath::from).collect())).await
}

/// The solution at `path`: its folders, projects and solution items.
pub async fn dotnet_solution(host: &Host, _window: &str, path: IpcPath) -> IdeResult<Solution> {
    host.blocking(move || ide_dotnet::read(&path)).await
}

/// The project root and the `dotnet` the project uses (Settings, `global.json`, else PATH).
fn dotnet_of(host: &Host, window: &str) -> IdeResult<(PathBuf, Option<PathBuf>)> {
    let project = host.project(window)?;
    let root = project.root.clone();
    let dotnet = ide_sdk::apply::dotnet_command(&root, &project.settings.effective());
    Ok((root, dotnet))
}

/// Avalonia previewer for the XAML file at `path` (CI-16): builds its project, starts the
/// designer host and streams frames on `on_event` until `cancel_request(request)`.
/// `text` is the editor's current text; later text goes through `dotnet_xaml_preview_update`.
pub async fn dotnet_xaml_preview(
    host: &Host,
    window: &str,
    request: RequestId,
    path: PathBuf,
    text: String,
    scale: f64,
    on_event: Channel<ide_dotnet::avalonia::PreviewEvent>,
) -> IdeResult<()> {
    use ide_dotnet::avalonia::{self, PreviewEvent, PreviewPhase};
    let (_, dotnet) = dotnet_of(host, window)?;
    let dotnet = dotnet.unwrap_or_else(|| PathBuf::from("dotnet"));
    let token = host.begin_request(request);
    let emit = |e: PreviewEvent| {
        let _ = on_event.send(e);
    };
    let scale = if scale.is_finite() { scale.clamp(1.0, 4.0) } else { 1.0 };
    let result = avalonia::run(request.0.into(), &dotnet, &path, text, scale, &token, &emit).await;
    host.end_request(request);
    match result {
        Err(e) if e.kind == ide_core::error::ErrorKind::Cancelled => Ok(()),
        Err(e) => {
            emit(PreviewEvent::Phase { phase: PreviewPhase::Failed, message: Some(e.message.clone()) });
            Err(e)
        }
        ok => ok,
    }
}

/// New XAML text for the preview started as `request`; false when it is not running.
pub fn dotnet_xaml_preview_update(request: RequestId, text: String) -> bool {
    ide_dotnet::avalonia::update(request.0.into(), text)
}

/// Searches the package sources of the project's `nuget.config` (`skip` / `take` page).
pub async fn nuget_search(
    host: &Host,
    window: &str,
    query: String,
    prerelease: bool,
    skip: u32,
    take: u32,
) -> IdeResult<PackageSearch> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let out = nuget::run(dotnet.as_deref(), &root, &nuget::search_args(&query, prerelease, skip, take))?;
        nuget::parse_search(&out)
    })
    .await
}

/// Every version of package `id`, newest first.
pub async fn nuget_versions(host: &Host, window: &str, id: String, prerelease: bool) -> IdeResult<Vec<String>> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        nuget::parse_versions(&nuget::run(dotnet.as_deref(), &root, &nuget::versions_args(&id, prerelease))?)
    })
    .await
}

/// The packages of every project of the solution at the root (else of the root's
/// project); `outdated` asks the sources for the latest versions too.
pub async fn nuget_installed(
    host: &Host,
    window: &str,
    outdated: bool,
    prerelease: bool,
) -> IdeResult<InstalledPackages> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let target = ide_dotnet::find(&root).into_iter().next().unwrap_or_else(|| root.clone());
        let out = nuget::run(dotnet.as_deref(), &root, &nuget::list_args(&target, outdated, prerelease))?;
        Ok(nuget::with_central_packages(nuget::parse_list(&out)?))
    })
    .await
}

/// Vulnerable (with the packages under them) and deprecated packages of every project
/// of the solution at the root, placed in their project files. Asks the package sources.
pub async fn nuget_audit(host: &Host, window: &str) -> IdeResult<PackageAudit> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let target = ide_dotnet::find(&root).into_iter().next().unwrap_or_else(|| root.clone());
        let mut listings = Vec::new();
        let mut failures = Vec::new();
        for vulnerable in [true, false] {
            match nuget::run(dotnet.as_deref(), &root, &nuget::audit_args(&target, vulnerable)) {
                Ok(out) => listings.push(out),
                Err(e) => failures.push(e),
            }
        }
        if listings.is_empty() {
            return Err(failures.remove(0));
        }
        let mut audit = nuget::parse_audit(&listings.iter().map(String::as_str).collect::<Vec<_>>())?;
        audit.problems.extend(failures.into_iter().map(|e| e.message));
        Ok(nuget::locate_findings(audit))
    })
    .await
}

/// Installs package `id` into `projects`, or moves them to `version` (else the latest
/// stable). Projects under central package management get the version in `Directory.Packages.props`.
pub async fn nuget_install(
    host: &Host,
    window: &str,
    projects: Vec<IpcPath>,
    id: String,
    version: Option<String>,
) -> IdeResult<()> {
    let (root, dotnet) = dotnet_of(host, window)?;
    if projects.is_empty() {
        return Err(IdeError::invalid("no project to install into"));
    }
    host.blocking(move || {
        for p in &projects {
            nuget::run(dotnet.as_deref(), &root, &nuget::add_args(p, &id, version.as_deref()))?;
        }
        Ok(())
    })
    .await
}

/// Removes package `id` from `projects`.
pub async fn nuget_remove(host: &Host, window: &str, projects: Vec<IpcPath>, id: String) -> IdeResult<()> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        for p in &projects {
            nuget::run(dotnet.as_deref(), &root, &nuget::remove_args(p, &id))?;
        }
        Ok(())
    })
    .await
}

/// The solution at the project root, required by solution edits.
fn solution_of(root: &Path) -> IdeResult<PathBuf> {
    ide_dotnet::find(root).into_iter().next().ok_or_else(|| IdeError::not_found("the project has no solution file"))
}

/// Installed project and solution templates (`dotnet new`), custom ones included.
pub async fn dotnet_templates(host: &Host, window: &str) -> IdeResult<Vec<ProjectTemplate>> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let version = nuget::run(dotnet.as_deref(), &root, &["--version".to_owned()])?.trim().to_owned();
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .ok_or_else(|| IdeError::not_found("no home directory"))?;
        let file = templates::cache_file(&home, &version);
        if !file.is_file() {
            // The cache is written on first use of `dotnet new`.
            nuget::run(dotnet.as_deref(), &root, &["new".into(), "list".into()])?;
        }
        templates::parse_cache(&std::fs::read_to_string(&file)?)
    })
    .await
}

/// Creates a project from `template` in `directory` and, with `add_to_solution`, adds
/// it to the root's solution (in `solution_folder`). Returns the new project file.
#[allow(clippy::too_many_arguments)]
pub async fn dotnet_new_project(
    host: &Host,
    window: &str,
    template: String,
    name: String,
    directory: IpcPath,
    language: Option<String>,
    values: Vec<TemplateValue>,
    add_to_solution: bool,
    solution_folder: Option<String>,
) -> IdeResult<IpcPath> {
    let (root, dotnet) = dotnet_of(host, window)?;
    if name.trim().is_empty() || name.contains(['/', '\\']) {
        return Err(IdeError::invalid("enter a project name"));
    }
    host.blocking(move || {
        let args = templates::new_args(&template, &name, &directory, language.as_deref(), &values);
        nuget::run(dotnet.as_deref(), &root, &args)?;
        let project = templates::created_project(&directory, &name)
            .ok_or_else(|| IdeError::not_found(format!("{template} created no project in {}", directory.display())))?;
        if add_to_solution {
            let solution = solution_of(&root)?;
            nuget::run(
                dotnet.as_deref(),
                &root,
                &templates::sln_add_args(&solution, std::slice::from_ref(&project), solution_folder.as_deref()),
            )?;
        }
        Ok(IpcPath::from(project))
    })
    .await
}

/// Adds existing projects to the root's solution.
pub async fn dotnet_solution_add(
    host: &Host,
    window: &str,
    projects: Vec<IpcPath>,
    solution_folder: Option<String>,
) -> IdeResult<()> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let solution = solution_of(&root)?;
        let paths: Vec<PathBuf> = projects.into_iter().map(PathBuf::from).collect();
        nuget::run(dotnet.as_deref(), &root, &templates::sln_add_args(&solution, &paths, solution_folder.as_deref()))
            .map(|_| ())
    })
    .await
}

/// Removes projects from the root's solution (their files stay).
pub async fn dotnet_solution_remove(host: &Host, window: &str, projects: Vec<IpcPath>) -> IdeResult<()> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let solution = solution_of(&root)?;
        let paths: Vec<PathBuf> = projects.into_iter().map(PathBuf::from).collect();
        nuget::run(dotnet.as_deref(), &root, &templates::sln_remove_args(&solution, &paths)).map(|_| ())
    })
    .await
}

/// Converts a `.sln` to `.slnx` next to it (SLN-6); the `.sln` stays. Returns the new file.
pub async fn dotnet_migrate_solution(host: &Host, window: &str, solution: IpcPath) -> IdeResult<IpcPath> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let solution = PathBuf::from(solution);
        let target = templates::migrated_solution(&solution);
        if target.exists() {
            return Err(IdeError::new(
                ide_core::error::ErrorKind::AlreadyExists,
                format!("{} already exists", target.file_name().unwrap_or_default().to_string_lossy()),
            ));
        }
        nuget::run(dotnet.as_deref(), &root, &templates::sln_migrate_args(&solution))?;
        ide_dotnet::solution_replaced(&solution, &target)?;
        Ok(IpcPath::from(target))
    })
    .await
}

/// The projects of the root's solution (none without one).
fn solution_projects(root: &Path) -> Vec<PathBuf> {
    ide_dotnet::find(root)
        .into_iter()
        .next()
        .and_then(|s| ide_dotnet::read(&s).ok())
        .map(|s| s.projects())
        .unwrap_or_default()
}

/// The solution's projects that use Entity Framework Core (PLT-2).
pub async fn dotnet_ef_projects(host: &Host, window: &str) -> IdeResult<Vec<IpcPath>> {
    let root = host.project(window)?.root.clone();
    host.blocking(move || {
        Ok(solution_projects(&root)
            .into_iter()
            .filter(|p| p.parent().is_some_and(ide_dotnet::ef::uses_ef_core))
            .map(IpcPath::from)
            .collect())
    })
    .await
}

/// The migrations of `project`, oldest first.
pub async fn dotnet_ef_migrations(host: &Host, _window: &str, project: IpcPath) -> IdeResult<Vec<String>> {
    host.blocking(move || Ok(ide_dotnet::ef::migrations(project.parent().unwrap_or(&project)))).await
}

/// The `dotnet ef` command line of `operation` on `project` (run in the Run tool window).
pub async fn dotnet_ef_command(
    host: &Host,
    window: &str,
    project: IpcPath,
    operation: ide_dotnet::ef::EfOperation,
) -> IdeResult<ide_dotnet::ef::EfCommand> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let dotnet = dotnet.unwrap_or_else(|| PathBuf::from("dotnet"));
        Ok(ide_dotnet::ef::command(&dotnet, &operation, &project, &solution_projects(&root)))
    })
    .await
}

/// Where the .NET SDK the project uses defines MSBuild target `name` (CI-18).
pub async fn dotnet_sdk_target(
    host: &Host,
    window: &str,
    name: String,
) -> IdeResult<Option<ide_dotnet::msbuild::FileLocation>> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        let dotnet = dotnet.unwrap_or_else(|| PathBuf::from("dotnet"));
        let Some(sdk) = ide_dotnet::msbuild::sdk_dir(&dotnet, &root) else { return Ok(None) };
        Ok(ide_dotnet::msbuild::find_target(&ide_dotnet::msbuild::sdk_files(&sdk), &name))
    })
    .await
}

/// Reads the MSBuild binary log at `path` for the binary log viewer (BLD-3).
pub async fn dotnet_binlog_read(host: &Host, path: PathBuf) -> IdeResult<ide_dotnet::binlog::Binlog> {
    host.blocking(move || ide_dotnet::binlog::read(&path)).await
}

/// The binary logs builds of this project wrote (Settings › .NET › binary log), newest first.
pub async fn dotnet_binlogs(host: &Host, window: &str) -> IdeResult<Vec<IpcPath>> {
    let dir = host.project(window)?.build_cache_dir().join("binlogs");
    host.blocking(move || {
        let mut logs: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "binlog"))
            .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
            .collect();
        logs.sort_by_key(|l| std::cmp::Reverse(l.0));
        Ok(logs.into_iter().map(|(_, p)| IpcPath::from(p)).collect())
    })
    .await
}

/// Where the generated sources of each of `projects` go (CI-17), created
/// empty when missing so the tree can list them before the first compile.
pub async fn dotnet_generated_dirs(host: &Host, window: &str, projects: Vec<IpcPath>) -> IdeResult<Vec<IpcPath>> {
    let base = host.project(window)?.build_cache_dir().join("generated");
    host.blocking(move || {
        Ok(projects
            .into_iter()
            .map(|p| {
                let dir = ide_dotnet::generators::output_dir(&base, &PathBuf::from(p));
                let _ = std::fs::create_dir_all(&dir);
                IpcPath::from(dir)
            })
            .collect())
    })
    .await
}

/// One side compile at a time: they share the compiler server and the cache.
static GENERATING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Runs `dotnet <args>` in `cwd`; stdout and stderr.
async fn dotnet_output(dotnet: &Path, cwd: &Path, args: &[String]) -> IdeResult<(String, String)> {
    let (_, stdout, stderr) = dotnet_status_output(dotnet, cwd, args).await?;
    Ok((stdout, stderr))
}

/// Compiles `project` once more with `EmitCompilerGeneratedFiles` into the
/// IDE's cache and lists its analysers and generated files (CI-17). The
/// project's own `obj` and `bin` are left alone; see `ide_dotnet::generators`.
/// A compile that fails without writing anything keeps the earlier listing.
pub async fn dotnet_generated_sources(
    host: &Host,
    window: &str,
    project: IpcPath,
) -> IdeResult<ide_dotnet::generators::GeneratedSources> {
    use ide_dotnet::generators::{self, GeneratedSources};
    let (_, dotnet) = dotnet_of(host, window)?;
    let dotnet = dotnet.unwrap_or_else(|| PathBuf::from("dotnet"));
    let base = host.project(window)?.build_cache_dir().join("generated");
    let project = PathBuf::from(project);
    let dir = generators::output_dir(&base, &project);
    let listing = |dir: &Path, errors: Vec<String>, stale: bool| GeneratedSources {
        dir: dir.to_path_buf().into(),
        analyzers: generators::analyzer_names(dir),
        files: generators::generated_files(dir).into_iter().map(IpcPath::from).collect(),
        needs_build: generators::needs_build(&errors),
        errors,
        stale,
    };
    // F# has no source generators (and no Roslyn analysers).
    if !project.extension().is_some_and(|e| e.eq_ignore_ascii_case("csproj") || e.eq_ignore_ascii_case("vbproj")) {
        return Ok(listing(&dir, Vec::new(), false));
    }
    let _guard = GENERATING.lock().await;
    let cwd = project.parent().unwrap_or(&project).to_path_buf();
    let path = project.to_string_lossy().into_owned();
    // The project's own CustomAfterMicrosoftCommonTargets (the global property
    // below would hide it) and its target frameworks, as MSBuild evaluates them.
    let mut eval: Vec<String> = vec!["msbuild".into(), path.clone(), "-nologo".into()];
    eval.extend(generators::EVAL_PROPERTIES.iter().map(|n| format!("-getProperty:{n}")));
    let props = generators::parse_properties(&dotnet_output(&dotnet, &cwd, &eval).await?.0);
    let prop = |k: &str| props.iter().find(|(n, _)| n == k).map(|(_, v)| v.trim().to_owned()).unwrap_or_default();
    let user_import = prop("CustomAfterMicrosoftCommonTargets");
    let first_tfm = prop("TargetFrameworks").split(';').map(str::trim).find(|t| !t.is_empty()).map(str::to_owned);

    let work = base.join("work");
    let (obj, next, targets) = (work.join("obj"), work.join("out"), work.join("generated.targets"));
    generators::remove_listing(&work);
    std::fs::create_dir_all(&next).map_err(|e| IdeError::internal(format!("cannot create {}: {e}", next.display())))?;
    std::fs::write(&targets, generators::targets_file(&project, &obj, &next, Some(&user_import)))
        .map_err(|e| IdeError::internal(format!("cannot write {}: {e}", targets.display())))?;
    let mut args: Vec<String> = vec![
        "msbuild".into(),
        path,
        "-nologo".into(),
        "-v:q".into(),
        "-t:Compile".into(),
        "-p:BuildProjectReferences=false".into(),
        format!("-p:CustomAfterMicrosoftCommonTargets={}", targets.display()),
        "-getItem:Analyzer".into(),
    ];
    if prop("TargetFramework").is_empty()
        && let Some(first) = first_tfm
    {
        args.push(format!("-p:TargetFramework={first}"));
    }
    let (stdout, stderr) = dotnet_output(&dotnet, &cwd, &args).await?;
    let errors = generators::errors(&format!("{stdout}\n{stderr}"));
    let analyzers = generators::parse_analyzers(&stdout);
    host.blocking(move || {
        // Failed and wrote nothing (code being edited, references not built):
        // keep showing what the last good compile generated.
        if !errors.is_empty() && (analyzers.is_empty() || generators::generated_files(&next).is_empty()) {
            generators::remove_listing(&work);
            let had = !generators::generated_files(&dir).is_empty();
            return Ok(listing(&dir, errors, had));
        }
        generators::add_analyzer_folders(&next, &analyzers)
            .map_err(|e| IdeError::internal(format!("cannot write {}: {e}", next.display())))?;
        generators::make_read_only(&generators::generated_files(&next));
        // Swap the new listing in whole, so the tree never shows half of it.
        generators::remove_listing(&dir);
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent).map_err(|e| IdeError::internal(e.to_string()))?;
        }
        std::fs::rename(&next, &dir).map_err(|e| IdeError::internal(format!("cannot move {}: {e}", dir.display())))?;
        generators::remove_listing(&work);
        Ok(listing(&dir, errors, false))
    })
    .await
}

/// The publish profiles of `project` (BLD-6).
pub async fn dotnet_publish_profiles(
    host: &Host,
    _window: &str,
    project: IpcPath,
) -> IdeResult<Vec<ide_dotnet::publish::PublishProfile>> {
    host.blocking(move || Ok(ide_dotnet::publish::profiles(&project))).await
}

/// The `dotnet publish` command line of `project` to `target`, with the project's `dotnet`.
pub async fn dotnet_publish_command(
    host: &Host,
    window: &str,
    project: IpcPath,
    target: ide_dotnet::publish::PublishTarget,
) -> IdeResult<ide_dotnet::publish::PublishCommand> {
    let (_, dotnet) = dotnet_of(host, window)?;
    let dotnet = dotnet.unwrap_or_else(|| PathBuf::from("dotnet"));
    Ok(ide_dotnet::publish::command(&dotnet, &project, &target))
}

/// Run in Container (PLT-5): the shell line that publishes `project` as image
/// `repository` and runs it, a web project's launch port mapped to the container's.
pub async fn dotnet_container_run(
    host: &Host,
    window: &str,
    project: IpcPath,
    repository: String,
) -> IdeResult<ide_dotnet::publish::PublishCommand> {
    let (_, dotnet) = dotnet_of(host, window)?;
    let dotnet = dotnet.unwrap_or_else(|| PathBuf::from("dotnet"));
    host.blocking(move || {
        let port = ide_dotnet::publish::web_host_port(&project);
        Ok(ide_dotnet::publish::container_run(&dotnet, &project, &repository, port))
    })
    .await
}

/// The `secrets.json` of `project` (PLT-4), with a `UserSecretsId` added when it has none.
pub async fn dotnet_user_secrets(host: &Host, window: &str, project: IpcPath) -> IdeResult<IpcPath> {
    let (_, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || ide_dotnet::secrets::ensure_secrets_file(dotnet.as_deref(), &project).map(IpcPath::from))
        .await
}

/// The projects `project` references.
pub async fn dotnet_project_references(host: &Host, _window: &str, project: IpcPath) -> IdeResult<Vec<IpcPath>> {
    host.blocking(move || Ok(templates::project_references(&project)?.into_iter().map(IpcPath::from).collect())).await
}

/// The Dependencies of each project (SLN-1), in the order given; a project that cannot
/// be read gets none.
pub async fn dotnet_dependencies(
    host: &Host,
    _window: &str,
    projects: Vec<IpcPath>,
) -> IdeResult<Vec<Option<ide_dotnet::dependencies::ProjectDependencies>>> {
    host.blocking(move || {
        Ok(projects.into_iter().map(|p| ide_dotnet::dependencies::read_dependencies(&PathBuf::from(p)).ok()).collect())
    })
    .await
}

/// Of `projects`, those whose NuGet restore is missing or older than their inputs (NUG-1).
pub async fn dotnet_restore_needed(host: &Host, _window: &str, projects: Vec<IpcPath>) -> IdeResult<Vec<IpcPath>> {
    host.blocking(move || {
        Ok(projects
            .into_iter()
            .map(PathBuf::from)
            .filter(|p| p.is_file() && ide_dotnet::restore::needs_restore(p))
            .map(IpcPath::from)
            .collect())
    })
    .await
}

/// Diagnostics source of restore errors and warnings, on the project or solution they name.
const RESTORE_SOURCE: &str = "nuget";

/// Restores the NuGet packages of `targets` (projects or a solution), one after another
/// (NUG-1). Their errors and warnings replace the earlier `nuget` diagnostics of the files
/// they name and of `clear`. Stopped by `cancel_request(request)`.
pub async fn dotnet_restore(
    host: &Host,
    window: &str,
    targets: Vec<IpcPath>,
    clear: Vec<IpcPath>,
    request: RequestId,
) -> IdeResult<ide_dotnet::restore::RestoreResult> {
    use ide_dotnet::restore::{self, RestoreMessage, RestoreResult, RestoreSeverity};
    host.require_trust(window)?;
    let (root, dotnet) = dotnet_of(host, window)?;
    let dotnet = dotnet.unwrap_or_else(|| PathBuf::from("dotnet"));
    let sink = host.project(window)?.sink.clone();
    let token = host.begin_project_request(window, request);
    let run = async {
        let mut result = RestoreResult { success: true, messages: Vec::new() };
        for target in targets {
            let target = PathBuf::from(target);
            let args = restore::restore_args(&target);
            let started = std::time::SystemTime::now();
            let (code, stdout, stderr) = tokio::select! {
                out = dotnet_status_output(&dotnet, &root, &args) => out?,
                () = token.cancelled() => return Err(IdeError::cancelled()),
            };
            let mut messages = restore::parse_output(&format!("{stdout}\n{stderr}"));
            let failed = code != 0 || messages.iter().any(|m| m.severity == RestoreSeverity::Error);
            if code != 0 && !messages.iter().any(|m| m.severity == RestoreSeverity::Error) {
                let last = stderr.lines().chain(stdout.lines()).rfind(|l| !l.trim().is_empty());
                messages.push(RestoreMessage {
                    severity: RestoreSeverity::Error,
                    code: None,
                    text: last
                        .map_or_else(|| format!("dotnet restore exited with code {code}"), |l| l.trim().to_owned()),
                    project: Some(IpcPath::from(target.clone())),
                });
            }
            result.success &= !failed;
            if !failed {
                let target = target.clone();
                let projects =
                    tokio::task::spawn_blocking(move || restore::restored_by(&target)).await.unwrap_or_default();
                restore::mark_restored(projects, started);
            }
            for m in messages {
                if !result.messages.contains(&m) {
                    result.messages.push(m);
                }
            }
        }
        Ok(result)
    };
    let result = run.await;
    host.end_request(request);
    let result = result?;
    let mut by_file: std::collections::BTreeMap<PathBuf, Vec<ide_core::Diagnostic>> =
        clear.into_iter().map(|p| (PathBuf::from(p), Vec::new())).collect();
    for m in &result.messages {
        let Some(path) = m.project.clone() else { continue };
        by_file.entry(PathBuf::from(path)).or_default().push(ide_core::Diagnostic {
            range: ide_core::Range::default(),
            severity: match m.severity {
                RestoreSeverity::Error => ide_core::Severity::Error,
                RestoreSeverity::Warning => ide_core::Severity::Warning,
            },
            message: m.text.clone(),
            source: Some("NuGet".into()),
            code: m.code.clone(),
            unnecessary: false,
            deprecated: false,
            server: Some(RESTORE_SOURCE.into()),
        });
    }
    for (path, items) in by_file {
        sink.emit(ide_core::IdeEvent::Diagnostics { path: path.into(), source: RESTORE_SOURCE.into(), items });
    }
    Ok(result)
}

/// Runs `dotnet <args>` in `cwd`: exit code, stdout and stderr; killed when dropped.
async fn dotnet_status_output(dotnet: &Path, cwd: &Path, args: &[String]) -> IdeResult<(i32, String, String)> {
    let out = tokio::process::Command::new(dotnet)
        .args(args)
        .current_dir(cwd)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_UI_LANGUAGE", "en")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", dotnet.display())))?;
    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

/// Adds and removes project references of `project`.
pub async fn dotnet_edit_references(
    host: &Host,
    window: &str,
    project: IpcPath,
    add: Vec<IpcPath>,
    remove: Vec<IpcPath>,
) -> IdeResult<()> {
    let (root, dotnet) = dotnet_of(host, window)?;
    host.blocking(move || {
        for (adding, list) in [(false, remove), (true, add)] {
            if !list.is_empty() {
                let paths: Vec<PathBuf> = list.into_iter().map(PathBuf::from).collect();
                nuget::run(dotnet.as_deref(), &root, &templates::reference_args(adding, &project, &paths))?;
            }
        }
        Ok(())
    })
    .await
}

/// The test projects of the solution at the root.
pub async fn dotnet_test_projects(host: &Host, window: &str) -> IdeResult<Vec<TestProject>> {
    let root = host.project(window)?.root.clone();
    host.blocking(move || {
        let solution = ide_dotnet::find(&root).into_iter().next();
        Ok(testing::test_projects(solution.as_deref(), &root))
    })
    .await
}

/// Builds, then discovers (`run` false) or runs the tests of `selection` (every
/// test project when empty), streaming progress and results on `on_event`.
/// Cancellable with `cancel_request(request)`.
#[allow(clippy::too_many_arguments)]
pub async fn dotnet_tests(
    host: &Host,
    window: &str,
    request: RequestId,
    run: bool,
    coverage: bool,
    debug: bool,
    selection: Vec<TestSelection>,
    on_event: Channel<TestEvent>,
) -> IdeResult<()> {
    let (root, dotnet) = dotnet_of(host, window)?;
    let token = host.begin_request(request);
    // A debug run collects no coverage: the coverage host would start the tests in another process.
    let coverage = (run && coverage && !debug).then(|| coverage_dir(request));
    let debug = run && debug;
    let result = tests_inner(&root, dotnet, run, coverage.as_deref(), debug, selection, &token, &on_event).await;
    host.end_request(request);
    if let Err(e) = &result {
        let _ = on_event.send(TestEvent::Phase {
            project: None,
            phase: TestPhase::Failed,
            message: Some(e.message.clone()),
        });
    }
    result
}

/// Where a coverage run writes its reports: a fresh folder per run.
fn coverage_dir(request: RequestId) -> PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    std::env::temp_dir().join("bayonet-coverage").join(format!("{}-{}-{stamp}", std::process::id(), request.0))
}

#[allow(clippy::too_many_arguments)]
async fn tests_inner(
    root: &Path,
    dotnet: Option<PathBuf>,
    run: bool,
    coverage: Option<&Path>,
    debug: bool,
    selection: Vec<TestSelection>,
    token: &ide_core::CancellationToken,
    on_event: &Channel<TestEvent>,
) -> IdeResult<()> {
    let dotnet = dotnet.unwrap_or_else(|| PathBuf::from("dotnet"));
    let solution = ide_dotnet::find(root).into_iter().next();
    let selection = if selection.is_empty() {
        let (s, r) = (solution.clone(), root.to_path_buf());
        tokio::task::spawn_blocking(move || testing::test_projects(s.as_deref(), &r))
            .await
            .map_err(|e| IdeError::internal(e.to_string()))?
            .into_iter()
            .map(|p| TestSelection { project: p.path, uids: Vec::new() })
            .collect()
    } else {
        selection
    };
    if selection.is_empty() {
        return Err(IdeError::not_found("no test projects in the solution"));
    }
    let send = |e: TestEvent| {
        let _ = on_event.send(e);
    };
    send(TestEvent::Phase { project: None, phase: TestPhase::Building, message: None });
    // One build of the solution when every test project is wanted, else of each project.
    let targets: Vec<PathBuf> = match &solution {
        Some(s) if selection.iter().all(|t| t.uids.is_empty()) && selection.len() > 1 => vec![s.clone()],
        _ => selection.iter().map(|t| t.project.to_path_buf()).collect(),
    };
    for target in &targets {
        let mut args: Vec<String> = vec!["build".into(), target.to_string_lossy().into_owned()];
        args.extend(["-nologo", "-tl:off", "-v:minimal", "-clp:NoSummary"].map(String::from));
        let out = on_event.clone();
        let on_line = move |line| {
            let _ = out.send(TestEvent::Output { project: None, line });
        };
        let code = stream_dotnet(&dotnet, root, &args, token, on_line).await?;
        if code != 0 {
            return Err(IdeError::new(ide_core::error::ErrorKind::Io, "build failed; see the output"));
        }
    }
    for t in &selection {
        if token.is_cancelled() {
            return Err(IdeError::cancelled());
        }
        let project = t.project.clone();
        let phase =
            |phase, message: Option<String>| TestEvent::Phase { project: Some(project.clone()), phase, message };
        send(phase(if run { TestPhase::Running } else { TestPhase::Discovering }, None));
        let mut app = match test_app(&dotnet, &t.project).await {
            Ok(app) => app,
            Err(e) => {
                send(phase(TestPhase::Failed, Some(e.message)));
                continue;
            }
        };
        let request = if run { SessionRequest::Run(t.uids.clone()) } else { SessionRequest::Discover };
        let dir = t.project.parent().map(Path::to_path_buf).unwrap_or_default();
        // Projects without the coverage extension run without coverage (the window says so).
        let report = coverage.filter(|_| testing::has_coverage_extension(&dir)).map(|out| {
            let name = format!(
                "{}.cobertura.xml",
                t.project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
            );
            app.args = testing::coverage_args(out, &name);
            out.join(name)
        });
        app.wait_for_debugger = debug;
        let out = on_event.clone();
        let out_project = project.clone();
        let started_project = project.clone();
        let result = testing::session_with(
            &app,
            request,
            token,
            |mut tests| {
                if !run {
                    testing::resolve_locations(&dir, &mut tests);
                }
                send(TestEvent::Tests { project: project.clone(), tests });
            },
            move |line| {
                let _ = out.send(TestEvent::Output { project: Some(out_project.clone()), line });
            },
            |pid| {
                if debug {
                    send(TestEvent::Debug { project: started_project, pid });
                }
            },
        )
        .await;
        if let Some(report) = report.filter(|r| r.is_file()) {
            send(TestEvent::Coverage { project: project.clone(), report: report.into() });
        }
        match result {
            Ok(()) => send(phase(TestPhase::Finished, None)),
            Err(e) if e.kind == ide_core::error::ErrorKind::Cancelled => return Err(e),
            Err(e) => send(phase(TestPhase::Failed, Some(e.message))),
        }
    }
    send(TestEvent::Phase { project: None, phase: TestPhase::Finished, message: None });
    Ok(())
}

/// The built test app of `project`: its `TargetPath` (first target framework when it has several).
async fn test_app(dotnet: &Path, project: &Path) -> IdeResult<TestApp> {
    let names = ["TargetPath", "IsTestingPlatformApplication"];
    let mut args: Vec<String> = vec!["msbuild".into(), project.to_string_lossy().into_owned()];
    args.extend(names.iter().map(|n| format!("-getProperty:{n}")));
    let info = {
        let p = project.to_path_buf();
        tokio::task::spawn_blocking(move || ide_dotnet::read_project(&p))
            .await
            .map_err(|e| IdeError::internal(e.to_string()))??
    };
    if info.target_frameworks.len() > 1
        && let Some(first) = info.target_frameworks.first()
    {
        args.push(format!("-p:TargetFramework={first}"));
    }
    let dir = project.parent().unwrap_or(project);
    let out = tokio::process::Command::new(dotnet)
        .args(&args)
        .current_dir(dir)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_UI_LANGUAGE", "en")
        .output()
        .await
        .map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", dotnet.display())))?;
    let props = testing::parse_msbuild_properties(&String::from_utf8_lossy(&out.stdout), &names);
    let get = |k: &str| props.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let assembly = get("TargetPath");
    if assembly.is_empty() || !Path::new(&assembly).is_file() {
        return Err(IdeError::not_found(format!("{} has no built test assembly", project.display())));
    }
    if !get("IsTestingPlatformApplication").eq_ignore_ascii_case("true") {
        return Err(IdeError::new(
            ide_core::error::ErrorKind::Unavailable,
            "this project runs on VSTest; only Microsoft.Testing.Platform test projects are supported so far",
        ));
    }
    Ok(TestApp {
        dotnet: dotnet.to_path_buf(),
        assembly: PathBuf::from(assembly),
        cwd: dir.to_path_buf(),
        env: Vec::new(),
        args: Vec::new(),
        wait_for_debugger: false,
    })
}

/// Runs `dotnet <args>`, calling `on_line` per output line; the exit code (cancellation kills it).
async fn stream_dotnet(
    dotnet: &Path,
    cwd: &Path,
    args: &[String],
    token: &ide_core::CancellationToken,
    on_line: impl Fn(String) + Send + Sync + Clone + 'static,
) -> IdeResult<i32> {
    let mut child = tokio::process::Command::new(dotnet)
        .args(args)
        .current_dir(cwd)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_UI_LANGUAGE", "en")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", dotnet.display())))?;
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let f = on_line.clone();
        readers.push(tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                f(l);
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let f = on_line.clone();
        readers.push(tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                f(l);
            }
        }));
    }
    let status = tokio::select! {
        s = child.wait() => s.map_err(IdeError::from)?,
        () = token.cancelled() => {
            let _ = child.kill().await;
            return Err(IdeError::cancelled());
        }
    };
    for r in readers {
        let _ = r.await;
    }
    Ok(status.code().unwrap_or(-1))
}

pub(crate) fn register(d: &mut Dispatcher) {
    d.add("dotnet_templates", |c| async move { dotnet_templates(&c.host, &c.window).await });
    d.add("dotnet_new_project", |c| async move {
        dotnet_new_project(
            &c.host,
            &c.window,
            c.arg("template")?,
            c.arg("name")?,
            c.arg("directory")?,
            c.arg("language")?,
            c.arg("values")?,
            c.arg("addToSolution")?,
            c.arg("solutionFolder")?,
        )
        .await
    });
    d.add("dotnet_solution_add", |c| async move {
        dotnet_solution_add(&c.host, &c.window, c.arg("projects")?, c.arg("solutionFolder")?).await
    });
    d.add(
        "dotnet_solution_remove",
        |c| async move { dotnet_solution_remove(&c.host, &c.window, c.arg("projects")?).await },
    );
    d.add("dotnet_migrate_solution", |c| async move {
        dotnet_migrate_solution(&c.host, &c.window, c.arg("solution")?).await
    });
    d.add("dotnet_sdk_target", |c| async move { dotnet_sdk_target(&c.host, &c.window, c.arg("name")?).await });
    d.add("dotnet_binlog_read", |c| async move { dotnet_binlog_read(&c.host, c.arg("path")?).await });
    d.add(
        "dotnet_generated_dirs",
        |c| async move { dotnet_generated_dirs(&c.host, &c.window, c.arg("projects")?).await },
    );
    d.add("dotnet_generated_sources", |c| async move {
        dotnet_generated_sources(&c.host, &c.window, c.arg("project")?).await
    });
    d.add("dotnet_binlogs", |c| async move { dotnet_binlogs(&c.host, &c.window).await });
    d.add("dotnet_ef_projects", |c| async move { dotnet_ef_projects(&c.host, &c.window).await });
    d.add("dotnet_ef_migrations", |c| async move { dotnet_ef_migrations(&c.host, &c.window, c.arg("project")?).await });
    d.add("dotnet_ef_command", |c| async move {
        dotnet_ef_command(&c.host, &c.window, c.arg("project")?, c.arg("operation")?).await
    });
    d.add("dotnet_dependencies", |c| async move { dotnet_dependencies(&c.host, &c.window, c.arg("projects")?).await });
    d.add(
        "dotnet_restore_needed",
        |c| async move { dotnet_restore_needed(&c.host, &c.window, c.arg("projects")?).await },
    );
    d.add("dotnet_restore", |c| async move {
        dotnet_restore(&c.host, &c.window, c.arg("targets")?, c.arg("clear")?, c.arg("request")?).await
    });
    d.add("dotnet_project_references", |c| async move {
        dotnet_project_references(&c.host, &c.window, c.arg("project")?).await
    });
    d.add("dotnet_edit_references", |c| async move {
        dotnet_edit_references(&c.host, &c.window, c.arg("project")?, c.arg("add")?, c.arg("remove")?).await
    });
    d.add("dotnet_xaml_preview", |c| async move {
        dotnet_xaml_preview(
            &c.host,
            &c.window,
            c.arg("request")?,
            c.arg("path")?,
            c.arg("text")?,
            c.arg("scale")?,
            c.sender("onEvent")?,
        )
        .await
    });
    d.add("dotnet_xaml_preview_update", |c| async move {
        Ok(dotnet_xaml_preview_update(c.arg("request")?, c.arg("text")?))
    });
    d.add("dotnet_test_projects", |c| async move { dotnet_test_projects(&c.host, &c.window).await });
    d.add("dotnet_tests", |c| async move {
        dotnet_tests(
            &c.host,
            &c.window,
            c.arg("request")?,
            c.arg("run")?,
            c.arg::<Option<bool>>("coverage")?.unwrap_or(false),
            c.arg::<Option<bool>>("debug")?.unwrap_or(false),
            c.arg("selection")?,
            c.sender("onEvent")?,
        )
        .await
    });
    d.add("nuget_search", |c| async move {
        nuget_search(&c.host, &c.window, c.arg("query")?, c.arg("prerelease")?, c.arg("skip")?, c.arg("take")?).await
    });
    d.add(
        "nuget_versions",
        |c| async move { nuget_versions(&c.host, &c.window, c.arg("id")?, c.arg("prerelease")?).await },
    );
    d.add("nuget_installed", |c| async move {
        nuget_installed(&c.host, &c.window, c.arg("outdated")?, c.arg("prerelease")?).await
    });
    d.add("dotnet_publish_profiles", |c| async move {
        dotnet_publish_profiles(&c.host, &c.window, c.arg("project")?).await
    });
    d.add("dotnet_publish_command", |c| async move {
        dotnet_publish_command(&c.host, &c.window, c.arg("project")?, c.arg("target")?).await
    });
    d.add("dotnet_container_run", |c| async move {
        dotnet_container_run(&c.host, &c.window, c.arg("project")?, c.arg("repository")?).await
    });
    d.add("dotnet_user_secrets", |c| async move { dotnet_user_secrets(&c.host, &c.window, c.arg("project")?).await });
    d.add("nuget_audit", |c| async move { nuget_audit(&c.host, &c.window).await });
    d.add("nuget_install", |c| async move {
        nuget_install(&c.host, &c.window, c.arg("projects")?, c.arg("id")?, c.arg("version")?).await
    });
    d.add("nuget_remove", |c| async move { nuget_remove(&c.host, &c.window, c.arg("projects")?, c.arg("id")?).await });
    d.add("dotnet_solutions", |c| async move { dotnet_solutions(&c.host, &c.window).await });
    d.add("dotnet_solution", |c| async move { dotnet_solution(&c.host, &c.window, c.arg("path")?).await });
}
