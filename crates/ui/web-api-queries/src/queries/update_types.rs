/// Typed actor that initiated an update or batch operation.
///
/// Stored as a snake_case string in the database (`actor_type` column).
/// Using a typed enum prevents silent typos at write-call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorType {
    /// Triggered by a human operator via the REST API.
    User,
    /// Triggered by a scheduled task.
    Scheduler,
}

impl ActorType {
    /// Returns the canonical snake_case string stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Scheduler => "scheduler",
        }
    }
}

impl std::fmt::Display for ActorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
