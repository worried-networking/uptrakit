//! [`UpdateExecutor`] implementation for the Agent Skills plugin.
//!
//! Reads the on-disk skill lock file, locates the matching entry by
//! `(source_url, skill_path)`, then runs:
//!
//! ```text
//! DISABLE_TELEMETRY=1 npx skills@<version> update -g <skill_name> -y
//! ```
//!
//! The lock-file read uses `sh -c 'cat ~/.agents/.skill-lock.json'` so
//! tilde expansion is handled by the shell, matching the same pattern used
//! in [`crate::detection`].

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::{
    CommandSpec, CommandUpdateParams, ExecuteUpdateResult, PluginError, ReleaseInfo, Result,
    UpdateOutputSender, execute_command_update,
};

use crate::error::SkillsError;
use crate::lock::{parse_skill_identifier, parse_skill_lock};
use crate::plugin::SkillsPlugin;

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for SkillsPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        _to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &UpdateOutputSender,
    ) -> Result<ExecuteUpdateResult> {
        // 1. Parse the composite identifier → (source_url, skill_path).
        let (url, skill_path) = parse_skill_identifier(package_identifier)
            .map_err(|e| e.context_to::<PluginError>())?;

        // 2. Read the on-disk lock file.
        let lock_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "sh",
                [
                    "-c".to_string(),
                    "cat ~/.agents/.skill-lock.json".to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "failed to read skill lock file: {e}"
                )))
            })?;

        if lock_output.exit_code != 0 {
            bail!(PluginError::PluginInternal(
                "skill lock file not found or unreadable".to_string()
            ));
        }

        // 3. Parse the lock JSON.
        let entries =
            parse_skill_lock(&lock_output.output).map_err(|e| e.context_to::<PluginError>())?;

        // 4. Find the entry whose source_url + skill_path match.
        let source_url = url.as_str().trim_end_matches('/');
        let entry = entries
            .iter()
            .find(|e| {
                e.source_url.trim_end_matches('/') == source_url && e.skill_path == skill_path
            })
            .ok_or_else(|| {
                report!(SkillsError::LockEntryNotFound(
                    package_identifier.to_string()
                ))
            })
            .context_to::<PluginError>()?;

        let skill_name = entry.name.clone();
        let skills_version = self.config.skills_version.clone();

        tracing::debug!(
            skill = %skill_name,
            skills_version = %skills_version,
            "running npx skills update"
        );

        // 5. Run: DISABLE_TELEMETRY=1 npx skills@{version} update -g {skill_name} -y
        let output = execute_command_update(
            CommandUpdateParams {
                executor: self.executor.as_ref(),
                binary: "npx",
                args: vec![
                    format!("skills@{skills_version}"),
                    "update".to_string(),
                    "-g".to_string(),
                    skill_name,
                    "-y".to_string(),
                ],
                privileged: false,
                spec_modifier: Some(Box::new(|spec| spec.with_env("DISABLE_TELEMETRY", "1"))),
                exit_code_success: None,
                exit_code_error: None,
            },
            output_tx,
        )
        .await?;

        // 6. Return result; skills update never requires a reboot.
        Ok(ExecuteUpdateResult::new(output, false))
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;
    use uptrakit_plugin_infrastructure_core::{
        UpdateExecutor,
        testing::{RoutedOutputExecutor, test_runtime_with_executor},
    };

    use crate::config::SkillsConfig;
    use crate::lock::encode_skill_identifier;
    use crate::plugin::SkillsPlugin;

    /// Minimal sample lock JSON containing a single "brainstorming" skill.
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

    fn brainstorming_id() -> String {
        encode_skill_identifier(
            "https://github.com/obra/superpowers",
            "skills/brainstorming/SKILL.md",
        )
    }

    fn make_tx() -> uptrakit_plugin_infrastructure_core::UpdateOutputSender {
        let (tx, _rx) = mpsc::channel(16);
        tx
    }

    /// Build a plugin whose first command (sh/cat) returns the given lock JSON
    /// and whose second command (npx) returns empty stdout — both with exit 0.
    ///
    /// `RoutedOutputExecutor` routes by program name: `"sh"` → lock content,
    /// `"npx"` → empty string.
    fn make_plugin_with_lock(lock_json: &'static str) -> SkillsPlugin {
        let executor = RoutedOutputExecutor::success([("sh", lock_json), ("npx", "")]);
        SkillsPlugin::new(
            SkillsConfig::default(),
            test_runtime_with_executor(executor),
        )
        .expect("create")
    }

    // ── execute_update happy path ─────────────────────────────────────────────

    #[tokio::test]
    async fn execute_update_calls_npx_skills_update() {
        let plugin = make_plugin_with_lock(SAMPLE_LOCK);
        let tx = make_tx();
        let result = plugin
            .execute_update(&brainstorming_id(), "deadbeef", None, &tx)
            .await
            .expect("update ok");

        assert!(
            !result.resumable,
            "skills update must not be resumable (reboot not required)"
        );
    }

    // ── invalid identifier ────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_update_invalid_identifier_fails() {
        let plugin = make_plugin_with_lock(SAMPLE_LOCK);
        let tx = make_tx();
        let result = plugin
            .execute_update("not-an-identifier", "deadbeef", None, &tx)
            .await;
        assert!(result.is_err(), "invalid identifier must return Err");
    }

    // ── skill not in lock ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_update_skill_not_in_lock_fails() {
        // Lock is an empty object — no skills are installed.
        let plugin = make_plugin_with_lock("{}");
        let tx = make_tx();
        let result = plugin
            .execute_update(&brainstorming_id(), "deadbeef", None, &tx)
            .await;
        assert!(result.is_err(), "skill absent from lock must return Err");
    }
}
