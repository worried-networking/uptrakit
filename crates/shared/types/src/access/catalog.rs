//! Built-in action catalog: single-source macro emitting `Resource`, the
//! validity matrix, per-action metadata, and (Task 4) typed constants.

use std::str::FromStr;

use super::is_valid_segment_path;
use super::selector::SelectorSupport;
use super::verb::Verb;

/// One built-in resource row of the catalog.
///
/// `#[non_exhaustive]` — rows will gain metadata fields. Documented
/// exception to the required-constructor rule
/// (`docs/development/coding-standards.md`): rows are static data emitted
/// only by the in-crate catalog macro; a foreign `CatalogEntry` would be a
/// fake catalog row, so no constructor is provided — consumers read fields.
#[non_exhaustive]
#[derive(Debug)]
pub struct CatalogEntry {
    pub resource: Resource,
    pub resource_str: &'static str,
    pub verbs: &'static [VerbEntry],
}

/// One valid (resource, verb) action of the catalog, with per-action
/// metadata. Same `#[non_exhaustive]`-without-constructor exception as
/// [`CatalogEntry`].
#[non_exhaustive]
#[derive(Debug)]
pub struct VerbEntry {
    pub verb: Verb,
    pub action_str: &'static str,
    pub description: &'static str,
    pub selector_support: SelectorSupport,
}

/// Error returned when parsing an invalid [`Resource`] string.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseResourceError {
    /// Not a catalog resource and not a dynamic namespace.
    #[error("unknown resource")]
    UnknownResource,
    /// `plugin.` / `surface.` prefix with nothing after it.
    #[error("empty dynamic remainder")]
    EmptyDynamicRemainder,
    /// A segment violates the kebab-case grammar.
    #[error("invalid resource segment grammar")]
    InvalidSegment,
}

macro_rules! access_catalog {
    (
        $(
            $variant:ident, $res_str:literal, {
                $( $verb:ident => ($verb_str:literal, $support:ident, $const_name:ident, $const_str_name:ident, $desc:literal) ),+ $(,)?
            }
        );+ $(;)?
    ) => {
        /// A resource in the access model: a built-in catalog variant or a
        /// dynamic-namespace resource.
        ///
        /// `#[non_exhaustive]`: new domains add variants. The dynamic
        /// variants are additionally variant-sealed — construct them only
        /// via [`Resource::plugin`] / [`Resource::surface`] / `FromStr`
        /// so the prefix + grammar validation cannot be bypassed.
        #[non_exhaustive]
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub enum Resource {
            $( $variant, )+
            /// `plugin.<plugin_type>` — stores the full resource string.
            #[non_exhaustive]
            Plugin(String),
            /// `surface.<surface_id>` — stores the full resource string.
            #[non_exhaustive]
            Surface(String),
        }

        impl Resource {
            /// Canonical wire string (`&'static` for built-ins).
            pub fn as_str(&self) -> &str {
                match self {
                    $( Resource::$variant => $res_str, )+
                    Resource::Plugin(s) => s.as_str(),
                    Resource::Surface(s) => s.as_str(),
                }
            }

            /// Verbs the validity matrix allows. Dynamic resources accept
            /// the full closed set at the type level; registry narrowing
            /// is a decision-time concern (M1.3+).
            pub fn allowed_verbs(&self) -> &'static [Verb] {
                match self {
                    $( Resource::$variant => &[ $( Verb::$verb ),+ ], )+
                    Resource::Plugin(_) | Resource::Surface(_) => Verb::ALL,
                }
            }
        }

        /// The built-in action catalog. Single source of truth for the
        /// validity matrix, per-action descriptions, and selector support.
        pub const CATALOG: &[CatalogEntry] = &[
            $(
                CatalogEntry {
                    resource: Resource::$variant,
                    resource_str: $res_str,
                    verbs: &[
                        $(
                            VerbEntry {
                                verb: Verb::$verb,
                                action_str: concat!($res_str, ":", $verb_str),
                                description: $desc,
                                selector_support: SelectorSupport::$support,
                            },
                        )+
                    ],
                },
            )+
        ];

        /// Typed constants for every valid built-in action, plus the
        /// paired `&'static str` action strings (for OpenAPI security
        /// declarations and the CI scope dictionary, which need
        /// compile-time strings; consts must be consumed via macro
        /// expansion, not literal-only attribute slots — spec note).
        pub mod actions {
            use crate::access::{Action, Verb};

            use super::Resource;

            $( $(
                pub const $const_name: Action = Action {
                    resource: Resource::$variant,
                    verb: Verb::$verb,
                };
                pub const $const_str_name: &str = concat!($res_str, ":", $verb_str);
            )+ )+
        }
    };
}

