//! Code analysis over a scope of files: Inspect Code (batch LSP diagnostics),
//! Locate Duplicates and Search / Replace Structurally (`ide-analysis`).
//!
//! Scopes are Find in Files scopes; excluded roots are never analysed. Every
//! command takes a UI-allocated `RequestId` and is cancelled via `cancel_request`.
use crate::Host;
use crate::Project;
use crate::ops::Channel;
use crate::rpc::Dispatcher;

use super::doc::blocking;
use ide_analysis::csharp_types::{DeclaredType, MemberKind, declared_types};
use ide_analysis::signature::{MethodSignature, NewParameter, SignatureRewrite, SignatureSites};
use ide_analysis::{
    DuplicateOptions, DuplicatesReport, SourceFile, StructuralFile, StructuralQuery, StructuralSummary,
};
use ide_core::{
    CancellationToken, Diagnostic, DocId, IdeError, IdeResult, IpcPath, LanguageId, Location, Pos, Range, RequestId,
};
use ide_language::LanguageRegistry;
use ide_lsp::{FileInspection, InspectHost, InspectOptions};
use ide_search::{FileMask, SearchScope};
use ide_workspace::Documents;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

/// Files of `scope`, minus excluded roots and ignored names.
fn files_in_scope(project: &Project, scope: &SearchScope, cancel: &CancellationToken) -> IdeResult<Vec<SourceFile>> {
    let mask = FileMask::new(&[], &project.search_excludes())?;
    Ok(ide_search::scope_files(&project.root, scope, &mask, None, cancel)?
        .into_iter()
        .map(|f| SourceFile { path: f.path, rel_path: f.rel_path })
        .collect())
}

/// Current text of a file (the unsaved buffer when open) and its language.
fn load(project: &Project, languages: &LanguageRegistry, path: &Path) -> Option<(LanguageId, String)> {
    if let Some(doc) = project.documents.by_path(path) {
        let snap = project.documents.text_snapshot(doc).ok()?;
        return (!snap.large).then_some((snap.language, snap.text));
    }
    let loaded = project.documents.load_file(path).ok()?;
    if loaded.binary || loaded.large || loaded.too_large {
        return None;
    }
    let text = ide_text::normalize_line_endings(&loaded.text).into_owned();
    let language = languages.detect(path, text.lines().next());
    Some((language, text))
}

