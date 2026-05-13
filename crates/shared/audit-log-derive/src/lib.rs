//! Proc-macros for the uptrakit semantic audit log subsystem.
//!
//! - [`AuditView`] derive — projects a domain entity into snapshot JSON.
//! - [`audit_actions!`] — generates per-action constructor methods on `AuditEntry`.

use proc_macro::TokenStream;

mod audit_actions;
mod audit_view;

/// Derive macro that projects a domain entity into a snapshot JSON representation
/// suitable for inclusion in an audit log entry.
///
/// # Attributes
///
/// Field-level `#[audit(...)]` attributes control projection behaviour (to be
/// defined in a future implementation pass).
///
/// # Example
///
/// ```rust,ignore
/// use uptrakit_audit_log_derive::AuditView;
///
/// #[derive(AuditView)]
/// pub struct HostSnapshot {
///     pub id: uuid::Uuid,
///     pub name: String,
/// }
/// ```
#[proc_macro_derive(AuditView, attributes(audit))]
pub fn derive_audit_view(input: TokenStream) -> TokenStream {
    audit_view::expand(input)
}

/// Function-like macro that generates per-action constructor methods on `AuditEntry`.
///
/// **Internal macro.** The generated code uses `crate::` paths and is only
/// correct when invoked from within `uptrakit-audit-log` itself.
///
/// # Example
///
/// ```rust,ignore
/// audit_actions! {
///     auth_login => AUTH_LOGIN, Event;
/// }
/// ```
#[doc(hidden)]
#[proc_macro]
pub fn audit_actions(input: TokenStream) -> TokenStream {
    audit_actions::expand(input)
}

/// Marker attribute for the `audit-coverage-check` static-analysis tool.
///
/// Functions annotated with `#[audit_required]` are treated as mutation sites
/// that require a catalog entry, even when they do not match the Axum handler
/// or scheduler detection patterns used by the walker.
///
/// This attribute is a **no-op at compile time** — it passes the annotated item
/// through unchanged. The `audit-coverage-check` binary reads the attribute
/// from source text via `syn`, not from the compiled artifact.
///
/// # Example
///
/// ```rust,ignore
/// use uptrakit_audit_log::audit_required;
///
/// #[audit_required]
/// pub async fn enqueue_batch_update(/* … */) {
///     // Must have a catalog entry.
/// }
/// ```
#[proc_macro_attribute]
pub fn audit_required(_attrs: TokenStream, item: TokenStream) -> TokenStream {
    item
}
