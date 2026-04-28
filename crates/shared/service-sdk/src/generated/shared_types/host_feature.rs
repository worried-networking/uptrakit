// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt;
/// Fine-grained host capability, reported by the agent after probing.
///
/// Uses `Cow<'static, str>` so well-known constants are zero-allocation
/// borrows while unknown features from newer agents are owned strings.
/// Forward-compatible — unknown features stored losslessly.
///
/// NOT derived from OS family — the agent explicitly detects each feature.
/// This prevents misclassification of containers, minimal images, and
/// non-standard configurations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HostFeature(Cow<'static, str>);
impl HostFeature {
    /// Const constructor for well-known feature identifiers. Zero allocation.
    pub const fn from_static(s: &'static str) -> Self {
        Self(Cow::Borrowed(s))
    }
    /// Runtime constructor for features from DB/wire. Allocates.
    pub fn new(s: impl Into<String>) -> Self {
        Self(Cow::Owned(s.into()))
    }
    /// Returns the string representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for HostFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl From<String> for HostFeature {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}
impl AsRef<str> for HostFeature {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
/// Well-known host feature constants.
pub mod host_features {
    use super::HostFeature;
    /// POSIX-compatible shell (bash, sh, zsh). Agent checks: `sh -c true`.
    pub const POSIX_SHELL: HostFeature = HostFeature::from_static("posix_shell");
    /// Privilege escalation available. Agent checks: `sudo -n -l`.
    pub const PRIVILEGE_ESCALATION: HostFeature = HostFeature::from_static("privilege_escalation");
    /// Systemd init system. Agent checks: `systemctl --version`.
    pub const SYSTEMD: HostFeature = HostFeature::from_static("systemd");
    /// RouterOS CLI available. Groundwork only — no runtime implementation yet.
    pub const ROUTER_OS_CLI: HostFeature = HostFeature::from_static("router_os_cli");
}
/// Features that can be detected by running a command via `CommandExecutor`.
///
/// Each entry is `(feature, program, args)`. Features not in this list
/// (e.g., `router_os_cli`) require non-POSIX detection and are not probed
/// by the standard probing function.
pub const PROBEABLE_FEATURES: &[(HostFeature, &str, &[&str])] = &[
    (host_features::POSIX_SHELL, "sh", &["-c", "true"]),
    (host_features::PRIVILEGE_ESCALATION, "sudo", &["-n", "-l"]),
    (host_features::SYSTEMD, "systemctl", &["--version"]),
];