access_catalog! {
    Services, "services", {
        Read => ("read", None, SERVICES_READ, SERVICES_READ_STR, "View tenant services and their status"),
        Approve => ("approve", None, SERVICES_APPROVE, SERVICES_APPROVE_STR, "Approve pending service enrollments"),
        Reject => ("reject", None, SERVICES_REJECT, SERVICES_REJECT_STR, "Reject pending service enrollments"),
        Delete => ("delete", None, SERVICES_DELETE, SERVICES_DELETE_STR, "Deactivate/remove services"),
        Update => ("update", None, SERVICES_UPDATE, SERVICES_UPDATE_STR, "Update service settings (ping interval, freeze, merge)"),
    };
    SystemServices, "system.services", {
        Read => ("read", None, SYSTEM_SERVICES_READ, SYSTEM_SERVICES_READ_STR, "View system services (MQTT bridge, external scheduler)"),
        Approve => ("approve", None, SYSTEM_SERVICES_APPROVE, SYSTEM_SERVICES_APPROVE_STR, "Approve pending system services"),
        Reject => ("reject", None, SYSTEM_SERVICES_REJECT, SYSTEM_SERVICES_REJECT_STR, "Reject pending system services"),
        Delete => ("delete", None, SYSTEM_SERVICES_DELETE, SYSTEM_SERVICES_DELETE_STR, "Deactivate system services"),
        Update => ("update", None, SYSTEM_SERVICES_UPDATE, SYSTEM_SERVICES_UPDATE_STR, "Update system service settings"),
    };
    Software, "software", {
        Read => ("read", None, SOFTWARE_READ, SOFTWARE_READ_STR, "View software items, plugin configs, history"),
        Create => ("create", None, SOFTWARE_CREATE, SOFTWARE_CREATE_STR, "Create software items and plugin configs"),
        Update => ("update", None, SOFTWARE_UPDATE, SOFTWARE_UPDATE_STR, "Edit software items and plugin configs"),
        Delete => ("delete", None, SOFTWARE_DELETE, SOFTWARE_DELETE_STR, "Delete software items and plugin configs"),
    };
    Checks, "checks", {
        Trigger => ("trigger", HostAndSoftware, CHECKS_TRIGGER, CHECKS_TRIGGER_STR, "Trigger version checks and autodiscovery"),
    };
    Updates, "updates", {
        Trigger => ("trigger", HostAndSoftware, UPDATES_TRIGGER, UPDATES_TRIGGER_STR, "Trigger update execution (single and batch)"),
    };
    Scheduler, "scheduler", {
        Manage => ("manage", None, SCHEDULER_MANAGE, SCHEDULER_MANAGE_STR, "Manage scheduled tasks"),
    };
    Hosts, "hosts", {
        Read => ("read", Host, HOSTS_READ, HOSTS_READ_STR, "View hosts"),
        Update => ("update", Host, HOSTS_UPDATE, HOSTS_UPDATE_STR, "Update host properties"),
        Delete => ("delete", Host, HOSTS_DELETE, HOSTS_DELETE_STR, "Deactivate hosts"),
    };
    HostTags, "hosts.tags", {
        Manage => ("manage", None, HOSTS_TAGS_MANAGE, HOSTS_TAGS_MANAGE_STR, "Create, edit, delete, and assign host tags (tag-scoped grants make this access-control authority)"),
    };
    Settings, "settings", {
        Read => ("read", None, SETTINGS_READ, SETTINGS_READ_STR, "View all tenant settings"),
    };
    SettingsAuth, "settings.auth", {
        Manage => ("manage", None, SETTINGS_AUTH_MANAGE, SETTINGS_AUTH_MANAGE_STR, "Manage registration, authentication, OIDC providers"),
    };
    SettingsEnrollmentTokens, "settings.enrollment-tokens", {
        Manage => ("manage", None, SETTINGS_ENROLLMENT_TOKENS_MANAGE, SETTINGS_ENROLLMENT_TOKENS_MANAGE_STR, "Manage tenant enrollment tokens"),
    };
    SettingsCertificates, "settings.certificates", {
        Manage => ("manage", None, SETTINGS_CERTIFICATES_MANAGE, SETTINGS_CERTIFICATES_MANAGE_STR, "Manage agent certificate settings"),
    };
    SystemSettings, "system.settings", {
        Manage => ("manage", None, SYSTEM_SETTINGS_MANAGE, SYSTEM_SETTINGS_MANAGE_STR, "Manage global infrastructure settings"),
    };
    Commands, "commands", {
        Manage => ("manage", None, COMMANDS_MANAGE, COMMANDS_MANAGE_STR, "Modify command-bearing plugin config fields (code execution authority)"),
    };
    PluginConfigs, "plugin-configs", {
        Trigger => ("trigger", None, PLUGIN_CONFIGS_TRIGGER, PLUGIN_CONFIGS_TRIGGER_STR, "Test plugin configurations against hosts (dry-run validation)"),
    };
    Notifications, "notifications", {
        Read => ("read", None, NOTIFICATIONS_READ, NOTIFICATIONS_READ_STR, "View notification channels, rules, log"),
        Manage => ("manage", None, NOTIFICATIONS_MANAGE, NOTIFICATIONS_MANAGE_STR, "Create/modify notification channels and rules; SMTP settings"),
    };
    Audit, "audit", {
        Read => ("read", None, AUDIT_READ, AUDIT_READ_STR, "View tenant-scoped audit log entries"),
    };
    SystemAudit, "system.audit", {
        Read => ("read", None, SYSTEM_AUDIT_READ, SYSTEM_AUDIT_READ_STR, "View system-level audit log entries"),
    };
    Users, "users", {
        Manage => ("manage", None, USERS_MANAGE, USERS_MANAGE_STR, "Manage user lifecycle (activate/deactivate, MFA resets, email changes)"),
    };
    Access, "access", {
        Manage => ("manage", None, ACCESS_MANAGE, ACCESS_MANAGE_STR, "Manage grants, roles, and role assignments (authority administration)"),
    };
    DiscoveryIgnores, "discovery.ignores", {
        Manage => ("manage", None, DISCOVERY_IGNORES_MANAGE, DISCOVERY_IGNORES_MANAGE_STR, "Manage autodiscovery ignore rules"),
    };
    Mcp, "mcp", {
        Use => ("use", None, MCP_USE, MCP_USE_STR, "Access the MCP server endpoint"),
    };
    SystemConfigState, "system.config-state", {
        Read => ("read", None, SYSTEM_CONFIG_STATE_READ, SYSTEM_CONFIG_STATE_READ_STR, "View instance config reload state"),
        Manage => ("manage", None, SYSTEM_CONFIG_STATE_MANAGE, SYSTEM_CONFIG_STATE_MANAGE_STR, "Manage instance config reload state (clear degraded)"),
    };
}

