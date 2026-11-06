//! Pull diagnostics (`textDocument/diagnostic`). Servers that provide them may stop pushing
//! their own analysis to a client that can pull (rust-analyzer pushes only `cargo check`
//! then): the editor pulls an open file's diagnostics when it opens, after edits and saves,
//! and when the server asks for a refresh. Pulled diagnostics are kept apart from pushed ones
//! and shown together.
//!
//! Bayonet: `pull-diagnostics` in a language's TOML pulls from a server that did not
//! advertise `diagnosticProvider`, and `workspace-diagnostics` (Roslyn) also pulls the whole
//! workspace (`workspace/diagnostic`) so problems in files that are not open are listed. Those
//! are kept as the server's pushed diagnostics: a file's own pull replaces them while it is
//! open, and they are kept when it closes.

use crate::server::{Running, Server};
use crate::uri::uri_to_path;
use crate::{LspManager, convert, virtual_doc};
use ide_core::{DocId, IdeEvent};
use ide_jsonrpc::RpcError;
use lsp_types as lsp;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// How long typing must pause before a changed file is pulled again.
pub(crate) const AFTER_EDIT: Duration = Duration::from_millis(300);
const TIMEOUT: Duration = Duration::from_secs(30);
/// Retries of a pull the server cancelled (`ServerCancelled`, e.g. while indexing).
const RETRIES: u32 = 20;

struct PullTarget {
    server: Arc<Server>,
    run: Running,
    path: PathBuf,
    uri: lsp::Uri,
}

fn provides_pull(server: &Server, run: &Running) -> bool {
    let caps = run.caps.read();
    caps.restriction.diagnostics() && (caps.has("diagnosticProvider") || server.effective_config().pull_diagnostics)
}

fn inter_file(run: &Running) -> bool {
    run.caps.read().raw.pointer("/diagnosticProvider/interFileDependencies").and_then(Value::as_bool) == Some(true)
}

impl LspManager {
    /// Pulls `doc`'s diagnostics from its servers that provide them (`only`: from that one)
    /// after `delay`; a later pull of the same file supersedes it.
    pub(crate) fn schedule_pull(&self, doc: DocId, only: Option<&Server>, delay: Duration) {
        let targets: Vec<PullTarget> = {
            let docs = self.docs.lock();
            let Some(d) = docs.get(&doc) else { return };
            // Decompiled C# is a source view, not compilable project code (e.g. Unity's older language version).
            if virtual_doc::is_csharp_decompiled(d.uri.as_str()) {
                return;
            }
            d.servers
                .iter()
                .filter(|a| only.is_none_or(|s| s.id == a.server.id))
                .filter_map(|a| {
                    let run = a.server.running()?;
                    (a.opened_gen == Some(run.generation) && provides_pull(&a.server, &run)).then(|| PullTarget {
                        server: a.server.clone(),
                        run,
                        path: d.path.clone(),
                        uri: d.uri.clone(),
                    })
                })
                .collect()
        };
        for t in targets {
            if t.server.suppresses_diagnostics(&t.path) {
                continue;
            }
            let pull = t.server.next_pull(&t.path);
            let me = self.me.clone();
            self.runtime.spawn(async move {
                tokio::time::sleep(delay).await;
                if let Some(me) = me.upgrade() {
                    me.pull(t, pull).await;
                }
            });
        }
    }

    /// Pulls every open file of `server` again (`workspace/diagnostic/refresh`).
    pub(crate) fn refresh_pulled_diagnostics(&self, server: &Server) {
        let docs: Vec<DocId> = self
            .docs
            .lock()
            .iter()
            .filter(|(_, d)| d.servers.iter().any(|a| a.server.id == server.id))
            .map(|(id, _)| *id)
            .collect();
        for doc in docs {
            self.schedule_pull(doc, Some(server), Duration::ZERO);
        }
    }

