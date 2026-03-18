use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{Result, UpstreamRelease, Version, execute_and_capture};

use crate::plugin::{SnapPlugin, is_prerelease_channel, parse_snap_info_channels};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for SnapPlugin {
    /// Fetch the latest release for a Snap package from a specific channel.
    ///
    /// Runs `snap info <name>` and parses the `channels:` section. Returns a
    /// single [`UpstreamRelease`] for the configured channel
    /// (default: `"latest/stable"`), or an empty vec if the snap is not
    /// available on that channel.
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching Snap releases via snap info");

        let stdout = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec("snap", ["info".to_string(), package_identifier.to_string()]),
            "snap info",
        )
        .await?;

        let channels = parse_snap_info_channels(&stdout);
        let target_channel = self.config.effective_channel();

        let Some(info) = channels.get(target_channel) else {
            tracing::debug!(
                package = %package_identifier,
                channel = %target_channel,
                "snap not available on channel"
            );
            return Ok(vec![]);
        };

        let is_prerelease = is_prerelease_channel(target_channel);
        tracing::debug!(
            version = %info.version,
            channel = %target_channel,
            is_prerelease,
            "Snap upstream version resolved"
        );

        Ok(vec![{
            let mut release = UpstreamRelease::new(
                Version::new(&info.version),
                info.version.clone(),
                is_prerelease,
                "",
            );
            release.published_at = info.published_at;
            release
        }])
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use uptrakit_plugin_infrastructure_core::command::{
        CommandExecutor, CommandOutput, CommandSpec,
    };
    use uptrakit_plugin_infrastructure_core::mpsc;
    use uptrakit_plugin_infrastructure_core::{
        HostCapabilities, HostRuntime, PosixHostRuntime, ReleaseFetcher, UpdateOutputLine,
    };

    use crate::config::SnapConfig;
    use crate::plugin::SnapPlugin;

    /// Mock executor that always returns Ok (even for non-zero exit codes).
    struct FixedOutputExecutor {
        output: String,
        exit_code: i32,
    }

    #[async_trait]
    impl CommandExecutor for FixedOutputExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output.clone(),
                exit_code: self.exit_code,
            })
        }

        async fn execute_quiet(
            &self,
            _spec: &CommandSpec,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output.clone(),
                exit_code: self.exit_code,
            })
        }
    }

    fn make_plugin(config: SnapConfig, stdout: &str, exit_code: i32) -> SnapPlugin {
        let executor = Arc::new(FixedOutputExecutor {
            output: stdout.to_string(),
            exit_code,
        }) as Arc<dyn CommandExecutor>;
        let caps = HostCapabilities::default();
        let runtime = Arc::new(PosixHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        SnapPlugin::new(config, runtime).unwrap()
    }

    // ── fetch_releases ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_releases_latest_stable() {
        let output = concat!(
            "name:    vlc\n",
            "channels:\n",
            "  latest/stable: 3.0.20 2024-01-12 (2359) 215MB -\n",
            "  latest/edge:   3.0.21 2024-01-15 (2400) 216MB -\n",
        );
        let plugin = make_plugin(SnapConfig::default(), output, 0);

        let releases = plugin.fetch_releases("vlc").await.unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, "3.0.20");
        assert!(!releases[0].is_prerelease);
    }

    #[tokio::test]
    async fn fetch_releases_edge_channel_is_prerelease() {
        let output = "channels:\n  latest/edge: 3.0.21 2024-01-15 (2400) 216MB -\n";
        let plugin = make_plugin(
            SnapConfig {
                channel: Some("latest/edge".to_string()),
            },
            output,
            0,
        );

        let releases = plugin.fetch_releases("vlc").await.unwrap();
        assert_eq!(releases.len(), 1);
        assert!(releases[0].is_prerelease);
    }

    #[tokio::test]
    async fn fetch_releases_channel_not_in_output_returns_empty() {
        let output = "channels:\n  latest/stable: 3.0.20 2024-01-12 (2359) 215MB -\n";
        let plugin = make_plugin(
            SnapConfig {
                channel: Some("1.0/stable".to_string()),
            },
            output,
            0,
        );

        let releases = plugin.fetch_releases("vlc").await.unwrap();
        assert!(releases.is_empty());
    }
}
