//! ZCode local data access layer.
//!
//! Design constraints (from the product requirements):
//! - READ-ONLY access to ZCode data. Never write, move or lock its files.
//! - No schema assumptions: SQLite table/column layout is discovered at
//!   runtime, JSONL fields are matched through alias sets. Anything that
//!   cannot be mapped is reported as "unavailable", never guessed.
//! - Incremental reads only (JSONL byte-offset watermarks, SQLite
//!   rowid/primary-key watermarks). No full-history rescans on refresh.

pub mod aggregate;
pub mod discover;
pub mod errors;
pub mod jsonl;
pub mod pricing;
pub mod sqlite;
pub mod store;
pub mod usage;
