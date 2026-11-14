//! IL view (CI-10): "Show IL" for the C# type at the caret.
//!
//! In a file Roslyn wrote for a referenced assembly (Go to Definition into a library)
//! the assembly is the one its header names, or rather its implementation; in the
//! project's own code it is the project's built output. The IL is a read-only virtual
//! document, `dotnet-il://<assembly>/<type>.il?<target>`, that the .NET helper
//! disassembles when it is opened.
use crate::Host;
use crate::rpc::Dispatcher;
use ide_core::{CancellationToken, IdeError, IdeResult, IpcPath, Pos};
use ide_dotnet::helper::{self, Helper};
use ide_lsp::types::VirtualDocument;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const SCHEME: &str = "dotnet-il:";

/// What a `dotnet-il:` URI carries, as hex of its JSON.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Target {
    /// Tried in order, see [`ide_dotnet::il::implementation_candidates`].
    assemblies: Vec<PathBuf>,
    #[serde(rename = "type")]
    type_name: String,
    /// When the first assembly was written: a rebuilt project gets a new document
    /// rather than the tab and cached text of the previous build.
    stamp: u64,
}

fn uri_of(target: &Target) -> String {
    let json = serde_json::to_vec(target).unwrap_or_default();
    let hex: String = json.iter().map(|b| format!("{b:02x}")).collect();
    let assembly = target.assemblies.first().and_then(|p| p.file_name()).unwrap_or_default().to_string_lossy();
    format!("{SCHEME}//{}/{}.il?{hex}", escape(&assembly), escape(short_name(&target.type_name)))
}

fn target_of(uri: &str) -> IdeResult<Target> {
    let invalid = || IdeError::invalid("invalid IL document URI");
    let hex = uri.strip_prefix(SCHEME).and_then(|rest| rest.split_once('?')).map(|(_, hex)| hex).ok_or_else(invalid)?;
    if hex.len() > 32768 || hex.len() % 2 != 0 || !hex.is_ascii() {
        return Err(invalid());
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| invalid()))
        .collect::<IdeResult<Vec<_>>>()?;
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}

/// `Outer`1+Inner` of `Namespace.Outer`1+Inner`.
fn short_name(type_name: &str) -> &str {
    let outer = type_name.split('+').next().unwrap_or(type_name);
    outer.rfind('.').map_or(type_name, |dot| &type_name[dot + 1..])
}

