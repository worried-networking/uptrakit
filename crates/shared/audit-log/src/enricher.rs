use crate::entry::AuditActorType;

/// Resolves a human-readable display name from an actor type and ID.
///
/// Implementations perform DB lookups and must never panic. Failures are
/// silently swallowed — enrichment is best-effort and must not affect audit
/// delivery.
#[async_trait::async_trait]
pub trait ActorEnricher: Send + Sync {
    async fn display_name(
        &self,
        actor_type: AuditActorType,
        actor_id: uuid::Uuid,
    ) -> Option<String>;
}