    /// After `doc` was saved: servers whose diagnostics depend on other files pull those
    /// open files too.
    pub(crate) fn pull_dependents(&self, doc: DocId) {
        let docs = self.docs.lock();
        let Some(d) = docs.get(&doc) else { return };
        let servers: Vec<Arc<Server>> = d
            .servers
            .iter()
            .filter(|a| a.server.running().is_some_and(|run| provides_pull(&a.server, &run) && inter_file(&run)))
            .map(|a| a.server.clone())
            .collect();
        let others: Vec<(DocId, Vec<Arc<Server>>)> = docs
            .iter()
            .filter(|(id, _)| **id != doc)
            .map(|(id, o)| {
                (*id, servers.iter().filter(|s| o.servers.iter().any(|a| a.server.id == s.id)).cloned().collect())
            })
            .collect();
        drop(docs);
        for (other, servers) in others {
            for server in servers {
                self.schedule_pull(other, Some(&server), Duration::ZERO);
            }
        }
    }

    /// Drops what was pulled for a closed file and shows what remains.
    pub(crate) fn forget_pulled(&self, server: &Server, path: &std::path::Path, run: Option<&Running>) {
        // A workspace-pulling server reports a closed file again only once it changes: what
        // the file's own pull found stays, as the workspace pull's.
        let keep = server.effective_config().workspace_diagnostics.then(|| server.diagnostics_for(path));
        if let Some(mut shown) = server.clear_pulled(path) {
            if let Some(keep) = keep {
                shown = server.set_diagnostics(path.to_owned(), keep);
            }
            let enc = run.map(|r| r.caps.read().encoding).unwrap_or_default();
            self.emit_diagnostics(server, path.to_owned(), shown, enc);
        }
    }

    async fn pull(&self, t: PullTarget, pull: u64) {
        let mut attempt = 0;
        loop {
            if !t.server.is_latest_pull(&t.path, pull) {
                return;
            }
            let mut params = json!({ "textDocument": { "uri": t.uri } });
            if let Some(id) = t.server.pulled_result_id(&t.path) {
                params["previousResultId"] = json!(id);
            }
            if let Some(identifier) = t.run.caps.read().raw.pointer("/diagnosticProvider/identifier") {
                params["identifier"] = identifier.clone();
            }
            let request = t.run.peer.request::<Value, Value>("textDocument/diagnostic", params, None);
            let report = match tokio::time::timeout(TIMEOUT, request).await {
                Ok(Ok(report)) => report,
                // This request has no cancellation of its own: the server cancelled it
                // (`ServerCancelled` while busy, which asks to be asked again).
                Ok(Err(RpcError::Cancelled)) if attempt < RETRIES => {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt.min(8)))).await;
                    continue;
                }
                Ok(Err(e)) => {
                    tracing::debug!("{}: textDocument/diagnostic: {e}", t.server.name);
                    return;
                }
                Err(_) => {
                    tracing::debug!("{}: textDocument/diagnostic timed out", t.server.name);
                    return;
                }
            };
            // A newer pull, a closed file or a restarted server makes this report stale.
            if !t.server.is_latest_pull(&t.path, pull)
                || t.server.running().is_none_or(|r| r.generation != t.run.generation)
                || !self.docs.lock().values().any(|d| d.path == t.path)
            {
                return;
            }
            let result_id = report.get("resultId").and_then(Value::as_str).map(str::to_owned);
            let items: Option<Vec<lsp::Diagnostic>> = match report.get("kind").and_then(Value::as_str) {
                Some("unchanged") => None,
                _ => match serde_json::from_value(report.get("items").cloned().unwrap_or(json!([]))) {
                    Ok(items) => Some(items),
                    Err(e) => {
                        tracing::debug!("{}: bad textDocument/diagnostic report: {e}", t.server.name);
                        return;
                    }
                },
            };
            if let Some(items) = &items {
                // perf/compare times the open file's diagnostics by this line.
                tracing::debug!("{}: {} diagnostics for {}", t.server.name, items.len(), t.path.display());
            }
            if t.server.effective_config().workspace_diagnostics {
                // The file's own report supersedes what the workspace pull last said about it.
                t.server.set_diagnostics(t.path.clone(), Vec::new());
            }
            let shown = t.server.set_pulled(t.path.clone(), result_id, items);
            let enc = t.run.caps.read().encoding;
            self.emit_diagnostics(&t.server, t.path, shown, enc);
            return;
        }
    }

    /// Shows `items` as `server`'s diagnostics for `path`.
    pub(crate) fn emit_diagnostics(
        &self,
        server: &Server,
        path: PathBuf,
        items: Vec<lsp::Diagnostic>,
        enc: crate::text::Encoding,
    ) {
        let conv = self.conv(enc);
        let items = items.into_iter().map(|d| convert::diagnostic(&conv, &path, d, &server.name)).collect();
        self.sink.emit(IdeEvent::Diagnostics { path: path.into(), source: server.name.clone(), items });
    }
}

