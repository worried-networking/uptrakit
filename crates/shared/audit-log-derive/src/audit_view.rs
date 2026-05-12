//! Expansion logic for the `#[derive(AuditView)]` macro.
//!
//! Currently a stub; full projection logic will be added in a subsequent
//! implementation pass once `AuditEntry` and the snapshot JSON contract are
//! finalised.

use proc_macro::TokenStream;

/// Expand the `AuditView` derive macro.
///
/// Returns an empty token stream until the full implementation is in place.
pub(crate) fn expand(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}
