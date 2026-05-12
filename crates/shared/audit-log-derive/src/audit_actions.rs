//! Expansion logic for the `audit_actions!` function-like macro.
//!
//! Currently a stub; full constructor-generation logic will be added in a
//! subsequent implementation pass once `AuditEntry` and the action registry
//! contract are finalised.

use proc_macro::TokenStream;

/// Expand the `audit_actions!` macro.
///
/// Returns an empty token stream until the full implementation is in place.
pub(crate) fn expand(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}
