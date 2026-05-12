//! Expansion logic for the `audit_actions!` function-like macro.
//!
//! Generates per-action constructor methods on [`uptrakit_audit_log::AuditEntry`].
//!
//! # Syntax
//!
//! ```text
//! audit_actions! {
//!     <method_name> => <CONST_NAME>, <Kind>;
//!     ...
//! }
//! ```
//!
//! where `<Kind>` is either `Event` or `Stateful`.
//!
//! - `Event` entries produce `AuditEntry<Event>::<method_name>() -> AuditEntryBuilder<Event>`.
//! - `Stateful` entries produce
//!   `AuditEntry<Stateful>::<method_name>(before: &B, after: &A) -> AuditEntryBuilder<Stateful, HasBefore, HasAfter>`.
//!
//! # Call-site contract
//!
//! This macro is **only** intended to be invoked from within `uptrakit-audit-log`
//! itself.  Generated code uses `crate::` paths so it resolves correctly inside
//! that crate without a circular dependency.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Ident, Token,
    parse::{Parse, ParseStream},
    parse_macro_input,
};

struct ActionList(Vec<Action>);

struct Action {
    method: Ident,
    action_const: Ident,
    kind: Ident,
}

impl Parse for ActionList {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut out = Vec::new();
        while !input.is_empty() {
            let method: Ident = input.parse()?;
            input.parse::<Token![=>]>()?;
            let action_const: Ident = input.parse()?;
            input.parse::<Token![,]>()?;
            let kind: Ident = input.parse()?;
            if !input.is_empty() {
                input.parse::<Token![;]>()?;
            }
            out.push(Action {
                method,
                action_const,
                kind,
            });
        }
        Ok(ActionList(out))
    }
}

/// Expand the `audit_actions!` macro.
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let list = parse_macro_input!(input as ActionList);
    let mut items: Vec<TokenStream2> = Vec::new();

    for a in list.0 {
        let m = &a.method;
        let c = &a.action_const;
        let item = if a.kind == "Event" {
            quote! {
                impl crate::entry::AuditEntry<crate::entry::Event> {
                    /// Returns a builder pre-configured with the
                    #[doc = concat!("`", stringify!(#c), "`")]
                    /// action type.
                    #[must_use]
                    pub fn #m() -> crate::entry::AuditEntryBuilder<crate::entry::Event> {
                        <Self>::builder_event(
                            crate::action_type::AuditActionType::from(
                                crate::action_type::AuditActionType::#c,
                            ),
                        )
                    }
                }
            }
        } else if a.kind == "Stateful" {
            quote! {
                impl crate::entry::AuditEntry<crate::entry::Stateful> {
                    /// Returns a builder pre-configured with the
                    #[doc = concat!("`", stringify!(#c), "`")]
                    /// action type, with `before` and `after` snapshots already captured.
                    #[must_use]
                    pub fn #m<
                        B: crate::entry::AuditView,
                        A: crate::entry::AuditView,
                    >(
                        before: &B,
                        after: &A,
                    ) -> crate::entry::AuditEntryBuilder<
                        crate::entry::Stateful,
                        crate::entry::HasBefore,
                        crate::entry::HasAfter,
                    > {
                        <Self>::builder_stateful(
                            crate::action_type::AuditActionType::from(
                                crate::action_type::AuditActionType::#c,
                            ),
                        )
                        .before(before)
                        .after(after)
                    }
                }
            }
        } else {
            let msg = format!(
                "audit_actions!: unknown kind `{}`; expected `Event` or `Stateful`",
                a.kind
            );
            quote! { ::std::compile_error!(#msg); }
        };
        items.push(item);
    }

    let out: TokenStream2 = quote! { #(#items)* };
    out.into()
}
