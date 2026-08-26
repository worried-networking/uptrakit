//! Tag-series prefix/version inference for PHS-discovered software.
//!
//! Upstream monorepos tag per component (`uptrakit-controller-standalone-v0.0.7`);
//! the PHS version helper reports that raw tag verbatim. [`split_tag_version`]
//! infers where the series prefix ends and the version begins so discovery can
//! report a bare `installed_version` and synthesize `tag_prefix` /
//! `version_strip_prefix` overrides on the release-fetch and shell targets.

use std::sync::LazyLock;

use regex::Regex;
use uptrakit_shared_types::version_prefix::validate_version_prefix;

/// Version tail shape: `\d+(\.\d+)+` (at least one dot — a bare integer never
/// matches) plus an optional pre-release/build suffix, anchored to the whole
/// candidate slice. Wrapped in `Option` so a (statically impossible) compile
/// failure degrades to "no inference" instead of panicking.
static VERSION_TAIL: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^\d+(?:\.\d+)+(?:[-+][0-9A-Za-z.+-]*)?$").ok());

/// Split a raw tag/version string into `(series_prefix, version)`.
///
/// The version starts at the LAST candidate boundary — a digit not preceded
/// by another digit or a dot — whose tail matches the version shape through
/// the end of the string. Preferring the last candidate keeps name parts that
/// contain dotted numbers in the prefix: `ubuntu-22.04-v1.2.3` →
/// (`ubuntu-22.04-v`, `1.2.3`).
///
/// Returns `None` when no version shape is found (conservative
/// no-match-no-change: callers keep today's verbatim behavior and synthesize
/// no overrides). The prefix may be empty (`"1.2.3"` → `("", "1.2.3")`).
pub fn split_tag_version(raw: &str) -> Option<(&str, &str)> {
    let tail = VERSION_TAIL.as_ref()?;
    let mut best = None;
    let mut prev: Option<char> = None;
    for (i, ch) in raw.char_indices() {
        // A version can only start on a digit that opens a number: mid-number
        // positions (previous char is a digit or a dot) are not boundaries.
        let boundary =
            ch.is_ascii_digit() && !matches!(prev, Some(p) if p.is_ascii_digit() || p == '.');
        // raw.get(i..) rather than &raw[i..]: i is always a char boundary, but
        // the workspace denies clippy::string_slice.
        if boundary && raw.get(i..).is_some_and(|c| tail.is_match(c)) {
            best = Some(i);
        }
        prev = Some(ch);
    }
    best.map(|i| raw.split_at(i))
}

