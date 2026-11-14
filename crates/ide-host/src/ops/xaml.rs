//! What the XAML of a project can name (CI-16): the elements, attributes and attribute
//! values of its UI framework (WPF, Avalonia, MAUI, WinUI) and of its own built types,
//! read from the assemblies MSBuild resolves for it by the .NET helper.

use crate::Host;
use crate::rpc::Dispatcher;
use ide_core::{CancellationToken, IdeError, IdeResult, IpcPath};
use ide_dotnet::helper::{self, Helper, XamlTypes};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// The project, and when its restore and its build last changed: the types stay the same until then.
type Key = (PathBuf, Option<SystemTime>, Option<SystemTime>);

static CACHE: Mutex<Vec<(Key, Arc<XamlTypes>)>> = Mutex::new(Vec::new());
const CACHE_MAX: usize = 4;

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// The types the XAML file at `path` can name, from the project above it.
pub async fn dotnet_xaml_types(host: &Host, window: &str, path: IpcPath) -> IdeResult<XamlTypes> {
    let project = host.project(window)?;
    project.require_trust()?;
    let file = path.0;
    let owner = host
        .blocking(move || {
            ide_dotnet::avalonia::project_of(&file)
                .ok_or_else(|| IdeError::not_found("no project file above this file"))
        })
        .await?;
    let assets = owner.parent().map(|d| d.join("obj/project.assets.json"));
    let settings = project.settings.effective();
    let dotnet = ide_sdk::apply::dotnet_command(&project.root, &settings).map_or_else(helper::dotnet, Ok)?;
    let env = ide_sdk::apply::dotnet_env(&settings);

    let (references, target) = references(&dotnet, &env, &owner).await?;
    let key: Key = (owner.clone(), assets.as_deref().and_then(modified), target.as_deref().and_then(modified));
    let cached = || CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, hit)) = cached().iter().find(|(k, _)| *k == key) {
        return Ok(XamlTypes::clone(hit));
    }
    let token = CancellationToken::new();
    let helper = Helper::open(&host.local_data_dir.join("tools"), &token).await?;
    let types = Arc::new(helper.xaml_types(&references, target.as_deref(), &token).await?);
    let mut cache = cached();
    cache.retain(|(k, _)| k.0 != key.0);
    cache.truncate(CACHE_MAX - 1);
    cache.insert(0, (key, types.clone()));
    Ok(XamlTypes::clone(&types))
}

/// The assemblies `project` compiles against, and its own when it has been built.
async fn references(
    dotnet: &Path,
    env: &std::collections::BTreeMap<String, String>,
    project: &Path,
) -> IdeResult<(Vec<PathBuf>, Option<PathBuf>)> {
    let mut args = vec![
        "msbuild".to_owned(),
        project.to_string_lossy().into_owned(),
        "-nologo".into(),
        "-t:ResolveReferences".into(),
        "-getItem:ReferencePath".into(),
        "-getProperty:TargetPath".into(),
    ];
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
    let out = tokio::process::Command::from(ide_dotnet::sdk_command(dotnet, env))
        .args(&args)
        .current_dir(project.parent().unwrap_or(project))
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", dotnet.display())))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let references = ide_dotnet::generators::parse_items(&stdout, "ReferencePath");
    if references.is_empty() {
        let name = project.file_name().unwrap_or_default().to_string_lossy();
        let reason = stdout.lines().find(|l| l.contains("error")).unwrap_or("no references resolved").trim().to_owned();
        return Err(IdeError::unavailable(format!("cannot resolve the references of {name}: {reason}")));
    }
    let target = ide_dotnet::generators::parse_properties(&stdout)
        .into_iter()
        .find(|(k, _)| k == "TargetPath")
        .map(|(_, v)| PathBuf::from(v))
        .filter(|p| p.is_file());
    Ok((references, target))
}

pub(crate) fn register(d: &mut Dispatcher) {
    d.add("dotnet_xaml_types", |c| async move { dotnet_xaml_types(&c.host, &c.window, c.arg("path")?).await });
}
