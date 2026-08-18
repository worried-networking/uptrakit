use async_trait::async_trait;
use futures_util::StreamExt as _;
use rootcause::prelude::*;
use serde::Deserialize;
use uptrakit_plugin_infrastructure_core::{
    BatchFetchItem, BatchFetchResult, PluginError, Result, UpstreamRelease, Version,
};

use crate::error::UvError;
use crate::plugin::UvPlugin;

/// PEP 503 name normalization: lowercase; collapse runs of `.`, `_`, `-` to `-`.
pub fn normalize_pep503(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_sep = false;
    for c in name.chars() {
        if matches!(c, '.' | '_' | '-') {
            if !prev_sep {
                out.push('-');
                prev_sep = true;
            }
        } else {
            out.push(c.to_ascii_lowercase());
            prev_sep = false;
        }
    }
    out
}

/// Derive the version segment from a distribution filename, anchored on the
/// PEP 503-normalized project name — never naive `{name}-{version}` splitting
/// (hyphenated projects make that ambiguous: `zope-interface-4.5.0.tar.gz`).
///
/// The name region is matched under full PEP 503 normalization via an
/// index-tracking walk (so dotted legacy sdists — `zope.interface-5.4.0.tar.gz`
/// under project `zope-interface` — match), while the version region is taken
/// from the **raw** remainder (so the version's own dots survive). The version
/// is the next `-`-delimited raw segment and must start with an ASCII digit.
///
/// Known ceiling: a separator RUN spanning the name/version boundary
/// (`name--1.0.tar.gz`) yields no digit-leading segment and is skipped —
/// such filenames are pathological and per-item tolerance covers them.
pub fn version_from_filename(filename: &str, normalized_name: &str) -> Option<String> {
    let stem = filename
        .strip_suffix(".tar.gz")
        .or_else(|| filename.strip_suffix(".tar.bz2"))
        .or_else(|| filename.strip_suffix(".zip"))
        .or_else(|| filename.strip_suffix(".whl"))
        .or_else(|| filename.strip_suffix(".egg"))?;

    let target = format!("{normalized_name}-");
    let mut norm = String::with_capacity(target.len());
    let mut prev_sep = false;
    let mut rest: Option<&str> = None;
    for (idx, c) in stem.char_indices() {
        if matches!(c, '.' | '_' | '-') {
            if !prev_sep {
                norm.push('-');
                prev_sep = true;
            }
        } else {
            norm.push(c.to_ascii_lowercase());
            prev_sep = false;
        }
        if norm == target {
            rest = stem.get(idx + c.len_utf8()..);
            break;
        }
        if !target.starts_with(norm.as_str()) {
            return None;
        }
    }
    let rest = rest?;
    let version = rest.split('-').next()?;
    version
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| version.to_string())
}

/// Extract the raw version set from a PEP 691 Simple-API JSON project page.
///
/// Prefers the PEP 700 `versions` array; falls back to deriving versions from
/// `files[].filename` for indexes without PEP 700 (devpi, Artifactory, Nexus,
/// GitLab commonly lack it). Wheels and sdists carry the same version string,
/// so dedup makes the wheel-vs-sdist preference moot for the version set.
/// Returns the set sorted+deduped (ordering happens in [`build_releases`]).
pub fn extract_versions(body: &str, normalized_name: &str) -> crate::error::Result<Vec<String>> {
    #[derive(Deserialize)]
    struct SimpleProject {
        #[serde(default)]
        versions: Option<Vec<String>>,
        #[serde(default)]
        files: Vec<SimpleFile>,
    }
    #[derive(Deserialize)]
    struct SimpleFile {
        filename: String,
    }

    let project: SimpleProject = serde_json::from_str(body).map_err(|e| {
        report!(UvError::Request(format!(
            "failed to parse Simple-API response for '{normalized_name}' as JSON \
             (HTML content negotiation?): {e}"
        )))
    })?;

    let mut versions: Vec<String> = match project.versions {
        Some(v) if !v.is_empty() => v,
        _ => project
            .files
            .iter()
            .filter_map(|f| version_from_filename(&f.filename, normalized_name))
            .collect(),
    };
    versions.sort();
    versions.dedup();

    if versions.is_empty() {
        bail!(UvError::EmptyIndex(format!(
            "no versions extractable for '{normalized_name}' from either the PEP 700 \
             versions array or file names"
        )));
    }
    Ok(versions)
}

