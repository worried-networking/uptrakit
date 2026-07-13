//! Validate-reject gate for `[nats]` config changes.
//!
//! [`NatsReloadable`] is a validate-reject gate for `[nats]` config changes.
//!
//! NATS URL hot-reload is intentionally unsupported (see
//! docs/development/nats-integration.md): the live consumers hold a client
//! captured at boot with no swap seam. File-sourced `nats.url` changes are
//! preempted by reexec triage (`reexec/triage.rs`); this gate is the backstop
//! for delta sources that bypass triage.
//! // unreachable until DbBump wires nats sections — sections_to_deltas has
//! // no `nats` arm today; kept so a future wiring cannot silently no-op.

#![cfg(feature = "nats")]

use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use uptrakit_config_reload::config::NatsConfig;
use uptrakit_config_reload::defaults::WATCHDOG_NATS;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::Reloadable;

/// Validate-reject gate for `[nats]` config changes.
///
/// NATS URL hot-reload is intentionally unsupported (see
/// docs/development/nats-integration.md): the live consumers hold a client
/// captured at boot with no swap seam. File-sourced `nats.url` changes are
/// preempted by reexec triage (`reexec/triage.rs`); this gate is the backstop
/// for delta sources that bypass triage.
/// // unreachable until DbBump wires nats sections — sections_to_deltas has
/// // no `nats` arm today; kept so a future wiring cannot silently no-op.
pub(crate) struct NatsReloadable {
    /// URL the running transport was built with (DB-reconciled effective
    /// value, not necessarily the boot file's); `None` = NATS unconfigured.
    effective_url: Option<String>,
    /// The `[nats]` section the process booted with — gates non-URL fields
    /// (`extra`): `build_deltas` emits a Nats delta on ANY section change
    /// (`prior.nats != new.nats`, whole-struct compare), so a URL-only gate
    /// would report APPLIED for an `extra`-only edit while changing nothing —
    /// the same vacuous class this spec kills.
    boot: NatsConfig,
}

impl NatsReloadable {
    pub(crate) fn new(effective_url: Option<String>, boot: NatsConfig) -> Self {
        Self {
            effective_url,
            boot,
        }
    }
}

impl Reloadable for NatsReloadable {
    type Config = NatsConfig;

    fn name(&self) -> &'static str {
        "nats"
    }

    fn validate(&self, new: &NatsConfig) -> Result<(), Report> {
        let new_url = (!new.url.is_empty()).then(|| new.url.clone());
        if new_url != self.effective_url {
            bail!(ConfigReloadError::Validate(
                "nats.url change requires reexec".to_string()
            ));
        }
        if new.extra != self.boot.extra {
            bail!(ConfigReloadError::Validate(
                "nats config change requires restart".to_string()
            ));
        }
        Ok(())
    }

    async fn apply(&self, _new: Arc<NatsConfig>) -> Result<(), Report> {
        // URL unchanged (validate gates any change); nothing to apply.
        Ok(())
    }

    async fn revert(&self) -> Result<(), Report> {
        Ok(())
    }

    async fn health_check(&self) -> Result<(), Report> {
        // No state is mutated by apply; health is always OK if validate
        // passed (same contract as EmbeddedServicesReloadable).
        Ok(())
    }

    fn rollback_window(&self) -> Duration {
        WATCHDOG_NATS
    }
}

uptrakit_config_reload::reloadable_erased_impl!(NatsReloadable, RuntimeConfigDelta::Nats);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn boot_with_url(url: &str) -> NatsConfig {
        NatsConfig::new(url)
    }

    #[test]
    fn nats_validate_rejects_url_change() {
        let reloadable = NatsReloadable::new(
            Some("nats://old:4222".into()),
            boot_with_url("nats://old:4222"),
        );
        let new_cfg = NatsConfig::new("nats://new:4222");
        let err = reloadable.validate(&new_cfg).unwrap_err();
        assert!(err.to_string().contains("nats.url"), "err: {err}");
    }

    #[test]
    fn nats_validate_accepts_unchanged_url() {
        let reloadable = NatsReloadable::new(
            Some("nats://old:4222".into()),
            boot_with_url("nats://old:4222"),
        );
        let new_cfg = NatsConfig::new("nats://old:4222");
        reloadable.validate(&new_cfg).unwrap();
    }

    #[test]
    fn nats_validate_rejects_url_change_when_unconfigured() {
        let reloadable = NatsReloadable::new(None, NatsConfig::default());
        let new_cfg = NatsConfig::new("nats://new:4222");
        let err = reloadable.validate(&new_cfg).unwrap_err();
        assert!(err.to_string().contains("nats.url"), "err: {err}");
    }

    #[test]
    fn nats_validate_accepts_still_unconfigured() {
        let reloadable = NatsReloadable::new(None, NatsConfig::default());
        let new_cfg = NatsConfig::default(); // url is empty
        reloadable.validate(&new_cfg).unwrap();
    }

    #[test]
    fn nats_validate_rejects_extra_change() {
        let reloadable = NatsReloadable::new(
            Some("nats://old:4222".into()),
            boot_with_url("nats://old:4222"),
        );
        let mut new_cfg = NatsConfig::new("nats://old:4222");
        new_cfg
            .extra
            .insert("unknown_key".into(), toml::Value::String("val".into()));
        let err = reloadable.validate(&new_cfg).unwrap_err();
        assert!(err.to_string().contains("nats config"), "err: {err}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn nats_apply_never_connects() {
        let url = "nats://unreachable.invalid:4222";
        let reloadable = NatsReloadable::new(Some(url.into()), boot_with_url(url));
        let new_cfg = Arc::new(NatsConfig::new(url));
        reloadable.apply(new_cfg).await.unwrap();
    }
}
