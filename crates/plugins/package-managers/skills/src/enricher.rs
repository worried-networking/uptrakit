//! `InstalledVersionEnricher` implementation for the Agent Skills plugin.
//!
//! An installed version is the SHA of the skill subtree as committed to the
//! source repository. This enricher maps that SHA back to its committer date,
//! formatted as strict ISO 8601 UTC second precision (see
//! [`crate::releases::DISPLAY_FMT`]), so the dashboard can render a stable,
//! human-friendly version string.
//!
//! The enricher groups input items by `(owner, repo, skill_dir)` so that
//! exactly one provider call is made per unique skill directory in a batch,
//! independent of how many items share that directory.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_global_github_provider::PACKAGE_MANAGER_SKILLS;
use uptrakit_plugin_infrastructure_core::{
    InstalledVersionDisplay, InstalledVersionEnricher, InstalledVersionItem, PluginError, Result,
};

use crate::error::SkillsError;
use crate::lock::parse_skill_identifier;
use crate::plugin::SkillsPlugin;
use crate::releases::{COMMIT_WINDOW, DISPLAY_FMT, derive_skill_dir, parse_github_owner_repo};

/// Identifier resolved into the three parts the GitHub provider primitive needs.
struct Resolved {
    owner: String,
    repo: String,
    skill_dir: String,
}

