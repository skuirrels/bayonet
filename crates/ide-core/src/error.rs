use serde::{Deserialize, Serialize};
use specta::Type;

pub type IdeResult<T> = Result<T, IdeError>;

/// The single error type that crosses the IPC boundary.
///
/// Crates keep rich internal errors and convert at their public edge; the UI
/// switches on `kind` and shows `message`.
#[derive(Clone, Debug, thiserror::Error, Serialize, Deserialize, Type)]
#[error("{kind:?}: {message}")]
pub struct IdeError {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    NotFound,
    AlreadyExists,
    PermissionDenied,
    InvalidInput,
    /// Document version mismatch; the UI must resync the full text.
    OutOfSync,
    Cancelled,
    /// A required external tool (language server, git, debugger) is unavailable.
    Unavailable,
    Io,
    Protocol,
    Internal,
}

impl IdeError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, m)
    }
    pub fn invalid(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, m)
    }
    pub fn internal(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, m)
    }
    pub fn unavailable(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unavailable, m)
    }
    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "cancelled")
    }
}

impl From<std::io::Error> for IdeError {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind as K;
        let kind = match e.kind() {
            K::NotFound => ErrorKind::NotFound,
            K::AlreadyExists => ErrorKind::AlreadyExists,
            K::PermissionDenied => ErrorKind::PermissionDenied,
            K::InvalidInput | K::InvalidData => ErrorKind::InvalidInput,
            _ => ErrorKind::Io,
        };
        Self::new(kind, e.to_string())
    }
}