/// Normalize a raw installed version into `(series_prefix, bare_version)`.
///
/// A non-empty inferred prefix is returned separately and stripped from the
/// version — but only when it contains at least one ASCII letter (a series
/// prefix is a name; a purely numeric "prefix" like `1.4.0-` out of
/// `1.4.0-2024.08.19` would synthesize a filter matching no release) and it
/// satisfies [`validate_version_prefix`] (no surrounding whitespace or
/// control chars, within the shared length cap) AND contains no interior
/// whitespace (a forge tag is a git ref, which can never hold a space, so a
/// spaced prefix is a `tag_prefix` filter that matches zero releases —
/// `validate_version_prefix` permits interior whitespace, so this is a
/// tighter check layered on top), so discovery never
/// synthesizes an override value the config forms would reject. Any other
/// outcome keeps the input verbatim with no prefix. Owned-`String` shape so
/// callers can move the result straight into
/// `DiscoveredSoftware.installed_version`.
pub fn normalize_installed_version(raw: String) -> (Option<String>, String) {
    match split_tag_version(&raw) {
        Some((prefix, version))
            if prefix.chars().any(|c| c.is_ascii_alphabetic())
                && !prefix.chars().any(char::is_whitespace)
                && validate_version_prefix(prefix, "inferred_prefix").is_ok() =>
        {
            (Some(prefix.to_string()), version.to_string())
        }
        _ => (None, raw),
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_shared_types::version_prefix::MAX_VERSION_PREFIX_LENGTH;

    use super::*;

    #[test]
    fn splits_series_prefixed_tag() {
        assert_eq!(
            split_tag_version("uptrakit-controller-standalone-v0.0.7"),
            Some(("uptrakit-controller-standalone-v", "0.0.7"))
        );
    }

    #[test]
    fn splits_v_prefix() {
        assert_eq!(split_tag_version("v1.2.3"), Some(("v", "1.2.3")));
    }

    #[test]
    fn splits_bare_version_with_empty_prefix() {
        assert_eq!(split_tag_version("1.2.3"), Some(("", "1.2.3")));
    }

    #[test]
    fn digit_in_prefix_is_not_a_version_candidate() {
        // "2" opens a number but "2-v1.2.3" fails the version shape (no dot
        // group), so the last matching candidate is "1.2.3".
        assert_eq!(split_tag_version("app2-v1.2.3"), Some(("app2-v", "1.2.3")));
    }

    #[test]
    fn build_suffix_included_in_version() {
        // Mid-number digits ("28", "3") and suffix digits ("1" in "k3s1") are
        // not candidate boundaries or fail the shape — the split stays put.
        assert_eq!(
            split_tag_version("k3s-v1.28.3+k3s1"),
            Some(("k3s-v", "1.28.3+k3s1"))
        );
    }

    #[test]
    fn prerelease_suffix_included_in_version() {
        assert_eq!(split_tag_version("1.2.3-rc.1"), Some(("", "1.2.3-rc.1")));
    }

    #[test]
    fn last_candidate_wins_dotted_number_in_name() {
        assert_eq!(
            split_tag_version("ubuntu-22.04-v1.2.3"),
            Some(("ubuntu-22.04-v", "1.2.3"))
        );
    }

    #[test]
    fn last_candidate_wins_attached_dotted_digits() {
        assert_eq!(
            split_tag_version("php8.2-fpm-1.2.3"),
            Some(("php8.2-fpm-", "1.2.3"))
        );
    }

    #[test]
    fn two_full_versions_split_at_the_last() {
        // Pinned: when two complete version shapes stack, the last one is the
        // version and everything before it is prefix.
        assert_eq!(split_tag_version("1.2.3-4.5.6"), Some(("1.2.3-", "4.5.6")));
    }

    #[test]
    fn bare_integer_never_splits() {
        assert_eq!(split_tag_version("123"), None);
    }

    #[test]
    fn no_digits_no_split() {
        assert_eq!(split_tag_version("latest"), None);
    }

    #[test]
    fn empty_string_no_split() {
        assert_eq!(split_tag_version(""), None);
    }

    #[test]
    fn dateish_dashes_no_split() {
        // Dashes, no dots: the version shape never matches.
        assert_eq!(split_tag_version("2024-01-05"), None);
    }

    #[test]
    fn normalize_strips_nonempty_prefix() {
        assert_eq!(
            normalize_installed_version("uptrakit-controller-standalone-v0.0.7".to_string()),
            (
                Some("uptrakit-controller-standalone-v".to_string()),
                "0.0.7".to_string()
            )
        );
    }

    #[test]
    fn normalize_keeps_bare_version_verbatim() {
        assert_eq!(
            normalize_installed_version("1.2.3".to_string()),
            (None, "1.2.3".to_string())
        );
    }

    #[test]
    fn normalize_no_match_verbatim() {
        assert_eq!(
            normalize_installed_version("latest".to_string()),
            (None, "latest".to_string())
        );
    }

    #[test]
    fn normalize_rejects_prefix_with_trailing_whitespace() {
        // The inferred prefix "myapp " fails validate_version_prefix, so the
        // whole input stays verbatim — discovery must never synthesize an
        // override value the config forms would reject.
        assert_eq!(
            normalize_installed_version("myapp 1.2.3".to_string()),
            (None, "myapp 1.2.3".to_string())
        );
    }

    #[test]
    fn normalize_rejects_prefix_with_interior_whitespace() {
        // validate_version_prefix permits interior whitespace, but a forge tag
        // (a git ref) can never contain a space, so a spaced prefix is a
        // tag_prefix filter that matches zero upstream releases — reject it and
        // keep the input verbatim with no prefix.
        assert_eq!(
            normalize_installed_version("my app v1.2.3".to_string()),
            (None, "my app v1.2.3".to_string())
        );
    }

    #[test]
    fn normalize_rejects_letterless_prefix() {
        // "1.4.0-2024.08.19" splits at the last candidate ("2024.08.19"), but
        // the numeric-only prefix "1.4.0-" is a version, not a series name —
        // synthesized as tag_prefix it would filter out every real release.
        assert_eq!(
            normalize_installed_version("1.4.0-2024.08.19".to_string()),
            (None, "1.4.0-2024.08.19".to_string())
        );
    }

    #[test]
    fn normalize_rejects_overlong_prefix() {
        let raw = format!("{}v1.2.3", "x".repeat(MAX_VERSION_PREFIX_LENGTH + 1));
        assert_eq!(normalize_installed_version(raw.clone()), (None, raw));
    }
}
