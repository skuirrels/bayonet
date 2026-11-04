use ide_core::{EventSink, IdeEvent};
use tauri::{AppHandle, Emitter};

/// Name of the single backend → UI event. Payload: [`IdeEvent`].
pub const IDE_EVENT: &str = "ide://event";

/// Forwards events to one window (`Some(label)`) or to all windows.
pub struct TauriSink {
    pub app: AppHandle,
    pub window: Option<String>,
}

impl EventSink for TauriSink {
    fn emit(&self, event: IdeEvent) {
        let result = match &self.window {
            Some(label) => self.app.emit_to(label.as_str(), IDE_EVENT, &event),
            None => self.app.emit(IDE_EVENT, &event),
        };
        if let Err(e) = result {
            tracing::warn!("failed to emit ide event: {e}");
        }
    }
}
