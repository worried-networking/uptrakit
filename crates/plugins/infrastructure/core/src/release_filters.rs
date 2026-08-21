//! Shared diagnostics for release-source plugins.
//!
//! Release-source plugins (GitHub, Forgejo, …) narrow an upstream release
//! listing with the same two filters: a `tag_prefix` series filter and
//! `asset_patterns` asset gating.  When those filters remove every release
//! that passed the draft/prerelease baseline, the result is a configuration
//! mistake rather than an empty upstream — and the operator-facing wording
//! explaining that must not drift between plugins, so it lives here.

/// Inputs describing one fully-filtered `fetch_releases` result.
///
/// Built per fetch by the calling plugin; see
/// [`FilteredOutDiagnostic::message`] for the decision rules.
#[derive(Debug)]
pub struct FilteredOutDiagnostic<'a> {
    /// Releases returned by the upstream API before any filtering.
    pub raw_count: usize,
    /// Releases that passed the draft/prerelease baseline checks.
    pub baseline_count: usize,
    /// Releases that survived every filter — the fetch result.
    pub surviving_count: usize,
    /// Whether the pagination window was consumed without proving the
    /// listing ended (matching releases may exist beyond it).
    pub window_exhausted: bool,
    /// Pagination window: maximum pages fetched.
    pub max_pages: usize,
    /// Pagination window: releases requested per page.
    pub per_page: usize,
    /// Configured series prefix, if any.
    pub tag_prefix: Option<&'a str>,
    /// Configured asset patterns, reported verbatim in the message.
    pub asset_patterns: &'a [String],
    /// Whether the plugin holds compiled asset filters.
    ///
    /// Taken from the compiled filters rather than [`Self::asset_patterns`]
    /// so this check can never disagree with the gating the plugin applies.
    pub asset_filters_active: bool,
}

impl FilteredOutDiagnostic<'_> {
    /// Decide whether a fully-filtered fetch result is a configuration error.
    ///
    /// Returns `Some(message)` when releases passed baseline checks but the
    /// series/asset filters removed all of them — a misconfigured
    /// `tag_prefix`/`asset_patterns` would otherwise present as a silent
    /// "no releases". Returns `None` for genuinely empty results or when no
    /// filter is active.
    pub fn message(&self) -> Option<String> {
        if self.baseline_count == 0 || self.surviving_count > 0 {
            return None;
        }
        let tag_prefix_active = self.tag_prefix.is_some_and(|p| !p.is_empty());
        if !tag_prefix_active && !self.asset_filters_active {
            return None;
        }
        let window_note = if self.window_exhausted {
            format!(
                "the fetch window ({} pages x {} releases = newest {}) was exhausted, so \
                 matching releases may exist beyond it",
                self.max_pages,
                self.per_page,
                self.max_pages * self.per_page,
            )
        } else {
            "every upstream release was fetched, so nothing upstream matches the filters"
                .to_string()
        };
        Some(format!(
            "no releases survive the configured filters (tag_prefix={:?}, asset_patterns={:?}): \
             {} of {} fetched releases passed draft/prerelease checks but all were filtered out; \
             {window_note}. These filters come from the assignment's effective config — a plugin \
             config profile or a per-host config_override (e.g. one written by autodiscovery). To \
             recover, edit or clear tag_prefix on this item's fetch_releases assignment and \
             version_strip_prefix on its detect_version assignment (an upstream series rename \
             stales both together). Do not set tag_strip_prefix to the full series prefix: it \
             strips without filtering and recreates cross-series phantom updates.",
            self.tag_prefix, self.asset_patterns, self.baseline_count, self.raw_count,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic<'a>(
        baseline_count: usize,
        surviving_count: usize,
        tag_prefix: Option<&'a str>,
        asset_patterns: &'a [String],
    ) -> FilteredOutDiagnostic<'a> {
        FilteredOutDiagnostic {
            raw_count: 12,
            baseline_count,
            surviving_count,
            window_exhausted: false,
            max_pages: 10,
            per_page: 50,
            tag_prefix,
            asset_patterns,
            asset_filters_active: !asset_patterns.is_empty(),
        }
    }

    #[test]
    fn empty_upstream_is_not_an_error() {
        assert!(diagnostic(0, 0, Some("app-v"), &[]).message().is_none());
    }

    #[test]
    fn survivors_are_not_an_error() {
        assert!(diagnostic(5, 2, Some("app-v"), &[]).message().is_none());
    }

    #[test]
    fn no_active_filter_is_not_an_error() {
        assert!(diagnostic(5, 0, None, &[]).message().is_none());
        assert!(diagnostic(5, 0, Some(""), &[]).message().is_none());
    }

    #[test]
    fn filtered_to_zero_names_filters_and_counts() {
        let patterns = vec![r".*\.deb$".to_string()];
        let msg = diagnostic(5, 0, Some("app-v"), &patterns)
            .message()
            .expect("filtered-to-zero is a configuration error");
        assert!(msg.contains("tag_prefix=Some(\"app-v\")"));
        // Debug formatting of the pattern list escapes the regex backslash.
        assert!(msg.contains(r#"asset_patterns=[".*\\.deb$"]"#));
        assert!(msg.contains("5 of 12 fetched releases"));
        assert!(msg.contains("every upstream release was fetched"));
        assert!(msg.contains("Do not set tag_strip_prefix"));
    }

    #[test]
    fn exhausted_window_notes_releases_beyond_it() {
        let mut d = diagnostic(5, 0, Some("app-v"), &[]);
        d.asset_filters_active = false;
        d.window_exhausted = true;
        let msg = d
            .message()
            .expect("filtered-to-zero is a configuration error");
        assert!(msg.contains("10 pages x 50 releases = newest 500"));
        assert!(msg.contains("may exist beyond it"));
    }
}
