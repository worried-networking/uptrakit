use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifies the scope of a settings version row.
///
/// Used to distinguish between global controller settings and per-tenant
/// settings during config reload operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// Global controller-wide settings.
    Global,
    /// Per-tenant settings identified by tenant UUID.
    Tenant(Uuid),
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Global => f.write_str("global"),
            Self::Tenant(id) => write!(f, "tenant:{id}"),
        }
    }
}
