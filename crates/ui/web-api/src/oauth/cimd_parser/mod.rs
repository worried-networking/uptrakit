//! Versioned CIMD document parser dispatch.
//!
//! Two-pass parsing: caller converts raw bytes to [`serde_json::Value`] first,
//! then calls the versioned parser. This lets the fetcher persist raw bytes
//! regardless of parse outcome.

pub mod v0_draft00;

pub use v0_draft00::{CimdDocument, CimdParseError, extract};
