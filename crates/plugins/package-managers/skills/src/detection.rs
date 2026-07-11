use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::{BatchDetectItem, BatchDetectResult, Result, Version};

use crate::lock::{parse_skill_identifier, parse_skill_lock};
use crate::plugin::SkillsPlugin;

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for SkillsPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        let (url, skill_path) = parse_skill_identifier(package_identifier)
            .map_err(|e| e.context_to::<uptrakit_plugin_infrastructure_core::PluginError>())?;

        let content = match self.read_lock_file().await? {
            None => return Ok(None),
            Some(c) => c,
        };

        let entries = parse_skill_lock(&content)
            .map_err(|e| e.context_to::<uptrakit_plugin_infrastructure_core::PluginError>())?;

        let source_url = url.as_str().trim_end_matches('/');
        let found = entries.iter().find(|e| {
            e.source_url.trim_end_matches('/') == source_url && e.skill_path == skill_path
        });

        Ok(found.map(|e| Version::new(&e.skill_folder_hash)))
    }

    #[tracing::instrument(skip_all)]
    async fn batch_detect(&self, items: &[BatchDetectItem]) -> Result<Vec<BatchDetectResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        let lock_entries = match self.read_lock_file().await {
            Ok(None) => None,
            Ok(Some(content)) => match parse_skill_lock(&content) {
                Ok(entries) => Some(entries),
                Err(e) => {
                    let msg = e.to_string();
                    let results = items
                        .iter()
                        .map(|item| {
                            BatchDetectResult::error(item.package_identifier.clone(), msg.clone())
                        })
                        .collect();
                    return Ok(results);
                }
            },
            Err(e) => {
                let msg = e.to_string();
                let results = items
                    .iter()
                    .map(|item| {
                        BatchDetectResult::error(item.package_identifier.clone(), msg.clone())
                    })
                    .collect();
                return Ok(results);
            }
        };

        let results = items
            .iter()
            .map(
                |item| match parse_skill_identifier(&item.package_identifier) {
                    Err(e) => {
                        BatchDetectResult::error(item.package_identifier.clone(), e.to_string())
                    }
                    Ok((url, skill_path)) => {
                        let version = lock_entries.as_ref().and_then(|entries| {
                            let source_url = url.as_str().trim_end_matches('/');
                            entries
                                .iter()
                                .find(|e| {
                                    e.source_url.trim_end_matches('/') == source_url
                                        && e.skill_path == skill_path
                                })
                                .map(|e| Version::new(&e.skill_folder_hash))
                        });
                        BatchDetectResult::new(item.package_identifier.clone(), version, None)
                    }
                },
            )
            .collect();

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_plugin_infrastructure_core::{
        BatchDetectItem, Version, VersionDetector,
        testing::{FixedOutputExecutor, test_runtime_with_executor},
    };

    use crate::config::SkillsConfig;
    use crate::lock::encode_skill_identifier;
    use crate::plugin::SkillsPlugin;

    const SAMPLE_LOCK: &str = r#"{
      "version": 3,
      "skills": {
        "brainstorming": {
          "source": "obra/superpowers",
          "sourceUrl": "https://github.com/obra/superpowers",
          "sourceType": "github",
          "skillPath": "skills/brainstorming/SKILL.md",
          "skillFolderHash": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
        }
      }
    }"#;

    fn make_plugin(lock_output: &str, exit_code: i32) -> SkillsPlugin {
        SkillsPlugin::new(
            SkillsConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::new(lock_output, exit_code)),
        )
        .expect("create")
    }

    fn brainstorming_id() -> String {
        encode_skill_identifier(
            "https://github.com/obra/superpowers",
            "skills/brainstorming/SKILL.md",
        )
    }

    #[tokio::test]
    async fn detect_installed_version_found() {
        let plugin = make_plugin(SAMPLE_LOCK, 0);
        let result = plugin
            .detect_installed_version(&brainstorming_id())
            .await
            .expect("ok");
        assert_eq!(
            result,
            Some(Version::new("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"))
        );
    }

    #[tokio::test]
    async fn detect_installed_version_skill_not_in_lock() {
        let other_id =
            encode_skill_identifier("https://github.com/other/repo", "skills/other/SKILL.md");
        let plugin = make_plugin(SAMPLE_LOCK, 0);
        let result = plugin
            .detect_installed_version(&other_id)
            .await
            .expect("ok");
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn detect_installed_version_missing_lock_file_returns_none() {
        // sentinel exit 44 = file absent => not installed, no error
        let plugin = make_plugin("", 44);
        let result = plugin
            .detect_installed_version(&brainstorming_id())
            .await
            .expect("ok");
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn detect_installed_version_unreadable_lock_file_errors() {
        let plugin = make_plugin("", 1);
        plugin
            .detect_installed_version(&brainstorming_id())
            .await
            .unwrap_err();
    }

    #[tokio::test]
    async fn detect_installed_version_invalid_identifier_fails() {
        let plugin = make_plugin(SAMPLE_LOCK, 0);
        plugin
            .detect_installed_version("not-an-identifier")
            .await
            .unwrap_err();
    }

    #[tokio::test]
    async fn batch_detect_single_match() {
        let plugin = make_plugin(SAMPLE_LOCK, 0);
        let items = vec![BatchDetectItem::new(brainstorming_id())];
        let results = plugin.batch_detect(&items).await.expect("ok");
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].installed_version,
            Some(Version::new("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"))
        );
        assert!(results[0].error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_partial_miss_does_not_fail_batch() {
        let other_id =
            encode_skill_identifier("https://github.com/other/repo", "skills/other/SKILL.md");
        let plugin = make_plugin(SAMPLE_LOCK, 0);
        let items = vec![
            BatchDetectItem::new(brainstorming_id()),
            BatchDetectItem::new(other_id.clone()),
        ];
        let results = plugin.batch_detect(&items).await.expect("ok");
        assert_eq!(results.len(), 2);
        let found = results
            .iter()
            .find(|r| r.package_identifier == brainstorming_id())
            .expect("found");
        assert!(found.installed_version.is_some());
        assert!(found.error.is_none());
        let miss = results
            .iter()
            .find(|r| r.package_identifier == other_id)
            .expect("miss");
        assert_eq!(miss.installed_version, None);
        assert!(miss.error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_invalid_id_returns_per_item_error() {
        let plugin = make_plugin(SAMPLE_LOCK, 0);
        let items = vec![
            BatchDetectItem::new(brainstorming_id()),
            BatchDetectItem::new("invalid-id".to_string()),
        ];
        let results = plugin.batch_detect(&items).await.expect("batch succeeds");
        let invalid = results
            .iter()
            .find(|r| r.package_identifier == "invalid-id")
            .expect("invalid");
        assert!(
            invalid.error.is_some(),
            "invalid id should produce per-item error"
        );
    }

    #[tokio::test]
    async fn batch_detect_missing_lock_file_yields_none_without_error() {
        let plugin = make_plugin("", 44);
        let items = vec![BatchDetectItem::new(brainstorming_id())];
        let results = plugin.batch_detect(&items).await.expect("ok");
        assert_eq!(results[0].installed_version, None);
        assert!(results[0].error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_unreadable_lock_file_yields_per_item_errors() {
        let plugin = make_plugin("", 1);
        let items = vec![BatchDetectItem::new(brainstorming_id())];
        let results = plugin.batch_detect(&items).await.expect("ok");
        assert!(results[0].error.is_some());
    }

    #[tokio::test]
    async fn batch_detect_corrupt_lock_file_yields_per_item_errors() {
        let plugin = make_plugin("{corrupt", 0);
        let items = vec![BatchDetectItem::new(brainstorming_id())];
        let results = plugin.batch_detect(&items).await.expect("ok");
        assert!(results[0].error.is_some());
    }
}
