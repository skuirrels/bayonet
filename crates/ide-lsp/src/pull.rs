//! Live diagnostics for servers that only answer `textDocument/diagnostic` and never
//! push (`pull-diagnostics` in the language's TOML: Roslyn). After a document opens or
//! changes, and when the server sends `workspace/diagnostic/refresh` (it finished
//! loading projects, a referenced file changed), each open document is pulled again
//! and the result published like a `publishDiagnostics`. Pulls are debounced per
//! document, and a newer edit drops the result of one still in flight.

use crate::LspManager;
use crate::convert;
use crate::server::Server;
use ide_core::{CancellationToken, DocId, IdeEvent};
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
        // Roslyn advertises `diagnosticProvider` only to clients that declare pull support,
        // but answers regardless; the TOML flag is what says it can.
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
