use sea_orm::{DatabaseConnection, EntityTrait};
use uptrakit_audit_log::{ActorEnricher, AuditActorType};
use uptrakit_shared_db::entity::{api_token, user};
use uuid::Uuid;

pub(crate) struct DbActorEnricher {
    db: DatabaseConnection,
}

impl DbActorEnricher {
    pub(crate) fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl ActorEnricher for DbActorEnricher {
    async fn display_name(&self, actor_type: AuditActorType, actor_id: Uuid) -> Option<String> {
        match actor_type {
            AuditActorType::User => {
                let row = user::Entity::find_by_id(actor_id)
                    .one(&self.db)
                    .await
                    .ok()??;
                Some(row.email.expose_email().to_string())
            }
            AuditActorType::ApiToken => {
                let (token, user) = api_token::Entity::find_by_id(actor_id)
                    .find_also_related(user::Entity)
                    .one(&self.db)
                    .await
                    .ok()??;
                let user = user?;
                Some(format!("{} ({})", token.name, user.email.expose_email()))
            }
            _ => None,
        }
    }
}
