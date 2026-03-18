use std::collections::HashMap;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{
    BatchFetchItem, BatchFetchResult, PluginError, Result, UpdateCategory, UpstreamRelease,
    Version, execute_and_capture,
};

use crate::plugin::{AptPlugin, MadisonEntry, validate_identifier};

impl AptPlugin {
    /// Parse `apt-cache madison <package>` output.
    ///
    /// Each line has the format:
    /// `   <package> | <version> | <source>`
    ///
    /// Returns the version and source from the first valid line
    /// (highest-priority candidate), or `None` if the output is empty or
    /// contains no parseable lines.
    pub(crate) fn parse_madison_output(output: &str) -> Option<MadisonEntry> {
        output.lines().find_map(|line| {
            let mut parts = line.splitn(3, '|');
            let _ = parts.next()?;
            let version = parts.next()?.trim();
            if version.is_empty() {
                return None;
            }
            let source = parts
                .next()
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            Some(MadisonEntry {
                version: version.to_string(),
                source,
            })
        })
    }

    /// Detect whether a madison source string indicates a security repository.
    ///
    /// APT security updates typically come from URLs containing "security"
    /// (e.g. `http://security.ubuntu.com/ubuntu noble-security/main`).
    pub(crate) fn is_security_source(source: &str) -> bool {
        source.to_ascii_lowercase().contains("security")
    }

    /// Parse `apt-cache madison pkg1 pkg2 ...` output for a batch query.
    ///
    /// Lines from a multi-package madison query are interleaved:
    /// ```text
    ///    nginx | 1.24.0 | http://archive.ubuntu.com/ubuntu noble/main amd64 Packages
    ///    curl  | 7.88.1 | http://deb.debian.org/debian bookworm/main amd64 Packages
    ///    nginx | 1.18.0 | http://archive.ubuntu.com/ubuntu focal/main amd64 Packages
    /// ```
    ///
    /// Groups lines by package name (first `|`-delimited field). For each
    /// package, only the *first* line is used (highest-priority candidate;
    /// madison output is already ordered by pin priority).
    pub(crate) fn parse_madison_output_batch(output: &str) -> HashMap<String, MadisonEntry> {
        let mut results: HashMap<String, MadisonEntry> = HashMap::new();
        for line in output.lines() {
            let mut parts = line.splitn(3, '|');
            let Some(pkg_name) = parts.next() else {
                continue;
            };
            let pkg_name = pkg_name.trim().to_string();
            if pkg_name.is_empty() {
                continue;
            }
            let Some(version) = parts.next() else {
                continue;
            };
            let version = version.trim();
            if version.is_empty() {
                continue;
            }
            let source = parts
                .next()
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            // Only keep the first entry per package (highest priority).
            results.entry(pkg_name).or_insert_with(|| MadisonEntry {
                version: version.to_string(),
                source,
            });
        }
        results
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for AptPlugin {
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching APT releases via apt-cache madison");

        let stdout = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec(
                "apt-cache",
                ["madison".to_string(), package_identifier.to_string()],
            ),
            "apt-cache madison",
        )
        .await?;

        let Some(entry) = Self::parse_madison_output(&stdout) else {
            // Package not found in any configured repository.
            return Ok(vec![]);
        };

        let category = if Self::is_security_source(&entry.source) {
            Some(UpdateCategory::Security)
        } else {
            None
        };

        tracing::debug!(
            version = %entry.version,
            ?category,
            source = %entry.source,
            "APT upstream version resolved"
        );
        Ok(vec![{
            let mut release =
                UpstreamRelease::new(Version::new(&entry.version), entry.version, false, "");
            release.category = category;
            release
        }])
    }

    /// Fetch available releases for multiple packages using a single `apt-cache madison` call.
    ///
    /// Runs:
    /// ```text
    /// apt-cache madison pkg1 pkg2 pkg3
    /// ```
    ///
    /// Output lines are grouped by package name; only the first (highest-priority)
    /// entry per package is used. Packages absent from the output have empty releases.
    #[tracing::instrument(skip_all)]
    async fn batch_fetch(&self, items: &[BatchFetchItem]) -> Result<Vec<BatchFetchResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec!["madison".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(
            count = items.len(),
            "batch fetching APT releases via apt-cache madison"
        );

        let stdout = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec("apt-cache", args),
            "apt-cache madison",
        )
        .await?;

        let parsed = Self::parse_madison_output_batch(&stdout);

