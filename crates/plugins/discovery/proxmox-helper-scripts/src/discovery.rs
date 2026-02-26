use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::PluginError;

/// Well-known path where PHS containers store their update script.
pub const UPDATE_SCRIPT_PATH: &str = "/usr/bin/update";

/// Base URL prefix for PHS community-scripts CT scripts (canonical form).
const PHS_CT_URL_PREFIX: &str =
    "https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/";

/// Alternative GitHub URL prefix for PHS community-scripts CT scripts.
///
/// Some `/usr/bin/update` scripts use the `github.com/…/raw/…` redirect form
/// instead of `raw.githubusercontent.com`. Both URLs serve identical content;
/// this prefix is recognised during detection and the resulting slug is always
/// fetched via the canonical [`PHS_CT_URL_PREFIX`] URL.
const PHS_CT_URL_PREFIX_ALT: &str =
    "https://github.com/community-scripts/ProxmoxVE/raw/main/ct/";

/// Base URL prefix for PHS community-scripts install scripts.
pub const PHS_INSTALL_URL_PREFIX: &str =
    "https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/install/";

/// Common system-level APT packages that are never a PHS "main application".
///
/// Used by [`extract_apt_package_candidates`] to filter out infrastructure
/// dependencies when extracting candidate package names from install scripts.
pub const SYSTEM_APT_PACKAGES: &[&str] = &[
    "build-essential",
    "ca-certificates",
    "curl",
    "git",
    "gnupg",
    "gnupg2",
    "graphicsmagick",
    "imagemagick",
    "jq",
    "libssl-dev",
    "lsb-release",
    "nodejs",
    "npm",
    "openssl",
    "python3",
    "python3-dev",
    "python3-pip",
    "python3-setuptools",
    "software-properties-common",
    "sudo",
    "unzip",
    "wget",
    "zip",
    "apt-transport-https",
    "apt-utils",
];

/// Result of analysing a PHS CT script for upstream source type.
///
/// Exactly one of `github_owner`+`github_repo` or `apt_package` will be set
/// for a successfully-classified script; both being `None` means the script
/// could not be classified (e.g. update via `npm`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhsScriptAnalysis {
    /// GitHub repository owner, if the app is GitHub-managed.
    pub github_owner: Option<String>,
    /// GitHub repository name, if the app is GitHub-managed.
    pub github_repo: Option<String>,
    /// Debian package name, if the app is APT-managed.
    pub apt_package: Option<String>,
    /// Human-readable application name extracted from `APP=` assignment.
    pub app_name: Option<String>,
    /// Version file basename when it differs from the container slug.
    ///
    /// PHS stores the installed version in `/root/.<key>` where `<key>` is the
    /// first argument passed to `check_for_gh_release`. When this key differs
    /// from the container slug (the `.sh` filename without extension), this
    /// field holds the key so the version helper script is invoked with the
    /// correct argument.
    ///
    /// Example: `paperless-ngx.sh` calls
    /// `check_for_gh_release "paperless" "paperless-ngx/paperless-ngx"`, so
    /// `version_file_basename = Some("paperless")` and the installed version
    /// lives in `/root/.paperless`, not `/root/.paperless-ngx`.
    ///
    /// When `None`, the container slug is used as the version file basename.
    pub version_file_basename: Option<String>,
}

/// A discovered PHS script reference extracted from the update script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhsScript {
    /// The slug extracted from the script URL (e.g. `booklore`, `crafty-controller`).
    pub slug: String,
    /// The full URL to the community-scripts CT script.
    pub script_url: String,
}

// ── Script analysis ───────────────────────────────────────────────────────────

