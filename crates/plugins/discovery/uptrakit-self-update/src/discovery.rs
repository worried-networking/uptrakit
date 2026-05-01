//! `Discoverer` implementation for [`UptrakitSelfUpdatePlugin`].
//!
//! `detect_host_compatibility` checks that the plugin is both enabled and
//! running as an embedded agent inside a controller-standalone (indicated by
//! the presence of a `ServiceMetadataProvider`).
//!
//! `discover_software` delegates to `build_software_item` which synthesises
//! a fully-specified `DiscoveredSoftware` with three `DiscoveryTarget` entries
//! (detect-version, fetch-releases, execute-update) derived from the service
//! metadata provided by the controller at construction time.

use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::{
    DeploymentTopology, DiscoveredSoftware, DiscoveryTarget, HostCompatibility, ServiceMetadata,
};
use uptrakit_shared_types::{PluginRole, plugin_ids};

use crate::error::SelfUpdateError;
use crate::plugin::UptrakitSelfUpdatePlugin;

// ── Discoverer ────────────────────────────────────────────────────────────

#[async_trait]
impl uptrakit_plugin_infrastructure_core::Discoverer for UptrakitSelfUpdatePlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<HostCompatibility> {
        if !self.config.enabled {
            return Ok(HostCompatibility::Incompatible(
                "uptrakit self-update is disabled — set `enabled = true` to opt in".to_string(),
            ));
        }
        if self.metadata_provider.is_none() {
            return Ok(HostCompatibility::Incompatible(
                "not running as embedded agent in controller-standalone: \
                 no metadata provider available"
                    .to_string(),
            ));
        }
        Ok(HostCompatibility::Compatible)
    }

    #[tracing::instrument(skip_all)]
    async fn discover_software(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<DiscoveredSoftware>> {
        if !self.config.enabled {
            return Ok(vec![]);
        }
        let Some(ref provider) = self.metadata_provider else {
            return Ok(vec![]);
        };
        let metadata = provider.get_metadata();
        match self.build_software_item(&metadata) {
            Ok(item) => Ok(vec![item]),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "uptrakit-self-update: skipping service item due to error"
                );
                Ok(vec![])
            }
        }
    }
}

// ── build_software_item ───────────────────────────────────────────────────

impl UptrakitSelfUpdatePlugin {
    /// Build a `DiscoveredSoftware` item from controller service metadata.
    ///
    /// Returns three `DiscoveryTarget` entries:
    /// - **detect_version** — runs `<binary> --version` on the agent
    /// - **fetch_releases** — queries GitHub Releases on the controller
    /// - **execute_update** — runs the inline shell / Docker update script on the agent
    ///
    /// # Errors
    ///
    /// Returns [`SelfUpdateError::NoBinaryPath`] when the topology is
    /// `UnixBinary` but `binary_path` is `None`.
    ///
    /// Returns [`SelfUpdateError::NoPidFile`] when the topology is
    /// `UnixBinary` with `reuseport_configured = true` but `pid_file` is `None`.
    pub(crate) fn build_software_item(
        &self,
        metadata: &ServiceMetadata,
    ) -> crate::error::Result<DiscoveredSoftware> {
        let detect_version_target = self.build_detect_version_target(metadata)?;
        let fetch_releases_target = self.build_fetch_releases_target(metadata);
        let execute_update_target = self.build_execute_update_target(metadata)?;

        Ok(DiscoveredSoftware {
            package_identifier: metadata.service_name.clone(),
            name: format!("Uptrakit \u{2014} {}", metadata.service_name),
            installed_version: metadata.version.clone(),
            targets: vec![
                detect_version_target,
                fetch_releases_target,
                execute_update_target,
            ],
            extra: Some(serde_json::json!({ "awaiting_restart_timeout": 120 })),
            qualifier: None,
            plugin_package_identifier: None,
            featured: true,
            installed_display_version: None,
        })
    }

    /// Build the detect-version target (runs on the agent).
    fn build_detect_version_target(
        &self,
        metadata: &ServiceMetadata,
    ) -> crate::error::Result<DiscoveryTarget> {
        let binary_path = metadata
            .binary_path
            .as_ref()
            .ok_or(SelfUpdateError::NoBinaryPath)?;
        let version_command = format!("{} --version", binary_path.display());
        Ok(DiscoveryTarget {
            plugin_type: plugin_ids::GENERIC_SHELL.clone(),
            plugin_config: serde_json::json!({
                "version_command": version_command,
                "version_regex": r"(?P<version>\d+\.\d+\.\d+)"
            }),
            plugin_config_name: format!("{} version detection", metadata.service_name),
            roles: vec![PluginRole::DetectVersion],
            package_identifier: None,
            config_override: None,
            execution_site: Some("agent".to_string()),
        })
    }

