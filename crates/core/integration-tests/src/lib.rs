// This crate exists solely for system integration tests.
// All production code is in `src/` (helper modules used by `tests/`).
// No library functionality is exported for other crates.

pub mod api_client;
pub mod containers;
