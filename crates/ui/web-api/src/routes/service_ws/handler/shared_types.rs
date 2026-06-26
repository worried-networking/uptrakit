//! Types and helpers shared across handler sub-modules.
//!
//! This module exists to break the import cycle between `messages` and
//! `updates`: both need [`ProcessorResponse`] / [`ProcessorAction`] and
//! [`load_linked_host_ids`], so those items live here instead.

use std::collections::HashSet;

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, EntityTrait, JoinType, QueryFilter, QuerySelect, RelationTrait as _};
use thiserror::Error;
use uptrakit_shared_db::entity::{host, service, service_host};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_wire::{CloseReason, ControllerMessage};

use crate::AppState;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Internal error type for helper functions (deliver_pending_updates, etc.).
#[derive(Debug, Error)]
pub(super) enum HandlerError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("websocket send failed")]
    WebSocketSend,
}

pub(super) type HandlerResult<T> = std::result::Result<T, Report<HandlerError>>;

impl_report_conversion!(sea_orm::DbErr => HandlerError::Database);

/// Maximum size of the `update_history.output` column (50 MB).
///
/// Docker image pulls generate very verbose progress output (tens of megabytes
/// for large images). This cap covers virtually all real-world update outputs
/// while preventing unbounded DB growth.
///
/// When the cap is first exceeded, a visible system output line is emitted
/// into the stream and the `output_truncated` flag is set on the history
/// record so the UI can display a persistent warning banner.
pub(super) const MAX_UPDATE_OUTPUT_BYTES: usize = 52_428_800;

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
// ServiceAuditCtx
// ---------------------------------------------------------------------------

/// Shared context for service-lifecycle audit helpers.
///
/// Used by both `audit_surface` and `audit_service` sub-modules.
pub(super) struct ServiceAuditCtx<'a> {
    pub(super) state: &'a AppState,
    pub(super) service_id: uuid::Uuid,
    pub(super) service_app_name: Option<&'a str>,
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

// ---------------------------------------------------------------------------
// WS handler constants and cross-cutting micro-helpers
// ---------------------------------------------------------------------------

/// Maximum time to wait for a WebSocket write (`sink.send()`) to complete.
///
/// If a service stops reading from the WebSocket, the OS TCP send buffer fills
/// and `sink.send()` blocks indefinitely. This timeout bounds the hang so that
/// the handler loop can break and clean up the connection. Kept deliberately
/// shorter than the agent-side `SEND_TIMEOUT` (30 s) so the controller detects
/// the stuck connection first.
pub(super) const WS_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

const MQTT_SERVICE_APP_NAME: &str = "uptrakit-mqtt";

/// Returns the tenant ID that a system service is bound to, if any.
///
/// Only the MQTT service carries a single-tenant binding; all other system
/// services (scheduler, etc.) operate globally and return `None`.
pub(super) fn system_service_tenant_binding(
    service_app_name: Option<&str>,
    default_tenant_id: uuid::Uuid,
) -> Option<uuid::Uuid> {
    (service_app_name == Some(MQTT_SERVICE_APP_NAME)).then_some(default_tenant_id)
}

/// Returns `true` when `payload_tenant_id` is within the scope that
/// `service_tenant_id` is allowed to access.
///
/// - Tenant-bound services (`Some`) may only write to their own tenant.
/// - System services (`None`) may write to any scope.
pub(super) fn is_valid_service_config_scope(
    service_tenant_id: Option<uuid::Uuid>,
    payload_tenant_id: Option<uuid::Uuid>,
) -> bool {
    match service_tenant_id {
        Some(bound_tenant_id) => payload_tenant_id == Some(bound_tenant_id),
        None => true,
    }
}