/// Wait after the first refresh before the workspace pull starts: loading a solution sends several.
const WORKSPACE_DEBOUNCE: Duration = Duration::from_secs(2);
/// Roslyn holds a workspace pull open until something changes; past this the pull is sent again.
const WORKSPACE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Failed pulls in a row (the server cancelled, or is restarting) before the loop stops.
const WORKSPACE_RETRIES: u32 = 5;

/// A server's running workspace pull: the instance it is for and the result ids it last got.
pub(crate) struct WorkspacePull {
    generation: u64,
    result_ids: HashMap<String, Vec<String>>,
}

impl LspManager {
    /// Starts pulling `server`'s whole-workspace diagnostics (`workspace-diagnostics` in the
    /// language's TOML) unless that already runs for this server instance. Problems in files
    /// that are not open are published like a `publishDiagnostics`.
    pub(crate) fn schedule_workspace_pull(&self, server: &Arc<Server>) {
        if !server.effective_config().workspace_diagnostics {
            return;
        }
        let Some(run) = server.running() else { return };
        {
            let mut pulls = self.workspace_pulls.lock();
            if pulls.get(&server.id).is_some_and(|w| w.generation == run.generation) {
                return;
            }
            pulls.insert(server.id, WorkspacePull { generation: run.generation, result_ids: HashMap::new() });
        }
        let weak = self.me.clone();
        let server = server.clone();
        self.runtime.spawn(async move {
            tokio::time::sleep(WORKSPACE_DEBOUNCE).await;
            if let Some(me) = weak.upgrade() {
                me.workspace_loop(server, run.generation).await;
            }
        });
    }

    fn workspace_pull_current(&self, server: &Server, generation: u64) -> bool {
        self.workspace_pulls.lock().get(&server.id).is_some_and(|w| w.generation == generation)
    }

    fn stop_workspace_pull(&self, server: &Server, generation: u64) {
        let mut pulls = self.workspace_pulls.lock();
        if pulls.get(&server.id).is_some_and(|w| w.generation == generation) {
            pulls.remove(&server.id);
        }
    }

    /// One pull at a time, as the server long-polls: each answer carries what changed since
    /// the result ids sent with it, and the next pull is sent straight away.
    async fn workspace_loop(self: Arc<Self>, server: Arc<Server>, generation: u64) {
        let mut failures = 0;
        loop {
            if self.shutting_down.load(Ordering::Acquire) || !self.workspace_pull_current(&server, generation) {
                return;
            }
            let Some(t) = self.ready_targets(std::slice::from_ref(&server), |_| true).await.pop() else {
                self.stop_workspace_pull(&server, generation);
                return;
            };
            if t.run.generation != generation {
                // A restarted server: its own refresh starts a new loop.
                self.stop_workspace_pull(&server, generation);
                return;
            }
            let previous: Vec<serde_json::Value> = self
                .workspace_pulls
                .lock()
                .get(&server.id)
                .map(|w| {
                    w.result_ids
                        .iter()
                        .flat_map(|(uri, ids)| ids.iter().map(move |id| serde_json::json!({ "uri": uri, "value": id })))
                        .collect()
                })
                .unwrap_or_default();
            let params = serde_json::json!({ "previousResultIds": previous });
            let report: serde_json::Value =
                match self.call(&t, "workspace/diagnostic", params, None, WORKSPACE_TIMEOUT).await {
                    Ok(r) => {
                        failures = 0;
                        r
                    }
                    Err(e) => {
                        failures += 1;
                        tracing::debug!("{}: workspace diagnostics: {e}", server.name);
                        if failures >= WORKSPACE_RETRIES {
                            self.stop_workspace_pull(&server, generation);
                            return;
                        }
                        tokio::time::sleep(Duration::from_secs(failures.into())).await;
                        continue;
                    }
                };
            if !self.workspace_pull_current(&server, generation) {
                return;
            }
            self.publish_workspace_report(&server, &t, generation, &report);
        }
    }

