//! Live diagnostics for servers that only answer `textDocument/diagnostic` and never
//! push (`pull-diagnostics` in the language's TOML: Roslyn). After a document opens or
//! changes, and when the server sends `workspace/diagnostic/refresh` (it finished
//! loading projects, a referenced file changed), each open document is pulled again
//! and the result published like a `publishDiagnostics`. Pulls are debounced per
//! document, and a newer edit drops the result of one still in flight.

use crate::LspManager;
use crate::convert;
use crate::server::Server;
use crate::uri::uri_to_path;
use ide_core::{CancellationToken, DocId, IdeEvent};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Wait after the last edit before pulling: typing would otherwise pull per keystroke.
const DEBOUNCE: Duration = Duration::from_millis(250);
/// A server still loading the solution answers late; past this, the next change retries.
const PULL_TIMEOUT: Duration = Duration::from_secs(60);

impl LspManager {
    /// Pulls `doc`'s diagnostics from its pull-only servers after [`DEBOUNCE`].
    pub(crate) fn schedule_pull(&self, doc: DocId) {
        let wanted = {
            let docs = self.docs.lock();
            docs.get(&doc).is_some_and(|d| d.servers.iter().any(|a| a.server.effective_config().pull_diagnostics))
        };
        if !wanted {
            return;
        }
        let generation = {
            let mut pulls = self.pulls.lock();
            let g = pulls.entry(doc).or_insert(0);
            *g += 1;
            *g
        };
        let weak = self.me.clone();
        self.runtime.spawn(async move {
            tokio::time::sleep(DEBOUNCE).await;
            if let Some(me) = weak.upgrade() {
                me.pull(doc, generation).await;
            }
        });
    }

    /// Re-pulls every open document `server` serves (`workspace/diagnostic/refresh`).
    pub(crate) fn schedule_pull_all(&self, server: &Server) {
        if !server.effective_config().pull_diagnostics {
            return;
        }
        let docs: Vec<DocId> = self
            .docs
            .lock()
            .iter()
            .filter(|(_, d)| d.servers.iter().any(|a| a.server.id == server.id))
            .map(|(id, _)| *id)
            .collect();
        for doc in docs {
            self.schedule_pull(doc);
        }
    }

    pub(crate) fn forget_pulls(&self, doc: DocId) {
        self.pulls.lock().remove(&doc);
    }

    fn pull_is_current(&self, doc: DocId, generation: u64) -> bool {
        self.pulls.lock().get(&doc) == Some(&generation)
    }

    async fn pull(self: Arc<Self>, doc: DocId, generation: u64) {
        if !self.pull_is_current(doc, generation) {
            return;
        }
        let Ok(snap) = self.snapshot(doc) else { return };
        let servers: Vec<Arc<Server>> =
            snap.servers.iter().filter(|s| s.effective_config().pull_diagnostics).cloned().collect();
        // The TOML flag is what says the server can pull (`initialize_params` declares pull
        // support to it), not whether it advertised `diagnosticProvider`.
        let targets = self.ready_targets(&servers, |_| true).await;
        for t in targets {
            if self.shutting_down.load(Ordering::Acquire) || t.server.suppresses_diagnostics(&snap.path) {
                continue;
            }
            let items = match self.pull_diagnostics(&t, &snap.ident(), &CancellationToken::new(), PULL_TIMEOUT).await {
                Ok(items) => items,
                Err(e) => {
                    tracing::debug!("{}: pull diagnostics for {}: {e}", t.server.name, snap.path.display());
                    continue;
                }
            };
            // An edit since then: its own pull publishes the newer result.
            if !self.pull_is_current(doc, generation) {
                return;
            }
            let conv = self.conv(t.enc());
            let converted: Vec<_> =
                items.iter().cloned().map(|d| convert::diagnostic(&conv, &snap.path, d, &t.server.name)).collect();
            t.server.set_diagnostics(snap.path.clone(), items);
            self.sink.emit(IdeEvent::Diagnostics {
                path: snap.path.clone().into(),
                source: t.server.name.clone(),
                items: converted,
            });
        }
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
            let converted: Vec<_> =
                items.iter().cloned().map(|d| convert::diagnostic(&conv, &path, d, &server.name)).collect();
            server.set_diagnostics(path.clone(), items);
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