        let results = items
            .iter()
            .map(|item| {
                let Some(entry) = parsed.get(&item.package_identifier) else {
                    // Package not found in any configured repository.
                    return BatchFetchResult::empty(item.package_identifier.clone());
                };

                let category = if Self::is_security_source(&entry.source) {
                    Some(UpdateCategory::Security)
                } else {
                    None
                };

                let release = {
                    let mut r = UpstreamRelease::new(
                        Version::new(&entry.version),
                        entry.version.clone(),
                        false,
                        "",
                    );
                    r.category = category;
                    r
                };
                BatchFetchResult::found(item.package_identifier.clone(), vec![release])
            })
            .collect();

        tracing::debug!(count = items.len(), "APT batch fetch complete");
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
    use uptrakit_plugin_infrastructure_core::testing::RoutedOutputExecutor;
    use uptrakit_plugin_infrastructure_core::{
        BatchFetchItem, HostCapabilities, HostRuntime, LocalCommandExecutor, PosixHostRuntime,
        ReleaseFetcher, UpdateCategory,
    };

    use crate::config::AptConfig;

    /// Helper to create an `AptPlugin` from a mock executor for testing.
    fn test_plugin_with_executor(
        config: AptConfig,
        executor: Arc<dyn CommandExecutor>,
    ) -> AptPlugin {
        let caps = HostCapabilities::default();
        let runtime = Arc::new(PosixHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        AptPlugin::new(config, runtime).unwrap()
    }

    // ── parse_madison_output ────────────────────────────────────────────

    #[test]
    fn parse_madison_output_single_entry() {
        let output = "   nginx | 1.24.0-2ubuntu7.3 | http://archive.ubuntu.com/ubuntu noble-updates/main amd64 Packages\n";
        let entry = AptPlugin::parse_madison_output(output).unwrap();
        assert_eq!(entry.version, "1.24.0-2ubuntu7.3");
        assert!(entry.source.contains("archive.ubuntu.com"));
    }

    #[test]
    fn parse_madison_output_multiple_entries_returns_first() {
        let output = concat!(
            "   nginx | 1.24.0-2ubuntu7.3 | http://archive.ubuntu.com/ubuntu noble-updates/main amd64 Packages\n",
            "   nginx | 1.18.0-6ubuntu14 | http://archive.ubuntu.com/ubuntu focal/main amd64 Packages\n",
        );
        let entry = AptPlugin::parse_madison_output(output).unwrap();
        assert_eq!(entry.version, "1.24.0-2ubuntu7.3");
    }

    #[test]
    fn parse_madison_output_malformed_line_skipped_gracefully() {
        let output = concat!("no pipe here\n", "   nginx | 1.24.0 | source\n",);
        let entry = AptPlugin::parse_madison_output(output).unwrap();
        assert_eq!(entry.version, "1.24.0");
    }

    #[test]
    fn parse_madison_output_empty() {
        assert!(AptPlugin::parse_madison_output("").is_none());
    }

    #[test]
    fn parse_madison_output_security_source() {
        let output = "   openssl | 3.0.2-0ubuntu1.16 | http://security.ubuntu.com/ubuntu noble-security/main amd64 Packages\n";
        let entry = AptPlugin::parse_madison_output(output).unwrap();
        assert_eq!(entry.version, "3.0.2-0ubuntu1.16");
        assert!(AptPlugin::is_security_source(&entry.source));
    }

    #[test]
    fn parse_madison_output_non_security_source() {
        let output =
            "   nginx | 1.24.0-2 | http://archive.ubuntu.com/ubuntu noble/main amd64 Packages\n";
        let entry = AptPlugin::parse_madison_output(output).unwrap();
        assert!(!AptPlugin::is_security_source(&entry.source));
    }

    #[test]
    fn is_security_source_detects_security_urls() {
        // Ubuntu security repo
        assert!(AptPlugin::is_security_source(
            "http://security.ubuntu.com/ubuntu noble-security/main amd64 Packages"
        ));
        // Debian security repo
        assert!(AptPlugin::is_security_source(
            "http://security.debian.org/debian-security bookworm-security/main amd64 Packages"
        ));
        // Mixed case
        assert!(AptPlugin::is_security_source(
            "http://SECURITY.ubuntu.com/ubuntu noble-Security/main amd64 Packages"
        ));
    }

    #[test]
    fn is_security_source_rejects_non_security_urls() {
        assert!(!AptPlugin::is_security_source(
            "http://archive.ubuntu.com/ubuntu noble/main amd64 Packages"
        ));
        assert!(!AptPlugin::is_security_source(
            "http://archive.ubuntu.com/ubuntu noble-updates/main amd64 Packages"
        ));
        assert!(!AptPlugin::is_security_source(""));
    }

    #[test]
    fn parse_madison_output_missing_source_field() {
        let output = "   nginx | 1.24.0\n";
        let entry = AptPlugin::parse_madison_output(output).unwrap();
        assert_eq!(entry.version, "1.24.0");
        assert!(entry.source.is_empty());
        assert!(!AptPlugin::is_security_source(&entry.source));
    }

    // ── parse_madison_output_batch ───────────────────────────────────────

    #[test]
    fn parse_madison_output_batch_groups_by_package() {
        let output = concat!(
            "   nginx | 1.24.0-2ubuntu7.3 | http://archive.ubuntu.com/ubuntu noble-updates/main amd64 Packages\n",
            "   curl  | 7.88.1-10+deb12u5 | http://deb.debian.org/debian bookworm/main amd64 Packages\n",
            "   nginx | 1.18.0-6ubuntu14  | http://archive.ubuntu.com/ubuntu focal/main amd64 Packages\n",
        );
        let result = AptPlugin::parse_madison_output_batch(output);
        assert_eq!(result.len(), 2);
        assert_eq!(result["nginx"].version, "1.24.0-2ubuntu7.3");
        assert_eq!(result["curl"].version, "7.88.1-10+deb12u5");
    }

    #[test]
    fn parse_madison_output_batch_only_first_entry_kept() {
        let output = concat!(
            "   nginx | 1.24.0 | source1\n",
            "   nginx | 1.18.0 | source2\n",
        );
        let result = AptPlugin::parse_madison_output_batch(output);
        assert_eq!(result.len(), 1);
        assert_eq!(result["nginx"].version, "1.24.0");
    }

    #[test]
    fn parse_madison_output_batch_security_source_detected() {
        let output = "   openssl | 3.0.2-0ubuntu1.16 | http://security.ubuntu.com/ubuntu noble-security/main amd64 Packages\n";
        let result = AptPlugin::parse_madison_output_batch(output);
        assert!(AptPlugin::is_security_source(&result["openssl"].source));
    }

    #[test]
    fn parse_madison_output_batch_empty_returns_empty() {
        let result = AptPlugin::parse_madison_output_batch("");
        assert!(result.is_empty());
    }

    // ── batch_fetch ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_fetch_mixed_packages() {
        let executor = RoutedOutputExecutor::success([(
            "apt-cache",
            concat!(
                "   nginx | 1.24.0-2ubuntu7.3 | http://archive.ubuntu.com/ubuntu noble/main amd64 Packages\n",
                "   openssl | 3.0.2-0ubuntu1.16 | http://security.ubuntu.com/ubuntu noble-security/main amd64 Packages\n",
            ),
        )]);
        let plugin = test_plugin_with_executor(AptConfig::default(), executor);

        let items = vec![
            BatchFetchItem::new("nginx".to_string()),
            BatchFetchItem::new("openssl".to_string()),
            BatchFetchItem::new("curl".to_string()),
        ];
        let results = plugin.batch_fetch(&items).await.expect("ok");

        assert_eq!(results.len(), 3);

        let nginx = results
            .iter()
            .find(|r| r.package_identifier == "nginx")
            .unwrap();
        assert_eq!(nginx.releases.len(), 1);
        assert_eq!(nginx.releases[0].tag, "1.24.0-2ubuntu7.3");
        assert!(nginx.releases[0].category.is_none());
        assert!(nginx.error.is_none());

        let openssl = results
            .iter()
            .find(|r| r.package_identifier == "openssl")
            .unwrap();
        assert_eq!(openssl.releases.len(), 1);
        assert_eq!(openssl.releases[0].category, Some(UpdateCategory::Security));
        assert!(openssl.error.is_none());

        let curl = results
            .iter()
            .find(|r| r.package_identifier == "curl")
            .unwrap();
        assert!(curl.releases.is_empty(), "absent package has no releases");
        assert!(curl.error.is_none(), "absent package is not an error");
    }

    #[tokio::test]
    async fn batch_fetch_empty_items_returns_empty() {
        let executor = Arc::new(LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let plugin = test_plugin_with_executor(AptConfig::default(), executor);
        let results = plugin.batch_fetch(&[]).await.expect("ok");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn batch_fetch_invalid_identifier_fails() {
        let executor = Arc::new(LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let plugin = test_plugin_with_executor(AptConfig::default(), executor);
        let items = vec![BatchFetchItem::new("INVALID".to_string())];
        let result = plugin.batch_fetch(&items).await;
        assert!(result.is_err());
    }
}