    fn publish_workspace_report(
        &self,
        server: &Server,
        t: &crate::Target,
        generation: u64,
        report: &serde_json::Value,
    ) {
        let found = workspace_report(report, |p| server.suppresses_diagnostics(p));
        if let Some(w) = self.workspace_pulls.lock().get_mut(&server.id).filter(|w| w.generation == generation) {
            for (uri, ids) in found.result_ids {
                w.result_ids.insert(uri, ids);
            }
        }
        // Open files are the document pull's; the server leaves them out anyway.
        let open: HashSet<PathBuf> = self.docs.lock().values().map(|d| d.path.clone()).collect();
        let conv = self.conv(t.enc());
        let mut with_problems = 0;
        for (path, items) in found.files {
            if open.contains(&path) {
                continue;
            }
            with_problems += usize::from(!items.is_empty());
            let shown = server.set_diagnostics(path.clone(), items);
            let converted: Vec<_> =
                shown.into_iter().map(|d| convert::diagnostic(&conv, &path, d, &server.name)).collect();
            self.sink.emit(IdeEvent::Diagnostics { path: path.into(), source: server.name.clone(), items: converted });
        }
        tracing::debug!("{}: workspace diagnostics: {with_problems} changed files with problems", server.name);
    }
}

/// A `workspace/diagnostic` answer: the full reports by file, and the result ids by URI.
#[derive(Default)]
struct WorkspaceReport {
    /// Files with a new report (an empty one clears the file).
    files: HashMap<PathBuf, Vec<lsp_types::Diagnostic>>,
    result_ids: HashMap<String, Vec<String>>,
}

/// Reads a `workspace/diagnostic` answer. A server may report a file several times (once per
/// target framework of a multi-targeting project): those reports are merged, without
/// duplicates. Unchanged reports keep only their result ids; `skip` drops files.
fn workspace_report(report: &serde_json::Value, skip: impl Fn(&Path) -> bool) -> WorkspaceReport {
    let mut out = WorkspaceReport::default();
    for item in report.get("items").and_then(serde_json::Value::as_array).into_iter().flatten() {
        let Some(uri) = item.get("uri").and_then(serde_json::Value::as_str) else { continue };
        if let Some(id) = item.get("resultId").and_then(serde_json::Value::as_str) {
            out.result_ids.entry(uri.to_owned()).or_default().push(id.to_owned());
        }
        if item.get("kind").and_then(serde_json::Value::as_str) != Some("full") {
            continue;
        }
        let Some(path) = uri_to_path(uri) else { continue };
        if skip(&path) {
            continue;
        }
        let items: Vec<lsp_types::Diagnostic> =
            item.get("items").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        let file = out.files.entry(path).or_default();
        for d in items {
            if !file.iter().any(|e| e.range == d.range && e.code == d.code && e.message == d.message) {
                file.push(d);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_report_merges_reports_per_file() {
        let report = serde_json::json!({ "items": [
            { "kind": "full", "uri": "file:///s/A.cs", "resultId": "1", "items": [
                { "range": { "start": { "line": 1, "character": 2 }, "end": { "line": 1, "character": 5 } }, "message": "CS0103" }
            ]},
            { "kind": "full", "uri": "file:///s/A.cs", "resultId": "2", "items": [
                { "range": { "start": { "line": 3, "character": 0 }, "end": { "line": 3, "character": 1 } }, "message": "IDE0005" },
                { "range": { "start": { "line": 1, "character": 2 }, "end": { "line": 1, "character": 5 } }, "message": "CS0103" }
            ]},
            { "kind": "unchanged", "uri": "file:///s/B.cs", "resultId": "3" },
            { "kind": "full", "uri": "file:///s/Gen.cs", "items": [] },
            { "kind": "full", "uri": "file:///s/C.cs", "resultId": "4", "items": [] }
        ]});
        let out = workspace_report(&report, |p| p.ends_with("Gen.cs"));
        assert_eq!(out.files[Path::new("/s/A.cs")].len(), 2);
        assert!(out.files[Path::new("/s/C.cs")].is_empty());
        assert!(!out.files.contains_key(Path::new("/s/B.cs")));
        assert!(!out.files.contains_key(Path::new("/s/Gen.cs")));
        assert_eq!(out.result_ids["file:///s/A.cs"], ["1", "2"]);
        assert_eq!(out.result_ids["file:///s/B.cs"], ["3"]);
    }
}
