//! Error classification for data-source reads.
//!
//! The distinction matters for the retry policy:
//! - `Busy` / `RetryLater`: the file is temporarily locked (ZCode is writing
//!   right now). The engine schedules another attempt shortly; the previous
//!   statistics stay on screen.
//! - `Gone`: the file disappeared (deleted / rotated). Its state is dropped.
//! - `Fatal`: the source is structurally broken. It is skipped and surfaced
//!   in the data-source inspector instead of crashing the app.

use std::fmt;

#[derive(Debug)]
pub enum SourceError {
    Busy,
    RetryLater(String),
    Gone,
    Fatal(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::Busy => write!(f, "source is busy (locked by writer)"),
            SourceError::RetryLater(m) => write!(f, "retry later: {m}"),
            SourceError::Gone => write!(f, "source disappeared"),
            SourceError::Fatal(m) => write!(f, "fatal: {m}"),
        }
    }
}

impl std::error::Error for SourceError {}

pub fn classify_io_error(err: &std::io::Error) -> SourceError {
    use std::io::ErrorKind::*;
    match err.kind() {
        NotFound => SourceError::Gone,
        PermissionDenied => SourceError::RetryLater(err.to_string()),
        // Windows sharing violations (file being replaced) arrive as Uncategorized
        // with os error 32/33; treat unknown transient errors as retryable.
        _ => match err.raw_os_error() {
            Some(32) | Some(33) => SourceError::RetryLater(err.to_string()),
            _ => SourceError::Fatal(err.to_string()),
        },
    }
}
