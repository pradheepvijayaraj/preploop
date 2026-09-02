//! Structured, serializable errors shared by the database and command layers.

use serde::Serialize;
use std::fmt::{Display, Formatter};

pub type LoopResult<T> = Result<T, LoopError>;

/// Stable error contract returned through Tauri IPC.
///
/// Internal library details are logged locally and never serialized. Expected
/// domain failures retain a machine-readable code and a user-safe message.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "code", content = "message", rename_all = "snake_case")]
pub enum LoopError {
    InvalidInput(String),
    NotFound(String),
    InvalidState(String),
    Unavailable(String),
    Internal(String),
}

impl LoopError {
    pub fn internal(error: impl Display) -> Self {
        log::error!("Internal backend error: {error}");
        Self::Internal("An internal error occurred".to_string())
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn invalid_state(message: impl Into<String>) -> Self {
        Self::InvalidState(message.into())
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable(message.into())
    }

    pub fn message(&self) -> &str {
        match self {
            Self::InvalidInput(message)
            | Self::NotFound(message)
            | Self::InvalidState(message)
            | Self::Unavailable(message)
            | Self::Internal(message) => message,
        }
    }
}

impl Display for LoopError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for LoopError {}

/// Fallible enum conversions still expose `String` for serde/rusqlite
/// compatibility. Once they cross into backend logic they represent corrupt
/// persisted data, so convert them to an internal error rather than guessing a
/// user-facing domain category.
impl From<String> for LoopError {
    fn from(error: String) -> Self {
        Self::internal(error)
    }
}

/// Extension trait for converting library errors into user-safe strings.
///
/// All errors flowing through `stringify_err` are assumed to originate from
/// external libraries (rusqlite, serde_json, std::io).  The raw message is
/// logged through the configured `log` facade while a generic, user-safe
/// message is returned to the frontend.
///
/// Domain-specific failures are constructed directly as `LoopError` variants
/// and do not pass through this trait.
pub trait ResultExt<T> {
    fn stringify_err(self) -> LoopResult<T>;
}

impl<T, E: std::fmt::Display> ResultExt<T> for Result<T, E> {
    fn stringify_err(self) -> LoopResult<T> {
        self.map_err(LoopError::internal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_stable_code_and_safe_message() {
        let value = serde_json::to_value(LoopError::not_found("Question bank not found")).unwrap();
        assert_eq!(value["code"], "not_found");
        assert_eq!(value["message"], "Question bank not found");
    }

    #[test]
    fn internal_errors_do_not_serialize_library_details() {
        let value = serde_json::to_value(LoopError::internal("SQL text and disk path")).unwrap();
        assert_eq!(value["code"], "internal");
        assert_eq!(value["message"], "An internal error occurred");
        assert!(!value.to_string().contains("SQL text"));
    }
}
