use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::{
    BatchUpdateItem, BatchUpdateResult, OutputStreamType, PluginError, ReleaseInfo, Result,
    UpdateOutputSender,
};

use crate::plugin::{APT_BATCH_PREF_FILE, AptPlugin, validate_identifier, validate_version};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::PackageIndexer for AptPlugin {
    #[tracing::instrument(skip_all)]
    async fn refresh_package_index(&self) -> Result<()> {
        tracing::info!("refreshing APT package index");
        let cmd_output = self
            .executor
            .execute_quiet(
                &CommandSpec::exec("apt-get", ["update".to_string(), "-q".to_string()])
                    .with_env("DEBIAN_FRONTEND", "noninteractive")
                    .privileged(),
            )
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "apt-get update failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        tracing::info!("APT package index refreshed");
        Ok(())
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for AptPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &UpdateOutputSender,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;
        validate_version(to_version).map_err(|e| report!(PluginError::Configuration(e)))?;

        tracing::debug!(
            package = %package_identifier,
            version = %to_version,
            "running apt-get install"
        );

        uptrakit_plugin_infrastructure_core::execute_command_update(
            uptrakit_plugin_infrastructure_core::CommandUpdateParams {
                executor: self.executor.as_ref(),
                binary: "apt-get",
                args: vec![
                    "install".to_string(),
                    "--yes".to_string(),
                    "--no-install-recommends".to_string(),
                    format!("{package_identifier}={to_version}"),
                ],
                privileged: true,
                spec_modifier: Some(Box::new(|spec| {
                    spec.with_env("DEBIAN_FRONTEND", "noninteractive")
                })),
                exit_code_success: None,
                exit_code_error: None,
            },
            output_tx,
        )
        .await
    }

    /// Execute batch updates using APT preferences pin-priority mechanism.
    ///
    /// Writes a temporary preferences file that blocks all upgrades except the
    /// specifically targeted packages, then runs `apt-get upgrade --yes`.
    /// The temp file is written to a user-writable path (no sudo needed) and
    /// deleted after the upgrade completes.
    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &UpdateOutputSender,
    ) -> Result<Vec<BatchUpdateResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all package identifiers and versions up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
            validate_version(&item.to_version)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        // Build apt_preferences content:
        // 1. Block all upgrades by default
        // 2. Allow only our targeted packages at pin-priority 990
        let mut prefs = String::from(
            "# uptrakit batch update preferences (temporary)\n\
             Package: *\n\
             Pin: release *\n\
             Pin-Priority: -1\n\n",
        );
        for item in items {
            prefs.push_str(&format!(
                "Package: {}\nPin: version {}\nPin-Priority: 990\n\n",
                item.package_identifier, item.to_version
            ));
        }

        // APT_BATCH_PREF_FILE is hardcoded here and in required_sudo_commands()
        // so the sudoers rule locks down this exact -o Dir::Etc::Preferences=
        // invocation. Never change this path without updating both.
        let pref_path = std::path::Path::new(APT_BATCH_PREF_FILE);
        std::fs::write(pref_path, &prefs).map_err(|e| {
            report!(PluginError::PluginInternal(format!(
                "failed to write apt preferences file: {e}"
            )))
        })?;

        let pref_path_str = pref_path.display().to_string();
        let dir_opt = format!("Dir::Etc::Preferences={pref_path_str}");

        let args = vec![
            "-o".to_string(),
            dir_opt,
            "upgrade".to_string(),
            "--yes".to_string(),
        ];

        let display_args = std::iter::once("apt-get")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");

        let pkg_list: Vec<&str> = items
            .iter()
            .map(|i| i.package_identifier.as_str())
            .collect();
        send_output(
            output_tx,
            &format!(
                "Batch updating {} packages: {}\nRunning: {display_args}",
                items.len(),
                pkg_list.join(", ")
            ),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_args}\n");

        tracing::debug!(
            count = items.len(),
            packages = ?pkg_list,
            "running apt-get batch upgrade"
        );

        let cmd_result = self
            .executor
            .execute(
                &CommandSpec::exec("apt-get", args)
                    .with_env("DEBIAN_FRONTEND", "noninteractive")
                    .privileged(),
                output_tx,
            )
            .await;

        // Always clean up the temp file (no sudo needed).
        if let Err(e) = std::fs::remove_file(pref_path) {
            tracing::warn!(path = %pref_path_str, error = %e, "failed to remove apt preferences file");
        }

        let cmd_output =
            cmd_result.map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;
        output.push_str(&cmd_output.output);

        let success = cmd_output.exit_code == 0;
        let results = items
            .iter()
            .map(|item| {
                BatchUpdateResult::new(item.package_identifier.clone(), success, output.clone())
            })
            .collect();

        Ok(results)
    }
}