/// Analyse the content of a PHS CT script to determine the upstream source type.
///
/// Detection priority:
/// 1. `check_for_gh_release` / `fetch_and_deploy_gh_release` calls, preferring the
///    call whose first key argument **slug-matches** `slug` (case-insensitive, with
///    optional hyphen normalisation).
/// 2. Bare `GH_REPO="owner/repo"` or `GH_REPO='owner/repo'` variable assignment.
/// 3. First `check_for_gh_release` / `fetch_and_deploy_gh_release` match, ignoring key.
/// 4. APT `install` with a non-system package name (only when GitHub detection yields nothing).
///
/// `app_name` is always extracted from the first `APP="…"` or `APP='…'` line.
pub fn analyze_phs_script(slug: &str, content: &str) -> PhsScriptAnalysis {
    let app_name = extract_app_name(content);

    // ── GitHub detection ──────────────────────────────────────────────────────
    // Collect all (key, owner, repo) triples from priority-1 patterns.
    let p1_matches = collect_gh_release_calls(content);

    if !p1_matches.is_empty() {
        // Prefer the first call whose key slug-matches the container slug.
        let best = p1_matches
            .iter()
            .find(|(key, _, _)| slug_matches(key, slug))
            .or_else(|| p1_matches.first());

        if let Some((best_key, owner, repo)) = best
            && is_valid_gh_component(owner)
            && is_valid_gh_component(repo)
        {
            return PhsScriptAnalysis {
                github_owner: Some(owner.clone()),
                github_repo: Some(repo.clone()),
                apt_package: None,
                app_name,
                version_file_basename: derive_version_file_basename(best_key, slug),
            };
        }
    }

    // Priority 2: bare GH_REPO= assignment.
    if let Some((owner, repo)) = extract_gh_repo_var(content)
        && is_valid_gh_component(&owner)
        && is_valid_gh_component(&repo)
    {
        return PhsScriptAnalysis {
            github_owner: Some(owner),
            github_repo: Some(repo),
            apt_package: None,
            app_name,
            version_file_basename: None,
        };
    }

    // ── APT detection (only when no GitHub upstream found) ───────────────────
    PhsScriptAnalysis {
        github_owner: None,
        github_repo: None,
        apt_package: extract_apt_package(content),
        app_name,
        version_file_basename: None,
    }
}

/// Extract APT package candidates from an install script.
///
/// Used as the install-script fallback when the CT script only has `apt upgrade -y`
/// and no specific `apt install <package>` line.  Returns all non-system packages
/// mentioned in `apt install` lines, deduplicated in first-seen order.
pub fn extract_apt_package_candidates(content: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut candidates = Vec::new();

    for pkg in apt_install_packages(content) {
        if SYSTEM_APT_PACKAGES.contains(&pkg.as_str()) {
            continue;
        }
        if !is_valid_deb_package(&pkg) {
            continue;
        }
        if seen.insert(pkg.clone()) {
            candidates.push(pkg);
        }
    }

    candidates
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Derive the version file basename from a `check_for_gh_release` key.
///
/// PHS writes the installed version to `/root/.<key>` where `<key>` is the
/// first argument to `check_for_gh_release`. When that key is a valid PHS slug
/// (`[a-z0-9][a-z0-9-]*`) and differs from the container slug, we return
/// `Some(key)` so callers can read the correct file.
///
/// Returns `None` when:
/// - The key equals the slug (no override needed).
/// - The key contains uppercase or other characters outside `[a-z0-9-]`
///   (we cannot safely map those to a predictable version file path).
fn derive_version_file_basename(key: &str, slug: &str) -> Option<String> {
    if is_valid_slug(key) && key != slug {
        Some(key.to_string())
    } else {
        None
    }
}

/// Extract all `apt install <pkg>` package names from `content`.
///
/// Handles both `apt install` and `apt-get install`, optional flags such as
/// `--only-upgrade`, `-y`, and `--no-install-recommends`.
fn apt_install_packages(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        // Match `apt install` or `apt-get install` with optional flags/options.
        // We look for the literal word "install" after "apt" or "apt-get",
        // then collect the first non-flag token.
        if let Some(pkg) = parse_apt_install_line(line) {
            packages.push(pkg);
        }
    }
    packages
}

/// Parse a single shell line for `apt[-get] [opts] install [opts] <pkg>`.
///
/// Returns the first non-flag argument after `install`, or `None` if the line
/// does not match.
fn parse_apt_install_line(line: &str) -> Option<String> {
    // Strip variable prefixes like `$STD `, `DEBIAN_FRONTEND=noninteractive `, etc.
    let stripped = strip_shell_prefixes(line);

    // Must start with apt or apt-get.
    let after_apt = if let Some(rest) = stripped.strip_prefix("apt-get") {
        rest
    } else if let Some(rest) = stripped.strip_prefix("apt") {
        rest
    } else {
        return None;
    };

    // Must be a word boundary (space, end of string, or nothing else).
    if !after_apt.is_empty() && !after_apt.starts_with([' ', '\t']) {
        return None;
    }

    // Walk through tokens looking for "install".
    let mut tokens = after_apt.split_whitespace();
    let mut found_install = false;
    let mut pkg: Option<String> = None;

    for tok in &mut tokens {
        if found_install {
            if tok.starts_with('-') {
                // Flag — skip (handles `--option=value` forms too).
                continue;
            }
            // First non-flag token after install is the package name.
            pkg = Some(tok.to_string());
            break;
        } else if tok == "install" {
            found_install = true;
        }
        // Other tokens before install are flags/env vars — just skip.
    }

    pkg
}

