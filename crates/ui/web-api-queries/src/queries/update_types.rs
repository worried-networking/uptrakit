use std::str::FromStr;
use thiserror::Error;

/// Typed actor that initiated an update or batch operation.
///
/// Stored as a snake_case string in the database (`actor_type` column). Internal write-path
/// discriminator — not a wire type. Per `docs/development/coding-standards.md`
/// §"Typed enums for internal write-path discriminators", this enum does not carry
/// `#[non_exhaustive]` and does not need an `Other(String)` variant: the set of strings written
/// to `update_history.actor_type` and `update_batches.actor_type` is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorType {
    /// Triggered by a human operator via the REST API.
    User,
    /// Triggered by an API token via the REST API.
    ApiToken,
    /// Triggered by a scheduled task.
    Scheduler,
    /// Triggered by a Service (Agent, Agent-SSH) over the service WS transport,
    /// except MQTT which carries its own variant for backwards compatibility with
    /// the on-disk `"uptrakit-mqtt"` string.
    Service,
    /// Triggered by an internal system path that does not correspond to a single
    /// Service identity (e.g. unattended bootstrap).
    SystemService,
    /// Triggered by the MQTT Service. Canonical on-disk value is `"uptrakit-mqtt"`
    /// (legacy spelling preserved — see `coding-standards.md`).
    Mqtt,
    /// Triggered by an instance-wide system path (e.g. scheduler cleanup that writes
    /// to `update_history`). Distinct from `AuditActorType::System` which targets
    /// the audit-log family.
    System,
}

impl ActorType {
    /// Returns the canonical snake_case string stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::ApiToken => "api_token",
            Self::Scheduler => "scheduler",
            Self::Service => "service",
            Self::SystemService => "system_service",
            Self::Mqtt => "uptrakit-mqtt",
            Self::System => "system",
        }
    }

    /// Map a Service binary's `service_app_name` to the typed actor.
    ///
    /// `"uptrakit-mqtt"` maps to [`ActorType::Mqtt`] (backwards-compatible with the legacy on-disk
    /// spelling). Every other value — including the registration fallback `"unknown"` and the
    /// agent-ssh binary `"uptrakit-agent-ssh"` — maps to [`ActorType::Service`]. The granular Service
    /// identity is carried separately in the row's `actor_id` (the Service UUID), so collapsing here
    /// loses no information that wasn't already available via a JOIN to `service.service_app_name`.
    pub fn from_service_app_name(service_app_name: &str) -> Self {
        match service_app_name {
            "uptrakit-mqtt" => Self::Mqtt,
            _ => Self::Service,
        }
    }
}

impl std::fmt::Display for ActorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned by `ActorType::from_str` for unknown strings.
///
/// `ActorType` is an internal closed enum; an unrecognised string is treated as a caller
/// bug, not a forward-compat case.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseActorTypeError {
    #[error("invalid actor_type value")]
    Invalid,
}

impl FromStr for ActorType {
    type Err = ParseActorTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Self::User),
            "api_token" => Ok(Self::ApiToken),
            "scheduler" => Ok(Self::Scheduler),
            "service" => Ok(Self::Service),
            "system_service" => Ok(Self::SystemService),
            "uptrakit-mqtt" => Ok(Self::Mqtt),
            "system" => Ok(Self::System),
            _ => Err(ParseActorTypeError::Invalid),
        }
    }
}

/// Typed batch category for update batch operations.
///
/// Stored as a snake_case string in the database (`batch_type` column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchType {
    /// A host-wide batch that updates all outdated software items on one host.
    HostUpdate,
    /// An item-wide rollout that updates a single software item across all hosts.
    ItemRollout,
    /// A host package batch that updates all outdated managed packages on one host.
    HostPackage,
}

impl BatchType {
    /// Returns the canonical snake_case string stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HostUpdate => "host_update",
            Self::ItemRollout => "item_rollout",
            Self::HostPackage => "host_package",
        }
    }
}

impl std::fmt::Display for BatchType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod actor_type_tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn actor_type_as_str_matches_on_disk_strings() {
        assert_eq!(ActorType::User.as_str(), "user");
        assert_eq!(ActorType::ApiToken.as_str(), "api_token");
        assert_eq!(ActorType::Scheduler.as_str(), "scheduler");
        assert_eq!(ActorType::Service.as_str(), "service");
        assert_eq!(ActorType::SystemService.as_str(), "system_service");
        assert_eq!(ActorType::Mqtt.as_str(), "uptrakit-mqtt");
        assert_eq!(ActorType::System.as_str(), "system");
    }

    #[test]
    fn actor_type_from_str_round_trips_every_variant() {
        for variant in [
            ActorType::User,
            ActorType::ApiToken,
            ActorType::Scheduler,
            ActorType::Service,
            ActorType::SystemService,
            ActorType::Mqtt,
            ActorType::System,
        ] {
            let s = variant.as_str();
            let parsed = ActorType::from_str(s).expect("known variant must parse");
            assert_eq!(parsed, variant, "round-trip mismatch for {s:?}");
        }
    }

    #[test]
    fn actor_type_from_str_rejects_unknown() {
        assert!(matches!(
            ActorType::from_str("nope"),
            Err(ParseActorTypeError::Invalid)
        ));
        assert!(matches!(
            ActorType::from_str(""),
            Err(ParseActorTypeError::Invalid)
        ));
    }

    #[test]
    fn from_service_app_name_maps_known_binaries() {
        assert_eq!(
            ActorType::from_service_app_name("uptrakit-mqtt"),
            ActorType::Mqtt
        );
    }

    #[test]
    fn from_service_app_name_falls_back_to_service_for_unknown() {
        assert_eq!(
            ActorType::from_service_app_name("uptrakit-agent-ssh"),
            ActorType::Service
        );
        assert_eq!(
            ActorType::from_service_app_name("uptrakit-agent"),
            ActorType::Service
        );
        assert_eq!(
            ActorType::from_service_app_name("unknown"),
            ActorType::Service
        );
        assert_eq!(ActorType::from_service_app_name(""), ActorType::Service);
    }
}
