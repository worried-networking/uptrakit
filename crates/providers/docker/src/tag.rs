use regex::Regex;

/// A tag with its parsed semver version.
#[derive(Debug, Clone)]
pub struct TagVersion {
    /// The original tag string.
    pub tag: String,
    /// The version string after prefix stripping.
    pub version_str: String,
    /// The parsed semver version.
    pub semver: semver::Version,
}

/// Strip a prefix from a tag name to extract the version string.
///
/// If the tag starts with the given prefix, the prefix is removed.
/// Otherwise, the tag is returned unchanged.
pub fn strip_tag_prefix<'a>(tag: &'a str, prefix: &str) -> &'a str {
    if prefix.is_empty() {
        return tag;
    }
    tag.strip_prefix(prefix).unwrap_or(tag)
}

/// Filter, parse, and sort tags by semver version (descending).
///
/// - `patterns`: regex patterns for tag filtering (OR logic, empty = all tags)
/// - `strip_prefix`: prefix to strip before semver parsing
/// - `include_prereleases`: whether to include pre-release versions
///
/// Tags that don't parse as semver after prefix stripping are excluded.
pub fn filter_and_sort_tags(
    tags: &[String],
    patterns: &[Regex],
    strip_prefix: &str,
    include_prereleases: bool,
) -> Vec<TagVersion> {
    let mut result: Vec<TagVersion> = tags
        .iter()
        .filter(|tag| {
            if patterns.is_empty() {
                return true;
            }
            patterns.iter().any(|re| re.is_match(tag))
        })
        .filter_map(|tag| {
            let version_str = strip_tag_prefix(tag, strip_prefix);
            let semver = semver::Version::parse(version_str).ok()?;

            if !include_prereleases && !semver.pre.is_empty() {
                return None;
            }

            Some(TagVersion {
                tag: tag.clone(),
                version_str: version_str.to_string(),
                semver,
            })
        })
        .collect();

    // Sort descending by semver version
    result.sort_by(|a, b| b.semver.cmp(&a.semver));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_v_prefix() {
        assert_eq!(strip_tag_prefix("v1.0.0", "v"), "1.0.0");
    }

    #[test]
    fn strips_release_prefix() {
        assert_eq!(strip_tag_prefix("release-1.2.3", "release-"), "1.2.3");
    }

    #[test]
    fn no_prefix_returns_unchanged() {
        assert_eq!(strip_tag_prefix("1.0.0", "v"), "1.0.0");
    }

    #[test]
    fn empty_prefix_returns_tag() {
        assert_eq!(strip_tag_prefix("v1.0.0", ""), "v1.0.0");
    }

    #[test]
    fn empty_tag_returns_empty() {
        assert_eq!(strip_tag_prefix("", "v"), "");
    }

    #[test]
    fn prefix_longer_than_tag() {
        assert_eq!(strip_tag_prefix("v", "version-"), "v");
    }

    #[test]
    fn case_sensitive() {
        assert_eq!(strip_tag_prefix("V1.0.0", "v"), "V1.0.0");
    }

    #[test]
    fn filter_and_sort_basic() {
        let tags: Vec<String> = vec![
            "v1.0.0".to_string(),
            "v2.0.0".to_string(),
            "v1.5.0".to_string(),
        ];
        let result = filter_and_sort_tags(&tags, &[], "v", false);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].version_str, "2.0.0");
        assert_eq!(result[1].version_str, "1.5.0");
        assert_eq!(result[2].version_str, "1.0.0");
    }

    #[test]
    fn filter_and_sort_excludes_non_semver() {
        let tags: Vec<String> = vec![
            "v1.0.0".to_string(),
            "latest".to_string(),
            "alpine".to_string(),
            "v2.0.0".to_string(),
        ];
        let result = filter_and_sort_tags(&tags, &[], "v", false);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].version_str, "2.0.0");
        assert_eq!(result[1].version_str, "1.0.0");
    }

    #[test]
    fn filter_and_sort_with_patterns() {
        let tags: Vec<String> = vec![
            "v1.0.0".to_string(),
            "v2.0.0-alpine".to_string(),
            "v1.5.0".to_string(),
        ];
        let patterns = vec![Regex::new(r"^v\d+\.\d+\.\d+$").expect("valid regex")];
        let result = filter_and_sort_tags(&tags, &patterns, "v", false);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].version_str, "1.5.0");
        assert_eq!(result[1].version_str, "1.0.0");
    }

    #[test]
    fn filter_and_sort_excludes_prereleases() {
        let tags: Vec<String> = vec![
            "1.0.0".to_string(),
            "2.0.0-beta.1".to_string(),
            "1.5.0".to_string(),
        ];
        let result = filter_and_sort_tags(&tags, &[], "", false);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].version_str, "1.5.0");
        assert_eq!(result[1].version_str, "1.0.0");
    }

    #[test]
    fn filter_and_sort_includes_prereleases() {
        let tags: Vec<String> = vec![
            "1.0.0".to_string(),
            "2.0.0-beta.1".to_string(),
            "1.5.0".to_string(),
        ];
        let result = filter_and_sort_tags(&tags, &[], "", true);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].version_str, "2.0.0-beta.1");
        assert_eq!(result[1].version_str, "1.5.0");
        assert_eq!(result[2].version_str, "1.0.0");
    }

    #[test]
    fn filter_and_sort_descending_order() {
        let tags: Vec<String> = vec![
            "1.0.0".to_string(),
            "1.10.0".to_string(),
            "1.2.0".to_string(),
            "1.9.0".to_string(),
        ];
        let result = filter_and_sort_tags(&tags, &[], "", false);
        let versions: Vec<&str> = result.iter().map(|t| t.version_str.as_str()).collect();
        assert_eq!(versions, ["1.10.0", "1.9.0", "1.2.0", "1.0.0"]);
    }

    #[test]
    fn filter_and_sort_empty_tags() {
        let tags: Vec<String> = vec![];
        let result = filter_and_sort_tags(&tags, &[], "v", false);
        assert!(result.is_empty());
    }

    #[test]
    fn filter_and_sort_all_non_semver() {
        let tags: Vec<String> = vec![
            "latest".to_string(),
            "alpine".to_string(),
            "slim".to_string(),
        ];
        let result = filter_and_sort_tags(&tags, &[], "", false);
        assert!(result.is_empty());
    }

    #[test]
    fn filter_and_sort_multiple_patterns_or_logic() {
        let tags: Vec<String> = vec![
            "v1.0.0".to_string(),
            "release-2.0.0".to_string(),
            "v3.0.0".to_string(),
        ];
        let patterns = vec![
            Regex::new(r"^v").expect("valid regex"),
            Regex::new(r"^release-").expect("valid regex"),
        ];
        // "release-2.0.0" won't parse as semver after stripping "v" prefix
        // but "v1.0.0" and "v3.0.0" will
        let result = filter_and_sort_tags(&tags, &patterns, "v", false);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].version_str, "3.0.0");
        assert_eq!(result[1].version_str, "1.0.0");
    }

    #[test]
    fn tag_version_preserves_original_tag() {
        let tags: Vec<String> = vec!["v1.2.3".to_string()];
        let result = filter_and_sort_tags(&tags, &[], "v", false);
        assert_eq!(result[0].tag, "v1.2.3");
        assert_eq!(result[0].version_str, "1.2.3");
    }
}