/// Parse, order, filter, and convert raw version strings into
/// [`UpstreamRelease`] values, **descending by the parsed PEP 440 key**
/// (newest first — the scheduler trusts plugin order verbatim).
///
/// Do NOT re-sort the built vec by the shared `Version`: its string fallback
/// is exactly the mis-ordering (`"1.9" > "1.10"`) pep440_rs exists to fix.
/// Per-version parse failures are skipped item-level. Prereleases
/// (`Version::any_prerelease()` — pre-release OR dev segment) are filtered
/// unless `include_prereleases`; an all-prerelease result is a valid empty
/// list, not an error.
pub fn build_releases(
    raw_versions: Vec<String>,
    include_prereleases: bool,
    project_url: &str,
) -> Vec<UpstreamRelease> {
    let mut parsed: Vec<(pep440_rs::Version, String)> = raw_versions
        .into_iter()
        .filter_map(|s| s.parse::<pep440_rs::Version>().ok().map(|v| (v, s)))
        .collect();
    parsed.sort_by(|a, b| b.0.cmp(&a.0));
    parsed
        .into_iter()
        .filter(|(v, _)| include_prereleases || !v.any_prerelease())
        .map(|(v, s)| UpstreamRelease::new(Version::new(&s), s, v.any_prerelease(), project_url))
        .collect()
}