#[async_trait]
impl InstalledVersionEnricher for SkillsPlugin {
    async fn enrich_installed_versions(
        &self,
        items: &[InstalledVersionItem],
    ) -> Result<Vec<InstalledVersionDisplay>> {
        let provider = self
            .provider
            .as_ref()
            .ok_or_else(|| {
                report!(SkillsError::ProviderUnavailable(
                    "skills installed-version enrichment requires the global GitHub provider"
                        .to_string()
                ))
            })
            .context_to::<PluginError>()?;

        // ── 1. Resolve identifiers (parse + derive skill_dir) once ────────────
        // Items with `installed_version = None` short-circuit to `None`
        // (`None`-input contract). Parse failures collapse to `None` here so
        // they propagate through the same miss path as unknown SHAs.
        let mut resolved: Vec<Option<Resolved>> = Vec::with_capacity(items.len());
        for item in items {
            if item.installed_version.is_none() {
                resolved.push(None);
                continue;
            }
            let parsed = (|| -> Result<Resolved> {
                let (source_url, skill_path) =
                    parse_skill_identifier(&item.package_identifier).context_to()?;
                let (owner, repo) = parse_github_owner_repo(&source_url)?;
                let skill_dir = derive_skill_dir(&skill_path).to_string();
                if skill_dir.is_empty() {
                    return Err(report!(SkillsError::InvalidIdentifier(
                        "skill path has no parent directory".to_string()
                    )))
                    .context_to();
                }
                Ok(Resolved {
                    owner,
                    repo,
                    skill_dir,
                })
            })();
            resolved.push(parsed.ok());
        }

        // ── 2. Group expected SHAs by (owner, repo, skill_dir) ────────────────
        let mut expected_by_key: HashMap<(String, String, String), HashSet<String>> =
            HashMap::new();
        for (r, item) in resolved.iter().zip(items.iter()) {
            let Some(r) = r else { continue };
            let Some(sha) = item.installed_version.as_ref() else {
                continue;
            };
            let key = (r.owner.clone(), r.repo.clone(), r.skill_dir.clone());
            expected_by_key.entry(key).or_default().insert(sha.clone());
        }

        // ── 3. One provider call per group; build SHA → date map ──────────────
        let mut dates_by_key: HashMap<
            (String, String, String),
            HashMap<String, time::OffsetDateTime>,
        > = HashMap::new();
        for (key, expected) in &expected_by_key {
            let resp = provider
                .list_recent_commit_dates_for_path(
                    PACKAGE_MANAGER_SKILLS,
                    &key.0,
                    &key.1,
                    &key.2,
                    COMMIT_WINDOW,
                    expected,
                )
                .await;
            let map = match resp {
                Ok(entries) => entries
                    .into_iter()
                    .map(|tc| (tc.tree_sha_at_path, tc.committed_at))
                    .collect::<HashMap<_, _>>(),
                Err(e) => {
                    tracing::warn!(
                        owner = %key.0,
                        repo = %key.1,
                        path = %key.2,
                        error = %e,
                        reason = "provider_error",
                        "installed-version enrichment: provider call failed; skipping group"
                    );
                    HashMap::new()
                }
            };
            dates_by_key.insert(key.clone(), map);
        }

        // ── 4. Build per-item output preserving input order ───────────────────
        let mut out = Vec::with_capacity(items.len());
        for (item, r) in items.iter().zip(resolved.iter()) {
            let display_version = match (r, &item.installed_version) {
                (Some(r), Some(sha)) => {
                    let key = (r.owner.clone(), r.repo.clone(), r.skill_dir.clone());
                    dates_by_key
                        .get(&key)
                        .and_then(|m| m.get(sha))
                        .and_then(|dt| dt.format(&DISPLAY_FMT).ok())
                }
                _ => None,
            };
            out.push(InstalledVersionDisplay::new(
                item.package_identifier.clone(),
                item.installed_version.clone(),
                display_version,
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use uptrakit_global_github_provider::{
        GitHubProviderClient, GitHubProviderError, GitHubRepositoryTree, GlobalProviderConsumerId,
        TreeCommit,
    };
    use uptrakit_plugin_infrastructure_core::testing::test_runtime;
    use uptrakit_plugin_infrastructure_core::{InstalledVersionEnricher, InstalledVersionItem};

    use crate::config::SkillsConfig;
    use crate::plugin::SkillsPlugin;

    fn make_plugin_with_provider(provider: Arc<dyn GitHubProviderClient>) -> SkillsPlugin {
        let mut plugin =
            SkillsPlugin::new(SkillsConfig::default(), test_runtime()).expect("create plugin");
        plugin.provider = Some(provider);
        plugin
    }

    fn skill_id(source_url: &str, skill_path: &str) -> String {
        format!("{source_url}#{skill_path}")
    }

    #[tokio::test]
    async fn enrich_installed_versions_maps_known_sha_to_commit_date() {
        struct P;
        #[async_trait]
        impl GitHubProviderClient for P {
            async fn fetch_repository_tree(
                &self,
                _: GlobalProviderConsumerId,
                _: &str,
                _: &str,
                _: &str,
                _: bool,
            ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
                Err(GitHubProviderError::Misconfigured(
                    "enrich_installed_versions must not call fetch_repository_tree".to_string(),
                ))
            }
            async fn list_recent_commit_dates_for_path(
                &self,
                _: GlobalProviderConsumerId,
                _: &str,
                _: &str,
                _: &str,
                _: usize,
                _expected: &HashSet<String>,
            ) -> std::result::Result<Vec<TreeCommit>, GitHubProviderError> {
                Ok(vec![
                    TreeCommit::new(
                        "sha_old".to_string(),
                        time::macros::datetime!(2026-04-01 00:00:00 UTC),
                    ),
                    TreeCommit::new(
                        "sha_new".to_string(),
                        time::macros::datetime!(2026-06-11 01:15:00 UTC),
                    ),
                ])
            }
        }
        let plugin = make_plugin_with_provider(Arc::new(P));
        let items = vec![
            InstalledVersionItem::new(
                skill_id(
                    "https://github.com/obra/superpowers",
                    "skills/brainstorming/SKILL.md",
                ),
                Some("sha_new".to_string()),
            ),
            InstalledVersionItem::new(
                skill_id(
                    "https://github.com/obra/superpowers",
                    "skills/dispatching/SKILL.md",
                ),
                Some("not_in_window".to_string()),
            ),
            InstalledVersionItem::new(
                skill_id(
                    "https://github.com/obra/superpowers",
                    "skills/empty/SKILL.md",
                ),
                None,
            ),
        ];

        let out = plugin.enrich_installed_versions(&items).await.expect("ok");

        assert_eq!(out.len(), 3, "must preserve length");
        assert_eq!(out[0].installed_version_echo.as_deref(), Some("sha_new"));
        assert_eq!(
            out[0].display_version.as_deref(),
            Some("2026-06-11T01:15:00Z")
        );
        assert_eq!(
            out[1].installed_version_echo.as_deref(),
            Some("not_in_window")
        );
        assert_eq!(out[1].display_version, None, "miss → None");
        assert_eq!(out[2].installed_version_echo, None);
        assert_eq!(out[2].display_version, None, "None-input contract");
    }

    #[tokio::test]
    async fn enrich_installed_versions_groups_by_skill_dir_to_minimize_calls() {
        struct Counting {
            calls: Arc<AtomicUsize>,
        }
        #[async_trait]
        impl GitHubProviderClient for Counting {
            async fn fetch_repository_tree(
                &self,
                _: GlobalProviderConsumerId,
                _: &str,
                _: &str,
                _: &str,
                _: bool,
            ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
                Err(GitHubProviderError::Misconfigured(
                    "enrich_installed_versions must not call fetch_repository_tree".to_string(),
                ))
            }
            async fn list_recent_commit_dates_for_path(
                &self,
                _: GlobalProviderConsumerId,
                _: &str,
                _: &str,
                _: &str,
                _: usize,
                _: &HashSet<String>,
            ) -> std::result::Result<Vec<TreeCommit>, GitHubProviderError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(vec![])
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let plugin = make_plugin_with_provider(Arc::new(Counting {
            calls: calls.clone(),
        }));

        let id1 = skill_id("https://github.com/obra/superpowers", "skills/a/SKILL.md");
        let id2 = skill_id("https://github.com/obra/superpowers", "skills/a/SKILL.md");
        let id3 = skill_id("https://github.com/obra/superpowers", "skills/b/SKILL.md");
        let items = vec![
            InstalledVersionItem::new(id1, Some("x".into())),
            InstalledVersionItem::new(id2, Some("y".into())),
            InstalledVersionItem::new(id3, Some("z".into())),
        ];
        let _ = plugin.enrich_installed_versions(&items).await.expect("ok");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "one call per unique (owner, repo, skill_dir)"
        );
    }
}