/// Strip common shell variable/function prefixes from a line.
///
/// Handles patterns like `$STD `, `$SUDO `, `DEBIAN_FRONTEND=... `, etc.
fn strip_shell_prefixes(line: &str) -> &str {
    let mut s = line;
    loop {
        let prev = s;
        // Strip leading `$VARNAME` tokens (e.g. `$STD`, `$SUDO`).
        if let Some(rest) = s.strip_prefix('$') {
            let end = rest.find([' ', '\t']).map(|i| i + 1).unwrap_or(rest.len());
            s = rest[end..].trim_start();
        }
        // Strip `KEY=VALUE` env assignments.
        else if let Some(pos) = s.find('=') {
            let key = &s[..pos];
            if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                let rest = s[pos + 1..].trim_start_matches(|c: char| c != ' ' && c != '\t');
                s = rest.trim_start();
            } else {
                break;
            }
        } else {
            break;
        }
        if s == prev {
            break;
        }
    }
    s
}

/// Collect all `(key, owner, repo)` triples from `check_for_gh_release` and
/// `fetch_and_deploy_gh_release` calls in `content`.
fn collect_gh_release_calls(content: &str) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        for prefix in &["check_for_gh_release", "fetch_and_deploy_gh_release"] {
            if let Some(rest) = line.find(prefix).map(|i| &line[i + prefix.len()..])
                && let Some((key, owner, repo)) = parse_gh_call_args(rest)
            {
                results.push((key, owner, repo));
            }
        }
    }
    results
}

/// Parse the argument list `"key" "owner/repo"` from the remainder of a
/// `check_for_gh_release` or `fetch_and_deploy_gh_release` call.
///
/// Returns `(key, owner, repo)` on success.
fn parse_gh_call_args(rest: &str) -> Option<(String, String, String)> {
    let rest = rest.trim();

    // First quoted argument: the key.
    let (key, after_key) = extract_quoted_arg(rest)?;

    // Second quoted argument: the repo in `owner/repo` form.
    let (repo_str, _) = extract_quoted_arg(after_key.trim())?;

    let slash = repo_str.find('/')?;
    let owner = repo_str[..slash].to_string();
    let repo = repo_str[slash + 1..].to_string();

    Some((key, owner, repo))
}

/// Extract the content of a single-quoted or double-quoted string from the
/// start of `s`.  Returns `(content, remainder_after_closing_quote)`.
fn extract_quoted_arg(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    let quote = s.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &s[1..];
    let close = rest.find(quote)?;
    Some((rest[..close].to_string(), &rest[close + 1..]))
}

/// Extract the `owner`/`repo` pair from a `GH_REPO="owner/repo"` or
/// `GH_REPO='owner/repo'` assignment.
fn extract_gh_repo_var(content: &str) -> Option<(String, String)> {
    for line in content.lines() {
        let line = line.trim();
        let rest = if let Some(r) = line.strip_prefix("GH_REPO=") {
            r
        } else {
            continue;
        };
        let (repo_str, _) = extract_quoted_arg(rest)?;
        let slash = repo_str.find('/')?;
        let owner = repo_str[..slash].to_string();
        let repo = repo_str[slash + 1..].to_string();
        return Some((owner, repo));
    }
    None
}

/// Extract the APT package name from the first `apt install <pkg>` line.
fn extract_apt_package(content: &str) -> Option<String> {
    apt_install_packages(content)
        .into_iter()
        .find(|pkg| is_valid_deb_package(pkg))
}

/// Extract the value of `APP="…"` or `APP='…'` from content.
fn extract_app_name(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("APP=")
            && let Some((name, _)) = extract_quoted_arg(rest)
            && !name.is_empty()
        {
            return Some(name);
        }
    }
    None
}

/// Check whether `key` slug-matches `slug` using three increasingly loose
/// comparison strategies.
fn slug_matches(key: &str, slug: &str) -> bool {
    // a. Exact case-insensitive match.
    if key.eq_ignore_ascii_case(slug) {
        return true;
    }
    // b. Strip hyphens from key, compare to slug.
    let key_no_hyphens = key.replace('-', "");
    if key_no_hyphens.eq_ignore_ascii_case(slug) {
        return true;
    }
    // c. Strip hyphens from slug, compare to key.
    let slug_no_hyphens = slug.replace('-', "");
    if slug_no_hyphens.eq_ignore_ascii_case(key) {
        return true;
    }
    false
}

