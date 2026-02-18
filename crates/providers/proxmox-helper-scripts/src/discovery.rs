use rootcause::prelude::*;
use uptrakit_provider_core::ProviderError;

/// Well-known path where PHS containers store their update script.
pub const UPDATE_SCRIPT_PATH: &str = "/usr/bin/update";

/// Base URL prefix for PHS community-scripts CT scripts.
const PHS_CT_URL_PREFIX: &str =
    "https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/";

/// A discovered PHS script reference extracted from the update script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhsScript {
    /// The slug extracted from the script URL (e.g. `booklore`, `crafty-controller`).
    pub slug: String,
    /// The full URL to the community-scripts CT script.
    pub script_url: String,
}

/// Parse PHS script references from the content of `/usr/bin/update`.
///
/// Scans each line for occurrences of the community-scripts CT URL prefix,
/// extracts the slug from the URL path, validates it, and deduplicates by slug.
pub fn parse_phs_scripts(content: &str) -> Vec<PhsScript> {
    let mut seen = std::collections::HashSet::new();
    let mut scripts = Vec::new();

    for line in content.lines() {
        let mut search_from = 0;
        while let Some(prefix_start) = line[search_from..].find(PHS_CT_URL_PREFIX) {
            let abs_start = search_from + prefix_start + PHS_CT_URL_PREFIX.len();
            search_from = abs_start;

            let remaining = &line[abs_start..];
            let Some(dot_sh_pos) = remaining.find(".sh") else {
                continue;
            };

            let slug = &remaining[..dot_sh_pos];
            if !is_valid_slug(slug) {
                continue;
            }

            if seen.insert(slug.to_string()) {
                let script_url = format!("{PHS_CT_URL_PREFIX}{slug}.sh");
                scripts.push(PhsScript {
                    slug: slug.to_string(),
                    script_url,
                });
            }
        }
    }

    scripts
}

/// Convert a slug to a human-readable display name.
///
/// Splits on `-`, capitalizes the first letter of each word, and joins with spaces.
pub fn slug_to_display_name(slug: &str) -> String {
    slug.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    format!("{upper}{}", chars.as_str())
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse a version string from the content of a PHS version file.
///
/// Returns `None` if the content is empty or whitespace-only.
pub fn parse_version_file(content: &str) -> Option<&str> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}

/// Validate a package identifier for use as a PHS slug.
///
/// Must be non-empty and match `[a-z0-9][a-z0-9-]*`. This prevents path
/// traversal and injection when constructing file paths from the identifier.
pub fn validate_package_identifier(id: &str) -> uptrakit_provider_core::Result<()> {
    if id.is_empty() {
        bail!(ProviderError::Configuration(
            "package identifier must not be empty".to_string()
        ));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        bail!(ProviderError::Configuration(format!(
            "invalid package identifier '{id}': must match [a-z0-9-]"
        )));
    }
    let first = id.as_bytes()[0];
    if first == b'-' {
        bail!(ProviderError::Configuration(format!(
            "invalid package identifier '{id}': must not start with '-'"
        )));
    }
    Ok(())
}