// ---- Inspect Code ----

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InspectedFile {
    pub path: IpcPath,
    pub rel_path: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InspectSummary {
    /// Files inspected by a language server.
    pub files: u32,
    pub problems: u32,
    /// Files in scope no language server serves.
    pub skipped: u32,
    /// Files whose server failed or never became ready.
    pub failed: u32,
    pub cancelled: bool,
    pub elapsed_ms: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InspectEvent {
    /// Files about to be inspected.
    Started {
        total: u32,
    },
    /// A file finished (with or without problems).
    File {
        file: InspectedFile,
        done: u32,
    },
    Done {
        summary: InspectSummary,
    },
}

struct DocumentsHost(Arc<Documents>);

impl InspectHost for DocumentsHost {
    fn open(&self, path: &Path) -> IdeResult<(DocId, bool)> {
        let was_open = self.0.by_path(path).is_some();
        Ok((self.0.open(path)?.id, was_open))
    }

    fn close(&self, doc: DocId) {
        self.0.close(doc);
    }
}

// ---- Locate Duplicates ----

// ---- Structural Search ----

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StructuralEvent {
    File { file: StructuralFile },
    Done { summary: StructuralSummary },
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StructuralLanguage {
    pub id: LanguageId,
    pub name: String,
}

/// Inspect Code: diagnostics of every file in `scope` that a language server
/// serves. Files stream through `on_event` as they finish; ends with `Done`.
pub async fn inspect_code(
    host: &Host,
    window: &str,
    request: RequestId,
    scope: SearchScope,
    on_event: Channel<InspectEvent>,
) -> IdeResult<()> {
    let started = Instant::now();
    let project = host.project(window)?;
    let cancel = host.begin_request(request);
    let listed = {
        let (project, cancel) = (project.clone(), cancel.clone());
        blocking(move || files_in_scope(&project, &scope, &cancel)).await
    };
    let files = match listed {
        Ok(files) => files,
        Err(e) => {
            host.end_request(request);
            return Err(e);
        }
    };
    let total_in_scope = files.len() as u32;
    let rel: std::collections::HashMap<_, _> = files
        .iter()
        .filter(|f| project.lsp.serves_path(&f.path))
        .map(|f| (f.path.clone(), f.rel_path.clone()))
        .collect();
    let mut summary = InspectSummary { skipped: total_in_scope - rel.len() as u32, ..Default::default() };
    let _ = on_event.send(InspectEvent::Started { total: rel.len() as u32 });

    let mut paths: Vec<_> = rel.keys().cloned().collect();
    paths.sort();
    let tally = Mutex::new(summary.clone());
    let done = std::sync::atomic::AtomicU32::new(0);
    let docs = Arc::new(DocumentsHost(project.documents.clone()));
    project
        .lsp
        .inspect_files(docs, paths, InspectOptions::default(), cancel.clone(), &|path, outcome| {
            let done = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let diagnostics = {
                let mut t = tally.lock();
                match outcome {
                    FileInspection::Inspected(items) => {
                        t.files += 1;
                        t.problems += items.len() as u32;
                        items
                    }
                    FileInspection::NoServer => {
                        t.skipped += 1;
                        Vec::new()
                    }
                    FileInspection::Failed(message) => {
                        tracing::debug!("inspect {}: {message}", path.display());
                        t.failed += 1;
                        Vec::new()
                    }
                }
            };
            let rel_path = rel.get(&path).cloned().unwrap_or_default();
            let file = InspectedFile { path: IpcPath(path), rel_path, diagnostics };
            let _ = on_event.send(InspectEvent::File { file, done });
        })
        .await;
    host.end_request(request);
    summary = tally.into_inner();
    summary.cancelled = cancel.is_cancelled();
    summary.elapsed_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
    let _ = on_event.send(InspectEvent::Done { summary });
    Ok(())
}

/// Locate Duplicates over the files of `scope`.
pub async fn analysis_locate_duplicates(
    host: &Host,
    window: &str,
    request: RequestId,
    scope: SearchScope,
    options: DuplicateOptions,
) -> IdeResult<DuplicatesReport> {
    let project = host.project(window)?;
    let cancel = host.begin_request(request);
    let languages = project.languages.clone();
    let syntax = host.syntax();
    let result = blocking(move || {
        let files = files_in_scope(&project, &scope, &cancel)?;
        let loader = |p: &Path| load(&project, &languages, p);
        Ok(ide_analysis::find_duplicates(&syntax, &files, &loader, &options, &cancel))
    })
    .await;
    host.end_request(request);
    result
}

/// Languages Search Structurally supports (those with a bundled grammar).
pub async fn structural_languages(host: &Host, _window: &str) -> IdeResult<Vec<StructuralLanguage>> {
    let syntax = host.syntax();
    let mut out: Vec<StructuralLanguage> = host
        .languages
        .all()
        .into_iter()
        .filter(|c| syntax.supports(&c.id))
        .map(|c| StructuralLanguage { id: c.id, name: c.name })
        .collect();
    out.sort_by_key(|l| l.name.to_lowercase());
    Ok(out)
}

/// Checks a structural search template (and language) while the user types.
pub async fn structural_validate(host: &Host, _window: &str, query: StructuralQuery) -> IdeResult<()> {
    let syntax = host.syntax();
    blocking(move || ide_analysis::validate_structural(&syntax, &query)).await
}

/// Search Structurally over `scope`; files with matches stream through
/// `on_event`, ending with `Done`. With a replacement, matches carry their
/// replacement text (applied by the UI as a workspace edit).
pub async fn structural_search(
    host: &Host,
    window: &str,
    request: RequestId,
    scope: SearchScope,
    query: StructuralQuery,
    on_event: Channel<StructuralEvent>,
) -> IdeResult<()> {
    let project = host.project(window)?;
    let cancel = host.begin_request(request);
    let languages = project.languages.clone();
    let syntax = host.syntax();
    let events = on_event.clone();
    let result = blocking(move || {
        let files = files_in_scope(&project, &scope, &cancel)?;
        let loader = |p: &Path| load(&project, &languages, p);
        ide_analysis::search_structurally(&syntax, &files, &loader, &query, &cancel, &|file| {
            let _ = events.send(StructuralEvent::File { file });
        })
    })
    .await;
    host.end_request(request);
    let _ = on_event.send(StructuralEvent::Done { summary: result? });
    Ok(())
}

// ---- Change Signature (C#) ----

/// The C# method, constructor, local function or delegate declared around `pos` in `path`.
pub async fn signature_at(host: &Host, window: &str, path: IpcPath, pos: Pos) -> IdeResult<Option<MethodSignature>> {
    let project = host.project(window)?;
    let languages = project.languages.clone();
    let syntax = host.syntax();
    blocking(move || {
        let (_, text) = load(&project, &languages, &path).ok_or_else(|| IdeError::not_found("cannot read the file"))?;
        Ok(ide_analysis::signature::signature_at(&syntax, &text, pos))
    })
    .await
}

/// Rewrites `signature`'s declaration and the calls at `locations` (the
/// language server's references, declaration included) for `change`; the UI
/// applies the edit. Files outside the project are not touched.
pub async fn change_signature(
    host: &Host,
    window: &str,
    signature: MethodSignature,
    change: Vec<NewParameter>,
    locations: Vec<Location>,
) -> IdeResult<SignatureRewrite> {
    let project = host.project(window)?;
    let languages = project.languages.clone();
    let syntax = host.syntax();
    blocking(move || {
        let mut by_file: Vec<(IpcPath, Vec<Range>)> = Vec::new();
        for l in locations.into_iter().filter(|l| l.uri.is_none() && l.path.starts_with(&project.root)) {
            match by_file.iter_mut().find(|(p, _)| *p == l.path) {
                Some((_, ranges)) => ranges.push(l.range),
                None => by_file.push((l.path, vec![l.range])),
            }
        }
        let files: Vec<SignatureSites> = by_file
            .into_iter()
            .filter_map(|(path, locations)| {
                let (_, text) = load(&project, &languages, &path)?;
                Some(SignatureSites { path, text, locations })
            })
            .collect();
        ide_analysis::signature::change_signature(&syntax, &signature, &change, &files).map_err(IdeError::invalid)
    })
    .await
}

// ---- C# types for XAML (CI-16) ----

/// A member a XAML binding can name, with where it is declared.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BindableMember {
    pub name: String,
    pub type_name: Option<String>,
    pub kind: MemberKind,
    pub path: IpcPath,
    pub range: Range,
    /// The type that declares it (a base type for inherited members).
    pub declared_in: String,
}

/// A C# type of the project: where it is declared and its members, inherited
/// ones (from base types in the project) after its own.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BindableType {
    pub name: String,
    pub namespace: String,
    pub path: IpcPath,
    pub range: Range,
    pub members: Vec<BindableMember>,
}

/// The parts (partial declarations) of the project's type `name`, in `namespace`
/// when given, else preferably in `near`, else the first namespace found.
fn type_parts(
    project: &Project,
    languages: &LanguageRegistry,
    syntax: &ide_language::Syntax,
    name: &str,
    namespace: Option<&str>,
    near: Option<&str>,
) -> Vec<(IpcPath, DeclaredType)> {
    let mut paths: Vec<IpcPath> = Vec::new();
    for s in project.index.symbols_named(name, None, 64) {
        if s.path.extension().is_some_and(|e| e == "cs") && !paths.contains(&s.path) {
            paths.push(s.path);
        }
    }
    // Partial declarations in a stable order, whatever order the index has.
    paths.sort();
    let mut parts: Vec<(IpcPath, DeclaredType)> = Vec::new();
    for path in paths {
        let Some((_, text)) = load(project, languages, &path) else { continue };
        for t in declared_types(syntax, &text) {
            if t.name == name {
                parts.push((path.clone(), t));
            }
        }
    }
    let chosen = namespace
        .or_else(|| near.filter(|n| parts.iter().any(|(_, t)| t.namespace == *n)))
        .map(str::to_string)
        .or_else(|| parts.first().map(|(_, t)| t.namespace.clone()));
    parts.retain(|(_, t)| Some(&t.namespace) == chosen.as_ref());
    parts
}

/// The project's C# type `name` (simple name, in `namespace` when given) and the
/// members a binding can name; `None` when the project declares no such type.
pub async fn csharp_type_members(
    host: &Host,
    window: &str,
    name: String,
    namespace: Option<String>,
) -> IdeResult<Option<BindableType>> {
    let project = host.project(window)?;
    let languages = project.languages.clone();
    let syntax = host.syntax();
    blocking(move || {
        let parts = type_parts(&project, &languages, &syntax, &name, namespace.as_deref(), None);
        let Some((path, first)) = parts.first().cloned() else { return Ok(None) };
        let mut out = BindableType {
            name: first.name.clone(),
            namespace: first.namespace.clone(),
            path,
            range: first.name_range,
            members: Vec::new(),
        };
        // Own members, then each base type's, breadth first; names already seen hide inherited ones.
        let mut queue = std::collections::VecDeque::from([(parts, 0usize)]);
        let mut visited = std::collections::HashSet::new();
        while let Some((parts, depth)) = queue.pop_front() {
            let Some((_, head)) = parts.first() else { continue };
            if !visited.insert(format!("{}.{}", head.namespace, head.name)) {
                continue;
            }
            let near = head.namespace.clone();
            let mut bases = Vec::new();
            for (path, t) in &parts {
                for m in &t.members {
                    if !out.members.iter().any(|o| o.name == m.name) {
                        out.members.push(BindableMember {
                            name: m.name.clone(),
                            type_name: m.type_name.clone(),
                            kind: m.kind.clone(),
                            path: path.clone(),
                            range: m.range,
                            declared_in: t.name.clone(),
                        });
                    }
                }
                bases.extend(t.bases.iter().cloned());
            }
            if depth < 4 {
                for base in bases {
                    let simple = base.rsplit('.').next().unwrap_or(&base).to_string();
                    let found = type_parts(&project, &languages, &syntax, &simple, None, Some(&near));
                    if !found.is_empty() {
                        queue.push_back((found, depth + 1));
                    }
                }
            }
        }
        Ok(Some(out))
    })
    .await
}

/// Commands of this module the remote server answers.
pub(crate) fn register(d: &mut Dispatcher) {
    d.add("inspect_code", |c| async move {
        inspect_code(&c.host, &c.window, c.arg("request")?, c.arg("scope")?, c.sender("onEvent")?).await
    });
    d.add("analysis_locate_duplicates", |c| async move {
        analysis_locate_duplicates(&c.host, &c.window, c.arg("request")?, c.arg("scope")?, c.arg("options")?).await
    });
    d.add("csharp_type_members", |c| async move {
        csharp_type_members(&c.host, &c.window, c.arg("name")?, c.arg("namespace")?).await
    });
    d.add("signature_at", |c| async move { signature_at(&c.host, &c.window, c.arg("path")?, c.arg("pos")?).await });
    d.add("change_signature", |c| async move {
        change_signature(&c.host, &c.window, c.arg("signature")?, c.arg("change")?, c.arg("locations")?).await
    });
    d.add("structural_languages", |c| async move { structural_languages(&c.host, &c.window).await });
    d.add("structural_validate", |c| async move { structural_validate(&c.host, &c.window, c.arg("query")?).await });
    d.add("structural_search", |c| async move {
        structural_search(&c.host, &c.window, c.arg("request")?, c.arg("scope")?, c.arg("query")?, c.sender("onEvent")?)
            .await
    });
}