/// A URI path segment: type names have `` ` ``, `+` and `<>` in them.
fn escape(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for b in segment.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The URI of the IL document for the C# type declared around `pos` in `path` (the first
/// type of the file without a position): open it as a virtual document.
pub async fn dotnet_il_uri(host: &Host, window: &str, path: IpcPath, pos: Option<Pos>) -> IdeResult<String> {
    let project = host.project(window)?;
    let syntax = host.syntax();
    let file = path.0;
    if !file.extension().is_some_and(|e| e.eq_ignore_ascii_case("cs")) {
        return Err(IdeError::invalid("IL can be shown for C# files only"));
    }
    let library = ide_workspace::documents::is_metadata_source(&file);
    let (type_name, referenced) = {
        let (project, file) = (project.clone(), file.clone());
        host.blocking(move || {
            let text = match project.documents.text_of(&file) {
                Some(text) => text,
                None => std::fs::read_to_string(&file)?,
            };
            let type_name = ide_analysis::csharp_types::metadata_name_at(&syntax, &text, pos)
                .ok_or_else(|| IdeError::not_found("this file declares no type"))?;
            Ok((type_name, ide_dotnet::il::metadata_source_assembly(&text)))
        })
        .await?
    };
    let settings = project.settings.effective();
    let dotnet = ide_sdk::apply::dotnet_command(&project.root, &settings);
    let assemblies = if library {
        let assembly = referenced.ok_or_else(|| {
            IdeError::not_found("this file does not name the assembly it comes from (it is the library's own source)")
        })?;
        let dotnet = dotnet.map_or_else(helper::dotnet, Ok).ok();
        host.blocking(move || {
            let root = dotnet.and_then(|d| dunce::canonicalize(d).ok()).and_then(|d| d.parent().map(Path::to_path_buf));
            Ok(ide_dotnet::il::implementation_candidates(&assembly, root.as_deref()))
        })
        .await?
    } else {
        // MSBuild evaluates the project to say where its output is.
        project.require_trust()?;
        let owner = host
            .blocking(move || {
                ide_dotnet::avalonia::project_of(&file)
                    .ok_or_else(|| IdeError::not_found("no project file above this file"))
            })
            .await?;
        let dotnet = dotnet.unwrap_or_else(|| PathBuf::from("dotnet"));
        vec![target_path(&dotnet, &ide_sdk::apply::dotnet_env(&settings), &owner).await?]
    };
    let stamp = assemblies
        .first()
        .and_then(|p| std::fs::metadata(p).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Ok(uri_of(&Target { assemblies, type_name, stamp }))
}

/// The built assembly of `project`: its `TargetPath` (first target framework when it has several).
async fn target_path(
    dotnet: &Path,
    env: &std::collections::BTreeMap<String, String>,
    project: &Path,
) -> IdeResult<PathBuf> {
    let mut args = vec!["msbuild".to_owned(), project.to_string_lossy().into_owned(), "-getProperty:TargetPath".into()];
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
    let name = project.file_name().unwrap_or_default().to_string_lossy();
    if !out.status.success() {
        let reason = stdout.lines().find(|l| l.contains("error")).unwrap_or("MSBuild failed").trim().to_owned();
        return Err(IdeError::unavailable(format!("cannot evaluate {name}: {reason}")));
    }
    let assembly = ide_dotnet::testing::parse_msbuild_properties(&stdout, &["TargetPath"])
        .into_iter()
        .find(|(k, _)| k == "TargetPath")
        .map(|(_, v)| PathBuf::from(v))
        .filter(|p| p.is_file())
        .ok_or_else(|| IdeError::not_found(format!("{name} has not been built: build it to see its IL")))?;
    Ok(assembly)
}

/// The last documents disassembled: the editor asks for one twice when it opens it
/// (its tab, then its buffer).
static RECENT: std::sync::Mutex<Vec<VirtualDocument>> = std::sync::Mutex::new(Vec::new());
const RECENT_MAX: usize = 4;

/// The IL document behind a `dotnet-il:` URI of [`dotnet_il_uri`].
pub async fn il_virtual_document(host: &Host, uri: &str) -> IdeResult<VirtualDocument> {
    let recent = || RECENT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = recent().iter().find(|d| d.uri == uri) {
        return Ok(hit.clone());
    }
    let target = target_of(uri)?;
    let token = CancellationToken::new();
    let helper = Helper::open(&host.local_data_dir.join("tools"), &token).await?;
    let il = helper.disassemble(&target.assemblies, &target.type_name, &token).await?;
    let title = format!("{}.il", short_name(&il.type_name));
    let document = VirtualDocument {
        uri: uri.to_owned(),
        language_id: host.languages.detect(Path::new(&title), None),
        text: il.text,
        title,
        description: Some(il.assembly.display().to_string()),
        decompiled: false,
        generated: false,
    };
    let mut recent = recent();
    recent.retain(|d| d.uri != uri);
    recent.truncate(RECENT_MAX - 1);
    recent.insert(0, document.clone());
    Ok(document)
}

pub(crate) fn register(d: &mut Dispatcher) {
    d.add("dotnet_il_uri", |c| async move { dotnet_il_uri(&c.host, &c.window, c.arg("path")?, c.arg("pos")?).await });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uri_carries_its_target_and_reads_as_assembly_and_type() {
        let target = Target {
            assemblies: vec![PathBuf::from("/dotnet/shared/Microsoft.NETCore.App/10.0.3/System.Collections.dll")],
            type_name: "System.Collections.Generic.List`1+Enumerator".into(),
            stamp: 42,
        };
        let uri = uri_of(&target);
        assert!(uri.starts_with("dotnet-il://System.Collections.dll/List%601%2BEnumerator.il?7b"), "{uri}");
        assert_eq!(target_of(&uri).unwrap(), target);
    }

    #[test]
    fn malformed_uris_are_rejected() {
        for uri in ["dotnet-il://a.dll/T.il", "dotnet-il://a.dll/T.il?zz", "dotnet-il://a.dll/T.il?7b7", "jdt://x?7b7d"]
        {
            assert_eq!(target_of(uri).unwrap_err().kind, ide_core::error::ErrorKind::InvalidInput, "{uri}");
        }
        // Valid hex and JSON, but not a target.
        assert!(target_of("dotnet-il://a.dll/T.il?7b7d").is_err());
    }

    #[test]
    fn short_names_keep_nested_types() {
        assert_eq!(short_name("System.Collections.Generic.List`1+Enumerator"), "List`1+Enumerator");
        assert_eq!(short_name("Program"), "Program");
        assert_eq!(short_name("A.B"), "B");
    }
}
