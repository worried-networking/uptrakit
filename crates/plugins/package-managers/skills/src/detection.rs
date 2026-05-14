use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, Result, Version, command::CommandSpec,
};

use crate::lock::{parse_skill_identifier, parse_skill_lock};
use crate::plugin::SkillsPlugin;

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for SkillsPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        let (url, skill_path) = parse_skill_identifier(package_identifier)
            .map_err(|e| e.context_to::<uptrakit_plugin_infrastructure_core::PluginError>())?;

        let cmd_output = match self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "sh",
                [
                    "-c".to_string(),
                    "cat ~/.agents/.skill-lock.json".to_string(),
                ],
            ))
            .await
        {
            Ok(out) if out.exit_code == 0 => out,
            Ok(_) | Err(_) => return Ok(None),
        };

        let entries = match parse_skill_lock(&cmd_output.output) {
            Ok(e) => e,
            Err(_) => return Ok(None),
        };

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

        let lock_content: Option<Vec<crate::lock::SkillLockEntry>> = match self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "sh",
                [
                    "-c".to_string(),
                    "cat ~/.agents/.skill-lock.json".to_string(),
                ],
            ))
            .await
        {
            Ok(out) if out.exit_code == 0 => parse_skill_lock(&out.output).ok(),
            _ => None,
        };

        let results = items
            .iter()
            .map(
                |item| match parse_skill_identifier(&item.package_identifier) {
                    Err(e) => {
                        BatchDetectResult::error(item.package_identifier.clone(), e.to_string())
                    }
                    Ok((url, skill_path)) => {
                        let version = lock_content.as_ref().and_then(|entries| {
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
      "brainstorming": {
        "source": "obra/superpowers",
        "sourceUrl": "https://github.com/obra/superpowers",
        "sourceType": "github",
        "skillPath": "skills/brainstorming/SKILL.md",
        "skillFolderHash": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
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
    async fn detect_installed_version_command_fails_returns_none() {
        let plugin = make_plugin("", 1);
        let result = plugin
            .detect_installed_version(&brainstorming_id())
            .await
            .expect("ok");
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn detect_installed_version_invalid_identifier_fails() {
        let plugin = make_plugin(SAMPLE_LOCK, 0);
        let result = plugin.detect_installed_version("not-an-identifier").await;
        assert!(result.is_err());
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
}