impl Resource {
    /// Whether this is a `system.`-plane resource (excluded from the `*`
    /// wildcard; `06-grant-model.md` §Grant patterns). Dynamic resources
    /// are never system-plane.
    pub fn is_system(&self) -> bool {
        self.as_str().starts_with("system.")
    }

    /// The plugin-type remainder of a `Plugin` resource.
    pub fn plugin_type(&self) -> Option<&str> {
        match self {
            Resource::Plugin(s) => s.strip_prefix("plugin."),
            _ => None,
        }
    }

    /// The surface-id remainder of a `Surface` resource.
    pub fn surface_id(&self) -> Option<&str> {
        match self {
            Resource::Surface(s) => s.strip_prefix("surface."),
            _ => None,
        }
    }

    /// Builds a `plugin.` resource from a plugin-type remainder — the
    /// remainder only (`"package-manager.apt"`), never a pre-prefixed
    /// string.
    pub fn plugin(plugin_type: &str) -> Result<Self, ParseResourceError> {
        Self::dynamic("plugin.", plugin_type).map(Resource::Plugin)
    }

    /// Builds a `surface.` resource from a surface-id remainder — the
    /// remainder only (`"proxmox.hosts"`), never a pre-prefixed string.
    pub fn surface(surface_id: &str) -> Result<Self, ParseResourceError> {
        Self::dynamic("surface.", surface_id).map(Resource::Surface)
    }

    fn dynamic(prefix: &str, remainder: &str) -> Result<String, ParseResourceError> {
        if remainder.is_empty() {
            return Err(ParseResourceError::EmptyDynamicRemainder);
        }
        if !is_valid_segment_path(remainder) {
            return Err(ParseResourceError::InvalidSegment);
        }
        Ok(format!("{prefix}{remainder}"))
    }
}

impl FromStr for Resource {
    type Err = ParseResourceError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(entry) = CATALOG.iter().find(|e| e.resource_str == s) {
            return Ok(entry.resource.clone());
        }
        if let Some(rest) = s.strip_prefix("plugin.") {
            return Self::plugin(rest);
        }
        if let Some(rest) = s.strip_prefix("surface.") {
            return Self::surface(rest);
        }
        Err(ParseResourceError::UnknownResource)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_non_empty_with_known_members() {
        assert!(!CATALOG.is_empty());
        assert!(CATALOG.iter().any(|e| e.resource_str == "hosts"));
        assert!(CATALOG.iter().any(|e| e.resource_str == "updates"));
        assert!(CATALOG.iter().any(|e| e.resource_str == "access"));
    }

