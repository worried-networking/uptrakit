//! Types and helpers shared across handler sub-modules.
//!
//! This module exists to break the import cycle between `messages` and
//! `updates`: both need [`ProcessorResponse`] / [`ProcessorAction`] and
//! [`load_linked_host_ids`], so those items live here instead.

use std::collections::HashSet;

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, EntityTrait, JoinType, QueryFilter, QuerySelect, RelationTrait as _};
use uptrakit_internal_wire::{CloseReason, ControllerMessage};
use uptrakit_shared_db::entity::{host, service, service_host};

use super::{HandlerError, HandlerResult};

// ---------------------------------------------------------------------------
// ProcessorAction
// ---------------------------------------------------------------------------

/// Action for the main loop after the processor handles a message.
pub(super) enum ProcessorAction {
    /// Continue processing messages.
    Continue,
    /// Break out of the main loop.
    Break,
    /// Send a WebSocket close frame with a reason, then break.
    CloseWithReason(CloseReason),
}

/// Response from the message processor to the main loop.
pub(super) struct ProcessorResponse {
    /// Optional message to send to the service before executing the action.
    pub replies: Vec<ControllerMessage>,
    /// Action for the main loop after sending replies.
    pub action: ProcessorAction,
}

impl ProcessorResponse {
    /// Continue with no reply.
    pub(crate) fn cont() -> Self {
        Self {
            replies: Vec::new(),
            action: ProcessorAction::Continue,
        }
    }

    /// Continue with a single reply.
    pub(crate) fn reply(msg: ControllerMessage) -> Self {
        Self {
            replies: vec![msg],
            action: ProcessorAction::Continue,
        }
    }

    /// Break with a single reply.
    pub(crate) fn reply_and_break(msg: ControllerMessage) -> Self {
        Self {
            replies: vec![msg],
            action: ProcessorAction::Break,
        }
    }

    /// Send a reply and close with a reason.
    pub(crate) fn reply_and_close(msg: ControllerMessage, reason: CloseReason) -> Self {
        Self {
            replies: vec![msg],
            action: ProcessorAction::CloseWithReason(reason),
        }
    }
}

// ---------------------------------------------------------------------------
// load_linked_host_ids
// ---------------------------------------------------------------------------

/// Load the set of host IDs linked to the given service.
#[tracing::instrument(skip_all, fields(%service_id))]
pub(super) async fn load_linked_host_ids(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
) -> HandlerResult<HashSet<uuid::Uuid>> {
    let links = service_host::Entity::find()
        .join(JoinType::InnerJoin, service_host::Relation::Host.def())
        .join(JoinType::InnerJoin, service_host::Relation::Service.def())
        .filter(service_host::Column::ServiceId.eq(service_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .filter(service::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to::<HandlerError>()?;

    Ok(links.into_iter().map(|l| l.host_id).collect())
}