/// Validate a GitHub `owner` or `repo` component: must be non-empty and must
/// not contain `/` or `..` (path traversal guards).
fn is_valid_gh_component(s: &str) -> bool {
    !s.is_empty() && !s.contains('/') && !s.contains("..")
}

/// Validate a Debian package name: `^[a-z0-9][a-z0-9+.-]+$`.
fn is_valid_deb_package(pkg: &str) -> bool {
    if pkg.len() < 2 {
        return false;
    }
    let mut chars = pkg.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '+' || c == '.' || c == '-')
}

// ── Existing helper functions (unchanged) ─────────────────────────────────────

/// Parse PHS script references from the content of `/usr/bin/update`.
///
/// Scans each line for occurrences of the community-scripts CT URL prefix in
/// both known forms (`raw.githubusercontent.com` and `github.com/…/raw/…`),
/// extracts the slug from the URL path, validates it, and deduplicates by slug.
/// The `script_url` in every returned [`PhsScript`] always uses the canonical
/// `raw.githubusercontent.com` form regardless of which prefix was matched.
pub fn parse_phs_scripts(content: &str) -> Vec<PhsScript> {
    let mut seen = std::collections::HashSet::new();
    let mut scripts = Vec::new();

    const PREFIXES: &[&str] = &[PHS_CT_URL_PREFIX, PHS_CT_URL_PREFIX_ALT];

    for line in content.lines() {
        for &prefix in PREFIXES {
            let mut search_from = 0;
            while let Some(prefix_start) = line[search_from..].find(prefix) {
                let abs_start = search_from + prefix_start + prefix.len();
                search_from = abs_start;

                let remaining = &line[abs_start..];
                let Some(dot_sh_pos) = remaining.find(".sh") else {
                    continue;
                };

                let slug = &remaining[..dot_sh_pos];
                if !is_valid_slug(slug) {
                    tracing::trace!(slug, "skipping invalid PHS slug");
                    continue;
                }

                if seen.insert(slug.to_string()) {
                    // Always normalise to the canonical raw.githubusercontent.com URL.
                    let script_url = format!("{PHS_CT_URL_PREFIX}{slug}.sh");
                    scripts.push(PhsScript {
                        slug: slug.to_string(),
                        script_url,
                    });
                }
            }
        }
    }

    tracing::debug!(count = scripts.len(), "parsed PHS scripts from update file");
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
pub fn validate_package_identifier(id: &str) -> uptrakit_plugin_infrastructure_core::Result<()> {
    if id.is_empty() {
        tracing::debug!(identifier = %id, "invalid PHS package identifier");
        bail!(PluginError::Configuration(
            "package identifier must not be empty".to_string()
        ));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        tracing::debug!(identifier = %id, "invalid PHS package identifier");
        bail!(PluginError::Configuration(format!(
            "invalid package identifier '{id}': must match [a-z0-9-]"
        )));
    }
    let first = id.as_bytes()[0];
    if first == b'-' {
        tracing::debug!(identifier = %id, "invalid PHS package identifier");
        bail!(PluginError::Configuration(format!(
            "invalid package identifier '{id}': must not start with '-'"
        )));
    }
    Ok(())
}

/// Check whether a slug string is valid: non-empty, only `[a-z0-9-]`,
/// and does not start with `-`.
pub(crate) fn is_valid_slug(slug: &str) -> bool {
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

    // ── analyze_phs_script ──────────────────────────────────────────

    #[test]
    fn analyze_booklore_style_single_repo_slug_match() {
        // Slug matches key directly (case-insensitive).
        let content = r#"
APP="BookLore"
check_for_gh_release "booklore" "BookLore/BookLore"
"#;
        let result = analyze_phs_script("booklore", content);
        assert_eq!(result.github_owner.as_deref(), Some("BookLore"));
        assert_eq!(result.github_repo.as_deref(), Some("BookLore"));
        assert!(result.apt_package.is_none());
        assert_eq!(result.app_name.as_deref(), Some("BookLore"));
    }

    #[test]
    fn analyze_case_insensitive_key_match() {
        // Key "Radarr" matches slug "radarr" via eq_ignore_ascii_case.
        let content = r#"check_for_gh_release "Radarr" "Radarr/Radarr""#;
        let result = analyze_phs_script("radarr", content);
        assert_eq!(result.github_owner.as_deref(), Some("Radarr"));
        assert_eq!(result.github_repo.as_deref(), Some("Radarr"));
    }

    #[test]
    fn analyze_hyphen_strip_key_match() {
        // Key "uptime-kuma" → strip hyphens → "uptimekuma" == slug "uptimekuma".
        let content = r#"check_for_gh_release "uptime-kuma" "louislam/uptime-kuma""#;
        let result = analyze_phs_script("uptimekuma", content);
        assert_eq!(result.github_owner.as_deref(), Some("louislam"));
        assert_eq!(result.github_repo.as_deref(), Some("uptime-kuma"));
    }

    #[test]
    fn analyze_pangolin_style_multi_repo_slug_match() {
        // Multi-component: three repos; slug "pangolin" matches the first.
        let content = r#"
check_for_gh_release "pangolin" "fosrl/pangolin"
check_for_gh_release "gerbil" "fosrl/gerbil"
check_for_gh_release "badger" "fosrl/badger"
"#;
        let result = analyze_phs_script("pangolin", content);
        assert_eq!(result.github_owner.as_deref(), Some("fosrl"));
        assert_eq!(result.github_repo.as_deref(), Some("pangolin"));
    }

    #[test]
    fn analyze_adguard_fetch_and_deploy() {
        // fetch_and_deploy_gh_release call; key "AdGuardHome" with hyphen-strip
        // comparison to slug "adguard" should NOT match (lengths differ too much),
        // so we fall back to the first match.
        let content = r#"
fetch_and_deploy_gh_release "AdGuardHome" "AdGuardTeam/AdGuardHome"
"#;
        let result = analyze_phs_script("adguard", content);
        // No key matches slug "adguard", so fallback to first match.
        assert_eq!(result.github_owner.as_deref(), Some("AdGuardTeam"));
        assert_eq!(result.github_repo.as_deref(), Some("AdGuardHome"));
        // Key "AdGuardHome" has uppercase → not a valid slug → no version file
        // override; the agent will fall back to reading /root/.adguard.
        assert!(result.version_file_basename.is_none());
    }

    #[test]
    fn analyze_paperless_ngx_version_file_override() {
        // Paperless-ngx: slug is "paperless-ngx" but the check_for_gh_release
        // key is "paperless", so the installed version lives in /root/.paperless.
        let content = r#"
APP="Paperless-ngx"
check_for_gh_release "paperless" "paperless-ngx/paperless-ngx"
"#;
        let result = analyze_phs_script("paperless-ngx", content);
        assert_eq!(result.github_owner.as_deref(), Some("paperless-ngx"));
        assert_eq!(result.github_repo.as_deref(), Some("paperless-ngx"));
        assert_eq!(result.app_name.as_deref(), Some("Paperless-ngx"));
        // Key "paperless" is a valid lowercase slug and differs from slug
        // "paperless-ngx", so the version file basename must be set.
        assert_eq!(result.version_file_basename.as_deref(), Some("paperless"));
    }

    #[test]
    fn analyze_version_file_same_as_slug_no_override() {
        // When the key exactly matches the slug, no override is needed.
        let content = r#"check_for_gh_release "booklore" "BookLore/BookLore""#;
        let result = analyze_phs_script("booklore", content);
        assert_eq!(result.github_owner.as_deref(), Some("BookLore"));
        assert!(result.version_file_basename.is_none());
    }

    #[test]
    fn analyze_gh_repo_var_no_version_file_override() {
        // The GH_REPO= path has no explicit key; version_file_basename is None.
        let content = r#"GH_REPO="someorg/someapp""#;
        let result = analyze_phs_script("someapp", content);
        assert_eq!(result.github_owner.as_deref(), Some("someorg"));
        assert!(result.version_file_basename.is_none());
    }

    #[test]
    fn analyze_gh_repo_var_fallback() {
        // GH_REPO= assignment used when no check_for_gh_release calls exist.
        let content = r#"
GH_REPO="someorg/someapp"
apt upgrade -y
"#;
        let result = analyze_phs_script("someapp", content);
        assert_eq!(result.github_owner.as_deref(), Some("someorg"));
        assert_eq!(result.github_repo.as_deref(), Some("someapp"));
        assert!(result.apt_package.is_none());
    }

    #[test]
    fn analyze_gh_repo_var_single_quotes() {
        let content = "GH_REPO='owner/repo'\n";
        let result = analyze_phs_script("repo", content);
        assert_eq!(result.github_owner.as_deref(), Some("owner"));
        assert_eq!(result.github_repo.as_deref(), Some("repo"));
    }

    #[test]
    fn analyze_grafana_apt_direct() {
        // APT direct: `apt --only-upgrade install -y grafana`.
        let content = r#"
APP="Grafana"
$STD apt-get update
$STD apt --only-upgrade install -y grafana
"#;
        let result = analyze_phs_script("grafana", content);
        assert!(result.github_owner.is_none());
        assert!(result.github_repo.is_none());
        assert_eq!(result.apt_package.as_deref(), Some("grafana"));
        assert_eq!(result.app_name.as_deref(), Some("Grafana"));
    }

    #[test]
    fn analyze_plex_apt_direct() {
        // APT direct: `apt install -y plexmediaserver`.
        let content = r#"$STD apt install -y plexmediaserver"#;
        let result = analyze_phs_script("plex", content);
        assert!(result.github_owner.is_none());
        assert_eq!(result.apt_package.as_deref(), Some("plexmediaserver"));
    }

    #[test]
    fn analyze_influxdb_apt_upgrade_only_yields_neither() {
        // Only `apt upgrade -y` — no `apt install <pkg>` → both None.
        let content = r#"
apt-get update
apt upgrade -y
"#;
        let result = analyze_phs_script("influxdb", content);
        assert!(result.github_owner.is_none());
        assert!(result.apt_package.is_none());
    }

    #[test]
    fn analyze_github_preferred_over_apt() {
        // If GitHub patterns exist, APT lines should be ignored.
        let content = r#"
check_for_gh_release "myapp" "myorg/myapp"
apt install -y somepackage
"#;
        let result = analyze_phs_script("myapp", content);
        assert_eq!(result.github_owner.as_deref(), Some("myorg"));
        assert!(result.apt_package.is_none());
    }

    #[test]
    fn analyze_rejects_traversal_in_owner() {
        let content = r#"GH_REPO="../bad/repo""#;
        let result = analyze_phs_script("repo", content);
        // ".." in owner is rejected; no GitHub detection.
        assert!(result.github_owner.is_none());
    }

    #[test]
    fn analyze_rejects_slash_in_repo() {
        // owner/repo/extra — repo contains a slash → rejected.
        let content = r#"GH_REPO="owner/repo/extra""#;
        let result = analyze_phs_script("extra", content);
        assert!(result.github_owner.is_none());
    }

    // ── extract_apt_package_candidates ─────────────────────────────

    #[test]
    fn candidates_influxdb_install_script() {
        // Simulates an influxdb install script with conditional variants.
        let content = r#"
apt-get install -y influxdb
apt-get install -y influxdb2
apt-get install -y influxdb3-core
apt-get install -y curl
"#;
        let result = extract_apt_package_candidates(content);
        assert!(result.contains(&"influxdb".to_string()));
        assert!(result.contains(&"influxdb2".to_string()));
        assert!(result.contains(&"influxdb3-core".to_string()));
        // curl is in SYSTEM_APT_PACKAGES → filtered out.
        assert!(!result.contains(&"curl".to_string()));
    }

    #[test]
    fn candidates_n8n_all_system_packages() {
        // All packages are system deps → returns empty.
        let content = r#"
apt-get install -y curl wget gnupg build-essential nodejs npm git
"#;
        let result = extract_apt_package_candidates(content);
        assert!(result.is_empty());
    }

    #[test]
    fn candidates_deduplicates() {
        let content = r#"
apt install -y myapp
apt install -y myapp
"#;
        let result = extract_apt_package_candidates(content);
        assert_eq!(result, vec!["myapp".to_string()]);
    }

    #[test]
    fn candidates_ignores_apt_upgrade() {
        // `apt upgrade -y` has no package name after install; not matched.
        let content = "apt upgrade -y\napt-get upgrade -y\n";
        let result = extract_apt_package_candidates(content);
        assert!(result.is_empty());
    }

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

    #[test]
    fn parse_alt_url_prefix() {
        // github.com/…/raw/… form is detected and normalised to the canonical URL.
        let content = r#"bash -c "$(curl -fsSL https://github.com/community-scripts/ProxmoxVE/raw/main/ct/booklore.sh)""#;
        let scripts = parse_phs_scripts(content);
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].slug, "booklore");
        assert_eq!(
            scripts[0].script_url,
            "https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/booklore.sh"
        );
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
