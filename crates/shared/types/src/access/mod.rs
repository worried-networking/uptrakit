//! Typed access-control vocabulary: actions (`resource:verb`), grant
//! patterns, selectors, and the built-in action catalog.
//!
//! One identifier grammar covers built-in resources and both dynamic
//! namespaces (`plugin.<plugin_type>`, `surface.<surface_id>`): resources
//! are dot-separated kebab-case segments, verbs come from a closed
//! nine-verb set. There is deliberately **no** `Other` catch-all — an
//! unknown or malformed action string is a parse error, and a parse error
//! is a deny. `system.`-prefixed resources are excluded from the `*`
//! wildcard and match only patterns literally starting with `system`.
//!
//! Design: `docs/superpowers/specs/2026-07-28-access-types-core-design.md`
//! and `.superpowers/authn-and-authz-refactoring/05-action-model.md`.

pub mod bounds;
mod selector;
mod verb;

pub use selector::{Selector, SelectorSupport, SelectorValidationError};
pub use verb::{ParseVerbError, Verb};
