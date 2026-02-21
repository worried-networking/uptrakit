/// Uptrakit CLI library target.
///
/// Exposes the CLI's internal modules so that integration tests in `tests/`
/// can import command functions and helpers without going through the binary
/// entry point.
pub mod client;
pub mod commands;
pub mod config;
pub mod error;
pub mod output;