/// Check whether a slug string is valid: non-empty, only `[a-z0-9-]`,
/// and does not start with `-`.
fn is_valid_slug(slug: &str) -> bool {
    if slug.is_empty() {
        return false;
    }
    if slug.starts_with('-') {
        return false;
    }
    slug.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_phs_scripts ───────────────────────────────────────────

    #[test]
    fn parse_single_script() {
        let content = r#"bash -c "$(curl -fsSL https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/booklore.sh)""#;
        let scripts = parse_phs_scripts(content);
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].slug, "booklore");
        assert_eq!(
            scripts[0].script_url,
            "https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/booklore.sh"
        );
    }

    #[test]
    fn parse_multiple_scripts() {
        let content = r#"
#!/bin/bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/booklore.sh)"
bash -c "$(curl -fsSL https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/crafty-controller.sh)"
"#;
        let scripts = parse_phs_scripts(content);
        assert_eq!(scripts.len(), 2);
        assert_eq!(scripts[0].slug, "booklore");
        assert_eq!(scripts[1].slug, "crafty-controller");
    }

    #[test]
    fn parse_deduplicates_by_slug() {
        let content = r#"
bash -c "$(curl -fsSL https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/booklore.sh)"
bash -c "$(curl -fsSL https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/booklore.sh)"
"#;
        let scripts = parse_phs_scripts(content);
        assert_eq!(scripts.len(), 1);
    }

    #[test]
    fn parse_ignores_invalid_slugs() {
        let content = r#"
bash -c "$(curl -fsSL https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/.sh)"
bash -c "$(curl -fsSL https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/UPPER.sh)"
bash -c "$(curl -fsSL https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/has space.sh)"
bash -c "$(curl -fsSL https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/-leading-dash.sh)"
"#;
        let scripts = parse_phs_scripts(content);
        assert!(scripts.is_empty());
    }

    #[test]
    fn parse_empty_content() {
        let scripts = parse_phs_scripts("");
        assert!(scripts.is_empty());
    }

    #[test]
    fn parse_no_matching_urls() {
        let content = "#!/bin/bash\necho hello\n";
        let scripts = parse_phs_scripts(content);
        assert!(scripts.is_empty());
    }

    #[test]
    fn parse_url_without_sh_extension() {
        let content =
            "https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/booklore";
        let scripts = parse_phs_scripts(content);
        assert!(scripts.is_empty());
    }

    #[test]
    fn parse_slug_with_digits() {
        let content =
            "https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/app2go.sh";
        let scripts = parse_phs_scripts(content);
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].slug, "app2go");
    }

    #[test]
    fn parse_multiple_urls_on_same_line() {
        let content = "https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/foo.sh and https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/bar.sh";
        let scripts = parse_phs_scripts(content);
        assert_eq!(scripts.len(), 2);
        assert_eq!(scripts[0].slug, "foo");
        assert_eq!(scripts[1].slug, "bar");
    }

    // ── slug_to_display_name ────────────────────────────────────────

    #[test]
    fn display_name_single_word() {
        assert_eq!(slug_to_display_name("booklore"), "Booklore");
    }

    #[test]
    fn display_name_hyphenated() {
        assert_eq!(
            slug_to_display_name("crafty-controller"),
            "Crafty Controller"
        );
    }

    #[test]
    fn display_name_with_digits() {
        assert_eq!(slug_to_display_name("app2go"), "App2go");
    }

    #[test]
    fn display_name_empty() {
        assert_eq!(slug_to_display_name(""), "");
    }

    #[test]
    fn display_name_single_char() {
        assert_eq!(slug_to_display_name("a"), "A");
    }

    // ── parse_version_file ──────────────────────────────────────────

    #[test]
    fn version_file_simple() {
        assert_eq!(parse_version_file("1.18.5"), Some("1.18.5"));
    }

    #[test]
    fn version_file_with_newline() {
        assert_eq!(parse_version_file("1.18.5\n"), Some("1.18.5"));
    }

    #[test]
    fn version_file_with_whitespace() {
        assert_eq!(parse_version_file("  1.18.5  \n"), Some("1.18.5"));
    }

    #[test]
    fn version_file_empty() {
        assert_eq!(parse_version_file(""), None);
    }

    #[test]
    fn version_file_whitespace_only() {
        assert_eq!(parse_version_file("   \n  "), None);
    }

    #[test]
    fn version_file_non_semver() {
        assert_eq!(parse_version_file("v2.0-beta"), Some("v2.0-beta"));
    }

    // ── validate_package_identifier ─────────────────────────────────

    #[test]
    fn valid_identifier_simple() {
        assert!(validate_package_identifier("booklore").is_ok());
    }

    #[test]
    fn valid_identifier_with_hyphens() {
        assert!(validate_package_identifier("crafty-controller").is_ok());
    }

    #[test]
    fn valid_identifier_with_digits() {
        assert!(validate_package_identifier("app2go").is_ok());
    }

    #[test]
    fn invalid_identifier_empty() {
        assert!(validate_package_identifier("").is_err());
    }

    #[test]
    fn invalid_identifier_uppercase() {
        assert!(validate_package_identifier("BookLore").is_err());
    }

    #[test]
    fn invalid_identifier_spaces() {
        assert!(validate_package_identifier("crafty controller").is_err());
    }

    #[test]
    fn invalid_identifier_path_traversal() {
        assert!(validate_package_identifier("../etc/passwd").is_err());
    }

    #[test]
    fn invalid_identifier_leading_dash() {
        assert!(validate_package_identifier("-leading").is_err());
    }

    #[test]
    fn valid_identifier_numeric_start() {
        assert!(validate_package_identifier("2fauth").is_ok());
    }

    // ── is_valid_slug ───────────────────────────────────────────────

    #[test]
    fn slug_valid() {
        assert!(is_valid_slug("booklore"));
        assert!(is_valid_slug("crafty-controller"));
        assert!(is_valid_slug("app2go"));
    }

    #[test]
    fn slug_invalid() {
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("-leading"));
        assert!(!is_valid_slug("UPPER"));
        assert!(!is_valid_slug("has space"));
        assert!(!is_valid_slug("path/traversal"));
    }
}
