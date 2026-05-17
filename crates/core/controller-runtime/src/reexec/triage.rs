//! Triage logic for deciding whether a config reload requires a full process
//! re-exec instead of an in-place hot-reload.
//!
//! Fields compared here are those whose change cannot be safely absorbed by
//! running reload handlers:
//!
//! - `db.url` — database connection string; the pool must be torn down and
//!   rebuilt against the new URL, which requires a fresh process.
//! - `master_key` — master encryption key source; the crypto subsystem
//!   does not support swapping keys at runtime.
//! - `log.path` — log file path; the tracing subscriber is initialized once
//!   at startup and cannot be re-pointed.
//! - `embedded_services` topology — enabling or disabling an embedded
//!   service (Agent, Agent-SSH, MQTT, Scheduler) requires re-registering
//!   subsystems that cannot be torn down in-process.
use uptrakit_config_reload::RuntimeConfig;

/// Decision returned by [`decide`].
#[derive(Debug)]
#[non_exhaustive]
pub(crate) struct ReexecDecision {
    /// Whether a re-exec is required.
    pub(crate) needed: bool,
    /// Human-readable reasons explaining why re-exec is needed.
    ///
    /// Empty when `needed` is `false`.
    pub(crate) reasons: Vec<&'static str>,
}

/// Inspect the difference between `prior` and `new` and decide whether the
/// delta requires a full process re-exec.
///
/// Returns a [`ReexecDecision`] with `needed = true` and one or more
/// `reasons` entries when at least one re-exec-forcing field changed.
#[must_use]
pub(crate) fn decide(prior: &RuntimeConfig, new: &RuntimeConfig) -> ReexecDecision {
    let mut reasons: Vec<&'static str> = Vec::new();

    if prior.db.url != new.db.url {
        reasons.push("db.url");
    }
    if prior.master_key != new.master_key {
        reasons.push("master_key");
    }
    if prior.log.path != new.log.path {
        reasons.push("log.path");
    }
    if prior.embedded_services != new.embedded_services {
        reasons.push("embedded_services topology");
    }

    ReexecDecision {
        needed: !reasons.is_empty(),
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_config_reload::config::{
        DbConfig, EmbeddedServicesConfig, LogConfig, RuntimeConfig,
    };
    use uptrakit_shared_types::SecretString;

    use super::decide;

    fn base_config() -> RuntimeConfig {
        let mut cfg = RuntimeConfig::default();
        cfg.db = DbConfig::new("sqlite:///var/lib/uptrakit/test.db");
        cfg.master_key = SecretString::new("file:/etc/uptrakit/master.key");
        cfg.log = LogConfig::new("/var/log/uptrakit/controller.log", "info");
        cfg.embedded_services = EmbeddedServicesConfig::default();
        cfg
    }

    #[test]
    fn identical_configs_no_reexec() {
        let cfg = base_config();
        let decision = decide(&cfg, &cfg.clone());
        assert!(!decision.needed);
        assert!(decision.reasons.is_empty());
    }

    #[test]
    fn db_url_change_requires_reexec() {
        let prior = base_config();
        let mut new = prior.clone();
        new.db = DbConfig::new("sqlite:///var/lib/uptrakit/other.db");
        let decision = decide(&prior, &new);
        assert!(decision.needed);
        assert!(decision.reasons.contains(&"db.url"));
    }

    #[test]
    fn master_key_change_requires_reexec() {
        let prior = base_config();
        let mut new = prior.clone();
        new.master_key = SecretString::new("file:/etc/uptrakit/new.key");
        let decision = decide(&prior, &new);
        assert!(decision.needed);
        assert!(decision.reasons.contains(&"master_key"));
    }

    #[test]
    fn log_path_change_requires_reexec() {
        let prior = base_config();
        let mut new = prior.clone();
        new.log = LogConfig::new("/var/log/uptrakit/other.log", "info");
        let decision = decide(&prior, &new);
        assert!(decision.needed);
        assert!(decision.reasons.contains(&"log.path"));
    }

    #[test]
    fn embedded_services_topology_change_requires_reexec() {
        let prior = base_config();
        let mut new = prior.clone();
        let mut svc = EmbeddedServicesConfig::default();
        svc.agent = true;
        new.embedded_services = svc;
        let decision = decide(&prior, &new);
        assert!(decision.needed);
        assert!(decision.reasons.contains(&"embedded_services topology"));
    }

    #[test]
    fn multiple_changes_reported() {
        let prior = base_config();
        let mut new = prior.clone();
        new.db = DbConfig::new("sqlite:///var/lib/uptrakit/other.db");
        new.master_key = SecretString::new("file:/etc/uptrakit/new.key");
        let decision = decide(&prior, &new);
        assert!(decision.needed);
        assert_eq!(decision.reasons.len(), 2);
    }
}
