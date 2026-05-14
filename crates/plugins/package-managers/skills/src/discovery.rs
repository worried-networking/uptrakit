use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::{
    DiscoveredSoftware, DiscoveryTarget, HostCompatibility, PluginRole, Result,
    command::CommandSpec, plugin_ids,
};

use crate::lock::{encode_skill_identifier, parse_skill_lock};
use crate::plugin::SkillsPlugin;

#[async_trait]
impl uptrakit_plugin_infrastructure_core::Discoverer for SkillsPlugin {
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering globally installed Skills");

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
            Ok(_) | Err(_) => {
                tracing::debug!("skill lock file absent or unreadable; returning empty discovery");
                return Ok(vec![]);
            }
        };

        let entries = match parse_skill_lock(&cmd_output.output) {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(error = %err, "failed to parse skill lock file");
                return Ok(vec![]);
            }
        };

        let mut discovered = Vec::new();
        for entry in entries {
            if entry.source_type != "github" {
                tracing::warn!(
                    name = %entry.name,
                    source_type = %entry.source_type,
                    "unsupported skill source type; skipping"
                );
                continue;
            }

            let encoded_id = encode_skill_identifier(&entry.source_url, &entry.skill_path);

            let target = DiscoveryTarget {
                plugin_type: plugin_ids::PACKAGE_MANAGER_SKILLS.clone(),
                plugin_config: serde_json::json!({}),
                plugin_config_name: "Agent Skills".to_string(),
                roles: vec![
                    PluginRole::DetectVersion,
                    PluginRole::FetchReleases,
                    PluginRole::ExecuteUpdate,
                ],
                package_identifier: Some(encoded_id),
                config_override: None,
                execution_site: None,
            };

            let extra = serde_json::json!({
                "source_url": entry.source_url,
                "skill_path": entry.skill_path,
                "agents": "~/.agents",
                "lock_name": ".skill-lock.json",
            });

            discovered.push(DiscoveredSoftware {
                package_identifier: entry.name.clone(),
                name: format!("LLM Skill: {}", entry.name),
                installed_version: entry.skill_folder_hash,
                targets: vec![target],
                extra: Some(extra),
                qualifier: None,
                plugin_package_identifier: None,
                featured: true,
                installed_display_version: None,
            });
        }

        tracing::debug!(count = discovered.len(), "skills discovery complete");
        Ok(discovered)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["npx".to_string()]))
            .await
        {
            Ok(out) if out.exit_code == 0 => Ok(HostCompatibility::Compatible),
            _ => Ok(HostCompatibility::Incompatible("npx not found".to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_plugin_infrastructure_core::{
        Discoverer, HostCompatibility,
        testing::{FixedOutputExecutor, test_runtime_with_executor},
    };

    use crate::config::SkillsConfig;
    use crate::plugin::SkillsPlugin;

    fn make_plugin(output: &str, exit_code: i32) -> SkillsPlugin {
        SkillsPlugin::new(
            SkillsConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::new(output, exit_code)),
        )
        .expect("create")
    }

    const SAMPLE_LOCK: &str = r#"{
      "brainstorming": {
        "source": "obra/superpowers",
        "sourceUrl": "https://github.com/obra/superpowers",
        "sourceType": "github",
        "skillPath": "skills/brainstorming/SKILL.md",
        "skillFolderHash": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        "installedAt": "2025-01-01T00:00:00Z",
        "updatedAt": "2025-01-02T00:00:00Z"
      }
    }"#;

    const MIXED_LOCK: &str = r#"{
      "brainstorming": {
        "source": "obra/superpowers",
        "sourceUrl": "https://github.com/obra/superpowers",
        "sourceType": "github",
        "skillPath": "skills/brainstorming/SKILL.md",
        "skillFolderHash": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
      },
      "local-skill": {
        "source": "local/source",
        "sourceUrl": "https://gitlab.com/local/source",
        "sourceType": "gitlab",
        "skillPath": "skills/local-skill/SKILL.md",
        "skillFolderHash": "aabbccddaabbccddaabbccddaabbccddaabbccdd"
      }
    }"#;

    #[tokio::test]
    async fn empty_lock_file_returns_empty_discovery() {
        let plugin = make_plugin("{}", 0);
        let result = plugin.discover_software().await.expect("ok");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn command_failure_returns_empty_discovery() {
        let plugin = make_plugin("", 1);
        let result = plugin.discover_software().await.expect("ok");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn github_entries_are_discovered() {
        let plugin = make_plugin(SAMPLE_LOCK, 0);
        let result = plugin.discover_software().await.expect("ok");
        assert_eq!(result.len(), 1);
        let sw = &result[0];
        assert_eq!(sw.name, "LLM Skill: brainstorming");
        assert_eq!(sw.package_identifier, "brainstorming");
        assert_eq!(
            sw.installed_version,
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
        );
        assert_eq!(sw.targets.len(), 1);
        let encoded = sw.targets[0].package_identifier.as_deref().expect("set");
        assert!(encoded.starts_with("https://github.com/obra/superpowers#"));
        assert!(encoded.contains("skills/brainstorming/SKILL.md"));
    }

    #[tokio::test]
    async fn non_github_entries_skipped() {
        let plugin = make_plugin(MIXED_LOCK, 0);
        let result = plugin.discover_software().await.expect("ok");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "LLM Skill: brainstorming");
    }

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_npx_found() {
        let plugin = make_plugin("", 0);
        let result = plugin.detect_host_compatibility().await.expect("ok");
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_npx_missing() {
        let plugin = make_plugin("", 1);
        let result = plugin.detect_host_compatibility().await.expect("ok");
        match result {
            HostCompatibility::Incompatible(msg) => {
                assert!(
                    msg.contains("npx"),
                    "message should mention npx, got: {msg}"
                );
            }
            _ => panic!("expected Incompatible"),
        }
    }
}