/// Fetch upstream releases for one project from the PyPI Simple API.
///
/// `GET {index_base}/{normalized_name}/` with
/// `Accept: application/vnd.pypi.simple.v1+json` (PEP 691). `index_base` must
/// already be trailing-slash-trimmed (`UvConfig::effective_index_url`);
/// `package_identifier` must already be validated (`require_package_identifier`)
/// — it is interpolated into the request path.
/// HTTP errors — including 404: tools installed from non-index sources
/// (git/path/URL) are absent from the index and error each cycle, a
/// documented limitation — and non-JSON or zero-version bodies all `bail!`.
/// The body read is unbounded in size (bounded in time by the plugin client's
/// request timeout) — accepted parity with the cargo/npm fetchers.
pub(crate) async fn fetch_uv_releases(
    client: &reqwest::Client,
    index_base: &str,
    include_prereleases: bool,
    package_identifier: &str,
) -> crate::error::Result<Vec<UpstreamRelease>> {
    let normalized = normalize_pep503(package_identifier);
    let url = format!("{index_base}/{normalized}/");
    tracing::debug!(package = %package_identifier, %url, "fetching uv releases from simple index");

    let response = client
        .get(&url)
        .header(
            reqwest::header::ACCEPT,
            "application/vnd.pypi.simple.v1+json",
        )
        .send()
        .await
        .map_err(|e| report!(UvError::Request(e.to_string())))?;

    let status = response.status();
    if !status.is_success() {
        let message = response.text().await.unwrap_or_default();
        bail!(UvError::ApiError { status, message });
    }

    let body = response
        .text()
        .await
        .map_err(|e| report!(UvError::Request(e.to_string())))?;

    let raw_versions = extract_versions(&body, &normalized)?;
    Ok(build_releases(raw_versions, include_prereleases, &url))
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for UvPlugin {
    /// Fetch available releases for a single uv tool (controller-side).
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;

        fetch_uv_releases(
            &self.client,
            self.config.effective_index_url(),
            self.config.include_prereleases,
            package_identifier,
        )
        .await
        .map_err(|e| report!(PluginError::PluginInternal(e.to_string())))
    }

    /// Fetch releases for multiple tools in parallel, bounded to 10 concurrent
    /// requests (mirrors cargo's `buffer_unordered(10)`).
    #[tracing::instrument(skip_all)]
    async fn batch_fetch(&self, items: &[BatchFetchItem]) -> Result<Vec<BatchFetchResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        tracing::debug!(count = items.len(), "batch fetching uv releases");

        let client = self.client.clone();
        let index_base = self.config.effective_index_url().to_string();
        let include_prereleases = self.config.include_prereleases;

        // Per-item identifier validation before any request (mirrors
        // batch_detect): an unvalidated identifier would be interpolated into
        // the request path, and custom indexes run under SsrfMode::Permissive.
        let mut invalid: Vec<BatchFetchResult> = Vec::new();
        let mut ids: Vec<String> = Vec::with_capacity(items.len());
        for item in items {
            match self.require_package_identifier(&item.package_identifier) {
                Ok(()) => ids.push(item.package_identifier.clone()),
                Err(e) => invalid.push(BatchFetchResult::error(
                    item.package_identifier.clone(),
                    e.to_string(),
                )),
            }
        }

        let mut results = futures_util::stream::iter(ids)
            .map(|id| {
                let client = client.clone();
                let index_base = index_base.clone();
                async move {
                    match fetch_uv_releases(&client, &index_base, include_prereleases, &id).await {
                        Ok(releases) => BatchFetchResult::found(id, releases),
                        Err(e) => BatchFetchResult::error(id, e.to_string()),
                    }
                }
            })
            .buffer_unordered(10)
            .collect::<Vec<_>>()
            .await;
        results.extend(invalid);

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::UvError;

    // ── normalize_pep503 ─────────────────────────────────────────────────

    #[test]
    fn normalize_pep503_lowercases_and_collapses_separator_runs() {
        assert_eq!(normalize_pep503("Ruamel.Yaml.Cmd"), "ruamel-yaml-cmd");
        assert_eq!(normalize_pep503("zope.interface"), "zope-interface");
        assert_eq!(normalize_pep503("a__b--c..d"), "a-b-c-d");
        assert_eq!(normalize_pep503("ruff"), "ruff");
    }

    // ── version_from_filename ────────────────────────────────────────────

    #[test]
    fn version_from_filename_wheel() {
        assert_eq!(
            version_from_filename(
                "zope_interface-5.4.0-py2.py3-none-any.whl",
                "zope-interface"
            ),
            Some("5.4.0".to_string())
        );
    }

    #[test]
    fn version_from_filename_sdist() {
        assert_eq!(
            version_from_filename("zope-interface-4.5.0.tar.gz", "zope-interface"),
            Some("4.5.0".to_string())
        );
    }

    /// Dotted legacy (pre-PEP-625) sdist under a hyphenated project: the name
    /// region must match under FULL PEP 503 normalization while the version
    /// region keeps its raw dots.
    #[test]
    fn version_from_filename_dotted_legacy_sdist() {
        assert_eq!(
            version_from_filename("zope.interface-5.4.0.tar.gz", "zope-interface"),
            Some("5.4.0".to_string())
        );
    }

    /// Hyphenated-project ambiguity: for project `zope`, the file
    /// `zope-interface-4.5.0.tar.gz` must NOT yield version `interface-…` —
    /// name-anchored parsing, not naive `{name}-{version}` splitting.
    #[test]
    fn version_from_filename_rejects_wrong_project() {
        assert_eq!(
            version_from_filename("zope-interface-4.5.0.tar.gz", "zope"),
            None
        );
        assert_eq!(
            version_from_filename("requests-2.31.0.tar.gz", "zope-interface"),
            None
        );
    }

    #[test]
    fn version_from_filename_rejects_unknown_extension_and_missing_version() {
        assert_eq!(
            version_from_filename("zope-interface-4.5.0.rpm", "zope-interface"),
            None
        );
        assert_eq!(
            version_from_filename("zope-interface.tar.gz", "zope-interface"),
            None
        );
    }

    // ── extract_versions ─────────────────────────────────────────────────

    #[test]
    fn extract_versions_prefers_pep700_versions_array() {
        let body = r#"{"name":"ruff","versions":["0.6.8","0.6.7"],"files":[]}"#;
        let versions = extract_versions(body, "ruff").unwrap();
        assert_eq!(versions, vec!["0.6.7".to_string(), "0.6.8".to_string()]);
    }

    /// PEP 691 body WITHOUT `versions` (most self-hosted indexes): derive from
    /// filenames — hyphenated project + dotted legacy sdist prove the
    /// name-anchored walk.
    #[test]
    fn extract_versions_filename_fallback() {
        let body = r#"{"name":"zope-interface","files":[
            {"filename":"zope.interface-4.5.0.tar.gz"},
            {"filename":"zope_interface-5.4.0-py2.py3-none-any.whl"},
            {"filename":"zope-interface-5.4.0.tar.gz"}
        ]}"#;
        let versions = extract_versions(body, "zope-interface").unwrap();
        assert_eq!(versions, vec!["4.5.0".to_string(), "5.4.0".to_string()]);
    }

    /// Non-JSON (HTML-negotiated) body ⇒ typed error, never empty success.
    #[test]
    fn extract_versions_html_body_is_error() {
        let Err(err) = extract_versions("<html><body>links…</body></html>", "ruff") else {
            panic!("expected HTML body to fail");
        };
        assert!(matches!(err.current_context(), UvError::Request(_)));
    }

    /// Valid JSON yielding zero versions via both paths ⇒ typed error
    /// (wrong index URL / lossy negotiation), never a silent empty list.
    #[test]
    fn extract_versions_zero_versions_is_error() {
        let Err(err) = extract_versions(r#"{"name":"ruff","files":[]}"#, "ruff") else {
            panic!("expected zero-version body to fail");
        };
        assert!(matches!(err.current_context(), UvError::EmptyIndex(_)));
    }

    // ── build_releases ───────────────────────────────────────────────────

    fn raw(versions: &[&str]) -> Vec<String> {
        versions.iter().map(|s| s.to_string()).collect()
    }

    /// Descending PEP 440 order — including the exact cases the shared
    /// `Version`'s string fallback mis-sorts (`1.9` vs `1.10`, `.post1`).
    #[test]
    fn build_releases_descending_pep440_order() {
        let releases = build_releases(
            raw(&["1.9", "1.10", "1.2.3", "1.2.3.post1"]),
            false,
            "https://pypi.org/simple/x/",
        );
        let order: Vec<&str> = releases.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(order, vec!["1.10", "1.9", "1.2.3.post1", "1.2.3"]);
    }

    /// Prerelease filtering on no-hyphen PEP 440 forms — cargo's
    /// `contains('-')` heuristic would miss every one of these.
    #[test]
    fn build_releases_filters_pep440_prereleases() {
        let releases = build_releases(
            raw(&["1.2.3", "1.2.3rc1", "1.0a1", "2.0.0.dev1"]),
            false,
            "u",
        );
        let tags: Vec<&str> = releases.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(tags, vec!["1.2.3"]);
    }

    #[test]
    fn build_releases_include_prereleases_keeps_and_marks() {
        let releases = build_releases(raw(&["1.2.3rc1", "1.2.3"]), true, "u");
        assert_eq!(releases.len(), 2);
        let rc = releases.iter().find(|r| r.tag == "1.2.3rc1").unwrap();
        assert!(rc.is_prerelease);
        let stable = releases.iter().find(|r| r.tag == "1.2.3").unwrap();
        assert!(!stable.is_prerelease);
    }

    /// All-prerelease package under include_prereleases=false ⇒ Ok(empty) —
    /// the fetch succeeded; there is legitimately no stable Release.
    #[test]
    fn build_releases_all_prerelease_is_empty_not_error() {
        assert!(build_releases(raw(&["1.0a1", "1.0b2"]), false, "u").is_empty());
    }

    /// Unparseable version strings are skipped item-level (homebrew-style
    /// tolerance), not fatal.
    #[test]
    fn build_releases_skips_unparseable_versions() {
        let releases = build_releases(raw(&["not-a-version", "1.2.3"]), false, "u");
        let tags: Vec<&str> = releases.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(tags, vec!["1.2.3"]);
    }

    // ── batch_fetch ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_fetch_rejects_invalid_identifiers_item_level() {
        use uptrakit_plugin_infrastructure_core::ReleaseFetcher as _;
        use uptrakit_plugin_infrastructure_core::testing::{
            FixedOutputExecutor, test_runtime_with_executor,
        };

        let plugin = crate::plugin::UvPlugin::new(
            crate::config::UvConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::success("")),
        )
        .expect("construct plugin");

        let items = vec![
            BatchFetchItem::new("owner/pkg"),
            BatchFetchItem::new("pkg name"),
        ];
        let results = plugin.batch_fetch(&items).await.expect("batch fetch");
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.error.is_some()));
    }
}
