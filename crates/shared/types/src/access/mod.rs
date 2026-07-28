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

mod action;
pub mod bounds;
mod catalog;
mod selector;
mod verb;

pub use action::{Action, ParseActionError};
pub use catalog::actions;
pub use catalog::{CATALOG, CatalogEntry, ParseResourceError, Resource, VerbEntry};
pub use selector::{Selector, SelectorSupport, SelectorValidationError};
pub use verb::{ParseVerbError, Verb};

/// Returns `true` when `s` is a single kebab-case identifier segment:
/// `^[a-z][a-z0-9]*(-[a-z0-9]+)*$`.
pub(crate) fn is_valid_segment(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    let mut prev_hyphen = false;
    for c in chars {
        match c {
            'a'..='z' | '0'..='9' => prev_hyphen = false,
            '-' if !prev_hyphen => prev_hyphen = true,
            _ => return false,
        }
    }
    !prev_hyphen
}

/// Returns `true` when `s` is one or more valid segments joined by `.`.
pub(crate) fn is_valid_segment_path(s: &str) -> bool {
    !s.is_empty() && s.split('.').all(is_valid_segment)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_grammar_table() {
        // (input, expected) — A3/A7 shapes at segment-path level.
        let cases: &[(&str, bool)] = &[
            ("a", true),
            ("hosts", true),
            ("a1", true),
            ("a-b", true),
            ("a-1", true),
            ("package-manager.apt", true),
            ("ssh-agent.hosts", true),
            ("a.b.c", true),
            ("", false),
            ("A", false),
            ("aB", false),
            ("1a", false),
            ("-a", false),
            ("a-", false),
            ("a--b", false),
            ("a_b", false),
            ("a..b", false),
            (".a", false),
            ("a.", false),
            ("a.*", false),
            ("*", false),
        ];
        for (input, expected) in cases {
            assert_eq!(
                is_valid_segment_path(input),
                *expected,
                "segment path grammar for {input:?}"
            );
        }
    }
}