    /// Build the fetch-releases target (runs on the controller via GitHub Releases).
    fn build_fetch_releases_target(&self, metadata: &ServiceMetadata) -> DiscoveryTarget {
        DiscoveryTarget {
            plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
            plugin_config: serde_json::json!({
                "owner": "uptrakit",
                "repo": "uptrakit",
                "tag_strip_prefix": "v",
                "asset_filter": metadata.service_name
            }),
            plugin_config_name: "Uptrakit GitHub Releases".to_string(),
            roles: vec![PluginRole::FetchReleases],
            package_identifier: None,
            config_override: None,
            execution_site: Some("controller".to_string()),
        }
    }

    /// Build the execute-update target.
    ///
    /// For `UnixBinary`: generates an inline shell script.
    /// For `DockerContainer`: generates a Docker update config.
    fn build_execute_update_target(
        &self,
        metadata: &ServiceMetadata,
    ) -> crate::error::Result<DiscoveryTarget> {
        match &metadata.deployment_topology {
            DeploymentTopology::UnixBinary => self.build_unix_binary_execute_target(metadata),
            DeploymentTopology::DockerContainer {
                image,
                container_name,
            } => Ok(self.build_docker_execute_target(metadata, image, container_name)),
            _ => Err(rootcause::report!(SelfUpdateError::NoBinaryPath)),
        }
    }

    fn build_unix_binary_execute_target(
        &self,
        metadata: &ServiceMetadata,
    ) -> crate::error::Result<DiscoveryTarget> {
        let binary_path = metadata
            .binary_path
            .as_ref()
            .ok_or(SelfUpdateError::NoBinaryPath)?;
        let binary_path_str = binary_path.display().to_string();

        let update_command = if metadata.reuseport_configured {
            let pid_file = metadata
                .pid_file
                .as_ref()
                .ok_or(SelfUpdateError::NoPidFile)?;
            let pid_file_str = pid_file.display().to_string();
            format!(
                r#"BINARY_PATH="{binary}"
TMP_PATH="${{BINARY_PATH}}.new-$$"
curl -L "$RELEASE_URL" -o "$TMP_PATH"
chmod +x "$TMP_PATH"
command -v codesign >/dev/null 2>&1 && codesign --sign - --force "$TMP_PATH"
mv "$TMP_PATH" "$BINARY_PATH"
kill -USR2 "$(cat "{pid_file}")"
"#,
                binary = binary_path_str,
                pid_file = pid_file_str,
            )
        } else {
            format!(
                r#"BINARY_PATH="{binary}"
TMP_PATH="${{BINARY_PATH}}.new-$$"
curl -L "$RELEASE_URL" -o "$TMP_PATH"
chmod +x "$TMP_PATH"
command -v codesign >/dev/null 2>&1 && codesign --sign - --force "$TMP_PATH"
mv "$TMP_PATH" "$BINARY_PATH"
systemd-run --on-active=10s systemctl restart "{service}"
"#,
                binary = binary_path_str,
                service = metadata.service_name,
            )
        };

        Ok(DiscoveryTarget {
            plugin_type: plugin_ids::GENERIC_SHELL.clone(),
            plugin_config: serde_json::json!({
                "update_command": update_command,
                "resumable": true
            }),
            plugin_config_name: format!("{} shell update", metadata.service_name),
            roles: vec![PluginRole::ExecuteUpdate],
            package_identifier: None,
            config_override: None,
            execution_site: Some("agent".to_string()),
        })
    }

    fn build_docker_execute_target(
        &self,
        metadata: &ServiceMetadata,
        image: &str,
        container_name: &str,
    ) -> DiscoveryTarget {
        DiscoveryTarget {
            plugin_type: plugin_ids::RELEASES_DOCKER.clone(),
            plugin_config: serde_json::json!({
                "image": image,
                "container_name": container_name,
                "resumable": true
            }),
            plugin_config_name: format!("{} Docker update", metadata.service_name),
            roles: vec![PluginRole::ExecuteUpdate],
            package_identifier: None,
            config_override: None,
            execution_site: Some("agent".to_string()),
        }
    }
}
