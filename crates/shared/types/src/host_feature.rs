use serde::{Deserialize, Serialize};

/// Fine-grained host capability, reported by the agent after probing.
///
/// NOT derived from OS family — the agent explicitly detects each feature.
/// This prevents misclassification of containers, minimal images, and
/// non-standard configurations.
///
/// All variants are `Copy`, enabling `&'static [HostFeature]` in role slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum HostFeature {
    /// POSIX-compatible shell (bash, sh, zsh). Agent checks: `which sh`.
    PosixShell,
    /// Privilege escalation available. Agent checks: `sudo -n true`.
    PrivilegeEscalation,
    /// Systemd init system. Agent checks: `systemctl --version`.
    Systemd,
    /// RouterOS CLI available. Agent checks: SSH banner or `/system/identity/print`.
    /// Groundwork only — no runtime implementation yet.
    RouterOsCli,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialization_roundtrip() {
        let cases = [
            (HostFeature::PosixShell, r#""posix_shell""#),
            (
                HostFeature::PrivilegeEscalation,
                r#""privilege_escalation""#,
            ),
            (HostFeature::Systemd, r#""systemd""#),
            (HostFeature::RouterOsCli, r#""router_os_cli""#),
        ];
        for (feature, expected) in cases {
            let json = serde_json::to_string(&feature).expect("serialize");
            assert_eq!(json, expected);
            let de: HostFeature = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(de, feature);
        }
    }

    #[test]
    fn ordering_is_deterministic() {
        let mut features = [
            HostFeature::Systemd,
            HostFeature::PosixShell,
            HostFeature::PrivilegeEscalation,
        ];
        features.sort();
        assert_eq!(features[0], HostFeature::PosixShell);
    }

    #[test]
    fn unknown_feature_string_fails_deserialization() {
        let result = serde_json::from_str::<HostFeature>(r#""unknown_feature""#);
        assert!(result.is_err());
    }
}
