//! Host compatibility requirements and validation.
//!
//! [`HostRequirements`] lives on each [`RoleSlot`](crate::descriptor::RoleSlot),
//! not on the descriptor itself. A single plugin can have roles with different
//! execution requirements (e.g., Proxmox controller-only `ReleaseFetcher` +
//! agent-side `InfraBundle` requiring Linux).
//!
//! [`RoleKey`] is a typed discriminant for per-instance plugin roles, used by
//! `validate_role_compatibility()` on `PluginMetadataOps`.

use rootcause::prelude::*;
use uptrakit_shared_types::{HostCapabilities, HostFeature, OsFamily, PluginRole, host_features};

use crate::error::PluginError;

/// Typed key for per-instance plugin roles.
///
/// Used by `validate_role_compatibility()` and `host_requirements_for_role()`
/// to select the correct `RoleSlot` without string matching.
/// Excludes singleton roles (transport, lifecycle, infra) which are not
/// assigned to hosts.
///
/// `#[non_exhaustive]` per project convention — new per-instance role types
/// may be added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RoleKey {
    Discoverer,
    VersionDetector,
    ReleaseFetcher,
    PackageIndexer,
    UpdateExecutor,
    LifecycleHook,
}

impl RoleKey {
    /// Convert a [`PluginRole`] to its corresponding [`RoleKey`].
    ///
    /// Returns `None` for [`PluginRole::Other`] and any future unknown variants,
    /// since those have no mapping to a typed role key.
    pub fn from_plugin_role(role: &PluginRole) -> Option<Self> {
        match role {
            PluginRole::DetectVersion => Some(Self::VersionDetector),
            PluginRole::FetchReleases => Some(Self::ReleaseFetcher),
            PluginRole::ExecuteUpdate => Some(Self::UpdateExecutor),
            PluginRole::PreUpdateHook | PluginRole::PostUpdateHook => Some(Self::LifecycleHook),
            PluginRole::Other(_) => None,
            // Future unknown variants per #[non_exhaustive]
            _ => {
                tracing::warn!("unknown PluginRole variant encountered in from_plugin_role");
                None
            }
        }
    }
}

impl std::fmt::Display for RoleKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Discoverer => write!(f, "discoverer"),
            Self::VersionDetector => write!(f, "version_detector"),
            Self::ReleaseFetcher => write!(f, "release_fetcher"),
            Self::PackageIndexer => write!(f, "package_indexer"),
            Self::UpdateExecutor => write!(f, "update_executor"),
            Self::LifecycleHook => write!(f, "lifecycle_hook"),
        }
    }
}

/// Error returned when a host does not meet a role's requirements.
///
/// Uses the project's standard error pattern: `rootcause::Report<HostCompatibilityError>`
/// for propagation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HostCompatibilityError {
    #[error("incompatible OS family: {actual:?} (expected one of {expected:?})")]
    IncompatibleOsFamily {
        actual: OsFamily,
        expected: &'static [OsFamily],
    },
    #[error("host OS family unknown")]
    UnknownOsFamily,
    #[error("host lacks required feature: {0:?}")]
    MissingFeature(HostFeature),
    #[error("{plugin_type} does not support role: {role}")]
    UnsupportedRole { plugin_type: String, role: RoleKey },
}

uptrakit_shared_macros::impl_report_conversion!(
    HostCompatibilityError => PluginError,
    |e: HostCompatibilityError| PluginError::UnsupportedOperation(e.to_string())
);

/// What a role needs from its target host.
///
/// Lives on `RoleSlot`, not on `PluginDescriptor`. Validated by the framework
/// at assignment time (controller) using the host's `HostCapabilities`.
///
/// Construction uses named constants (`POSIX`, `POSIX_PRIVILEGED`, `CONTROLLER_ONLY`)
/// for common cases and `const fn new()` for custom combinations.
pub struct HostRequirements {
    /// Compatible OS families. Empty = any OS family.
    pub os_families: &'static [OsFamily],
    /// Required host features. All must be present.
    /// Only checked when `features` is non-empty in `HostCapabilities`
    /// (i.e., the agent has reported). Legacy agents with empty features
    /// skip this check to avoid rejecting existing assignments.
    pub required_features: &'static [HostFeature],
    /// Role runs on controller only, no host access needed.
    /// When true, `os_families` and `required_features` are ignored.
    pub controller_only: bool,
}

/// Feature arrays for `HostRequirements` constants.
///
/// Extracted as `static` items because `HostFeature` contains `Cow<'static, str>`
/// whose destructor cannot be evaluated at compile-time, preventing inline
/// `&[HostFeature]` temporaries in `const` contexts.
mod feature_arrays {
    use super::*;

    pub(super) static POSIX_SHELL: [HostFeature; 1] = [host_features::POSIX_SHELL];
    pub(super) static POSIX_SHELL_AND_SUDO: [HostFeature; 2] = [
        host_features::POSIX_SHELL,
        host_features::PRIVILEGE_ESCALATION,
    ];
}

impl HostRequirements {
    /// Construct custom host requirements. Usable in `const` / `static` contexts.
    pub const fn new(
        os_families: &'static [OsFamily],
        required_features: &'static [HostFeature],
        controller_only: bool,
    ) -> Self {
        Self {
            os_families,
            required_features,
            controller_only,
        }
    }

    /// Controller-only role — no host access needed.
    pub const CONTROLLER_ONLY: Self = Self::new(&[], &[], true);

    /// Alias for `CONTROLLER_ONLY`.
    pub const NONE: Self = Self::CONTROLLER_ONLY;

    /// Standard POSIX host (Linux, macOS, FreeBSD) with shell access.
    pub const POSIX: Self = Self::new(
        &[OsFamily::Linux, OsFamily::MacOs, OsFamily::FreeBsd],
        &feature_arrays::POSIX_SHELL,
        false,
    );