    #[test]
    fn every_resource_string_round_trips() {
        for entry in CATALOG {
            let parsed: Resource = entry.resource_str.parse().expect("parses");
            assert_eq!(parsed, entry.resource);
            assert_eq!(parsed.as_str(), entry.resource_str);
        }
    }

    #[test]
    fn every_resource_string_satisfies_grammar() {
        for entry in CATALOG {
            assert!(
                crate::access::is_valid_segment_path(entry.resource_str),
                "grammar violation in catalog: {}",
                entry.resource_str
            );
        }
    }

    #[test]
    fn action_strings_consistent_with_parts() {
        for entry in CATALOG {
            for ve in entry.verbs {
                let expected = format!("{}:{}", entry.resource_str, ve.verb.as_str());
                assert_eq!(ve.action_str, expected, "row/verb literal mismatch");
            }
        }
    }

    #[test]
    fn selector_capable_actions_are_exactly_five() {
        let capable: Vec<&str> = CATALOG
            .iter()
            .flat_map(|e| e.verbs.iter())
            .filter(|ve| !matches!(ve.selector_support, SelectorSupport::None))
            .map(|ve| ve.action_str)
            .collect();
        assert_eq!(
            capable,
            [
                "checks:trigger",
                "updates:trigger",
                "hosts:read",
                "hosts:update",
                "hosts:delete",
            ]
        );
    }

    #[test]
    fn system_plane_rows_report_is_system() {
        for entry in CATALOG {
            assert_eq!(
                entry.resource.is_system(),
                entry.resource_str.starts_with("system."),
                "{}",
                entry.resource_str
            );
        }
    }

    #[test]
    fn scope_token_charset_property() {
        // A15: every action string is a valid RFC 6749 scope token
        // (NQCHAR: %x21 / %x23-5B / %x5D-7E).
        for entry in CATALOG {
            for ve in entry.verbs {
                for b in ve.action_str.bytes() {
                    assert!(
                        b == 0x21 || (0x23..=0x5B).contains(&b) || (0x5D..=0x7E).contains(&b),
                        "invalid scope-token byte {b:#x} in {}",
                        ve.action_str
                    );
                }
            }
        }
    }

    #[test]
    fn dynamic_resources_parse_and_round_trip() {
        // A2 resource side + constructor seam.
        let plugin: Resource = "plugin.package-manager.apt".parse().expect("parses");
        assert_eq!(plugin.as_str(), "plugin.package-manager.apt");
        assert_eq!(plugin.plugin_type(), Some("package-manager.apt"));
        assert_eq!(
            plugin,
            Resource::plugin("package-manager.apt").expect("builds")
        );
        assert!(!plugin.is_system());
        assert_eq!(plugin.allowed_verbs(), Verb::ALL);

        let surface: Resource = "surface.ssh-agent.hosts".parse().expect("parses");
        assert_eq!(surface.surface_id(), Some("ssh-agent.hosts"));
        assert_eq!(
            surface,
            Resource::surface("ssh-agent.hosts").expect("builds")
        );
    }

    #[test]
    fn invalid_resources_rejected() {
        // (input, expected error) — A4, A7, A8 resource side.
        let cases: &[(&str, ParseResourceError)] = &[
            ("frobnicate", ParseResourceError::UnknownResource),
            ("plugin", ParseResourceError::UnknownResource),
            ("surface", ParseResourceError::UnknownResource),
            ("system.frobnicate", ParseResourceError::UnknownResource),
            ("plugin.", ParseResourceError::EmptyDynamicRemainder),
            ("surface.", ParseResourceError::EmptyDynamicRemainder),
            ("plugin.Apt", ParseResourceError::InvalidSegment),
            ("plugin.a_b", ParseResourceError::InvalidSegment),
            ("plugin.1a", ParseResourceError::InvalidSegment),
            ("plugin.-x", ParseResourceError::InvalidSegment),
            ("plugin.x-", ParseResourceError::InvalidSegment),
            ("plugin.a..b", ParseResourceError::InvalidSegment),
            ("surface.a..b", ParseResourceError::InvalidSegment),
        ];
        for (input, expected) in cases {
            assert_eq!(
                &input.parse::<Resource>().expect_err("must reject"),
                expected,
                "for {input:?}"
            );
        }
    }

    #[test]
    fn dynamic_constructors_reject_bad_remainders() {
        assert!(Resource::plugin("").is_err(), "empty remainder");
        assert!(Resource::surface("").is_err(), "empty remainder");
        assert!(Resource::plugin("Bad").is_err(), "uppercase segment");
        assert!(Resource::surface("a..b").is_err(), "empty middle segment");
    }
}
