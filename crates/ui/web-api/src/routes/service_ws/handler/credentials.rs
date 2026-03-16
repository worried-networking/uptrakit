//! Service credential delivery for authenticated connections.
//!
//! Contains [`deliver_service_credentials`], which sends DB URL, NATS URL,
//! and master-key hex to services that carry the corresponding capabilities.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::SinkExt;

use uptrakit_internal_wire::{
    Capability, ControllerMessage, OutgoingSeq, ServiceCredentialsPayload,
};

use crate::AppState;
use crate::routes::service_ws::protocol::serialize_controller_msg;

// ---------------------------------------------------------------------------
// deliver_service_credentials
// ---------------------------------------------------------------------------

/// Deliver service credentials (DB URL, NATS URL, master key) to services
/// that have the corresponding capabilities.
///
/// Returns `Some(())` on success (including when no credentials are needed)
/// or `None` if the WebSocket write failed and the connection should close.
pub(super) async fn deliver_service_credentials(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    capabilities: &BTreeSet<Capability>,
    service_id: uuid::Uuid,
    out_seq: &mut OutgoingSeq,
) -> Option<()> {
    let has_db_access = capabilities.contains(&Capability::DatabaseAccess);
    let has_nats_access = capabilities.contains(&Capability::NatsAccess);
    let has_master_key_access = capabilities.contains(&Capability::MasterKeyAccess);

    if !has_db_access && !has_nats_access && !has_master_key_access {
        return Some(());
    }

    let sources = &state.credential_sources;
    let payload = ServiceCredentialsPayload {
        db_url: if has_db_access {
            sources
                .db_url
                .as_ref()
                .map(|u| uptrakit_internal_wire::SecretString::new(u.clone()))
        } else {
            None
        },
        nats_url: if has_nats_access {
            sources.nats_url.clone()
        } else {
            None
        },
        master_key_hex: if has_master_key_access {
            sources.master_key_hex.clone()
        } else {
            None
        },
    };
    let cred_msg = ControllerMessage::ServiceCredentials(payload);
    if let Some(json) = serialize_controller_msg(out_seq, cred_msg)
        && sink.send(Message::Text(json.into())).await.is_err()
    {
        return None;
    }
    tracing::info!(
        %service_id,
        db = has_db_access,
        nats = has_nats_access,
        master_key = has_master_key_access,
        "delivered service credentials"
    );

    Some(())
}
