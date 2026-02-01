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
}