    /// POSIX host with privilege escalation (sudo).
    pub const POSIX_PRIVILEGED: Self = Self::new(
        &[OsFamily::Linux, OsFamily::MacOs, OsFamily::FreeBsd],
        &feature_arrays::POSIX_SHELL_AND_SUDO,
        false,
    );

    /// Validate that the given host capabilities satisfy these requirements.
    pub fn is_compatible_with(
        &self,
        caps: &HostCapabilities,
    ) -> std::result::Result<(), Report<HostCompatibilityError>> {
        if self.controller_only {
            return Ok(());
        }

        // OS family check — always applied (derived from host.os_type, always available)
        if !self.os_families.is_empty() {
            match &caps.os_family {
                Some(f) if self.os_families.contains(f) => {}
                Some(f) => {
                    return Err(report!(HostCompatibilityError::IncompatibleOsFamily {
                        actual: *f,
                        expected: self.os_families,
                    }));
                }
                None => return Err(report!(HostCompatibilityError::UnknownOsFamily)),
            }
        }

        // Feature check — only when the agent has reported features.
        // Legacy agents have empty features; skipping avoids rejecting existing assignments.
        if !caps.features.is_empty() {
            for feature in self.required_features {
                if !caps.has_feature(feature) {
                    return Err(report!(HostCompatibilityError::MissingFeature(
                        feature.clone()
                    )));
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — assert!(result.is_ok()) is idiomatic in tests"
    )]

    use super::*;
    use std::collections::BTreeSet;

    fn linux_caps_with_features(features: &[HostFeature]) -> HostCapabilities {
        HostCapabilities {
            os_family: Some(OsFamily::Linux),
            os_version: None,
            architecture: None,
            features: features.iter().cloned().collect(),
        }
    }

    #[test]
    fn controller_only_always_compatible() {
        let caps = HostCapabilities::default();
        assert!(
            HostRequirements::CONTROLLER_ONLY
                .is_compatible_with(&caps)
                .is_ok()
        );
    }

    #[test]
    fn posix_compatible_with_linux_shell() {
        let caps = linux_caps_with_features(&[host_features::POSIX_SHELL]);
        assert!(HostRequirements::POSIX.is_compatible_with(&caps).is_ok());
    }

    #[test]
    fn posix_incompatible_with_routeros() {
        let caps = HostCapabilities {
            os_family: Some(OsFamily::RouterOs),
            features: BTreeSet::new(),
            ..Default::default()
        };
        assert!(HostRequirements::POSIX.is_compatible_with(&caps).is_err());
    }

    #[test]
    fn posix_privileged_requires_sudo() {
        let caps = linux_caps_with_features(&[host_features::POSIX_SHELL]);
        assert!(
            HostRequirements::POSIX_PRIVILEGED
                .is_compatible_with(&caps)
                .is_err()
        );

        let caps_with_sudo = linux_caps_with_features(&[
            host_features::POSIX_SHELL,
            host_features::PRIVILEGE_ESCALATION,
        ]);
        assert!(
            HostRequirements::POSIX_PRIVILEGED
                .is_compatible_with(&caps_with_sudo)
                .is_ok()
        );
    }

    #[test]
    fn empty_features_skips_feature_check() {
        // Legacy agent: features empty, should still be compatible on OS family alone
        let caps = HostCapabilities {
            os_family: Some(OsFamily::Linux),
            features: BTreeSet::new(),
            ..Default::default()
        };
        assert!(
            HostRequirements::POSIX_PRIVILEGED
                .is_compatible_with(&caps)
                .is_ok()
        );
    }

    #[test]
    fn unknown_os_family_rejected() {
        let caps = HostCapabilities::default(); // os_family: None
        assert!(HostRequirements::POSIX.is_compatible_with(&caps).is_err());
    }

    #[test]
    fn custom_requirements() {
        static REQUIRED: [HostFeature; 2] = [host_features::POSIX_SHELL, host_features::SYSTEMD];
        let reqs = HostRequirements::new(&[OsFamily::Linux], &REQUIRED, false);
        let caps = linux_caps_with_features(&[host_features::POSIX_SHELL, host_features::SYSTEMD]);
        assert!(reqs.is_compatible_with(&caps).is_ok());

        let caps_no_systemd = linux_caps_with_features(&[host_features::POSIX_SHELL]);
        assert!(reqs.is_compatible_with(&caps_no_systemd).is_err());
    }

    #[test]
    fn role_key_display() {
        assert_eq!(RoleKey::Discoverer.to_string(), "discoverer");
        assert_eq!(RoleKey::UpdateExecutor.to_string(), "update_executor");
    }

    #[test]
    fn from_plugin_role_known_variants() {
        assert_eq!(
            RoleKey::from_plugin_role(&PluginRole::DetectVersion),
            Some(RoleKey::VersionDetector),
        );
        assert_eq!(
            RoleKey::from_plugin_role(&PluginRole::FetchReleases),
            Some(RoleKey::ReleaseFetcher),
        );
        assert_eq!(
            RoleKey::from_plugin_role(&PluginRole::ExecuteUpdate),
            Some(RoleKey::UpdateExecutor),
        );
        assert_eq!(
            RoleKey::from_plugin_role(&PluginRole::PreUpdateHook),
            Some(RoleKey::LifecycleHook),
        );
        assert_eq!(
            RoleKey::from_plugin_role(&PluginRole::PostUpdateHook),
            Some(RoleKey::LifecycleHook),
        );
    }

    #[test]
    fn from_plugin_role_other_returns_none() {
        assert_eq!(
            RoleKey::from_plugin_role(&PluginRole::Other("custom_role".to_string())),
            None,
        );
    }
}
