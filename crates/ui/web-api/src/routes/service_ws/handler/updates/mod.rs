//! Update delivery, ownership, reconnect recovery, and update-lifecycle message
//! handlers, split across topical submodules. This facade wires the submodules
//! and re-exports the surface consumed by sibling handler modules.
//!
//! - [`ownership`] — host-link visibility checks
//! - [`replay`] — reconnect recovery + pending replay
//! - [`started`] / [`output`] / [`result`] — per-message handlers
//! - [`batch`] — batch result handling + progress
//! - [`dispatch`] — successor dispatch (batch/queue)
//! - [`audit`] / [`lookups`] / [`finalize`] / [`stdin`] — cross-cutting helpers

use std::time::Duration;

use super::shared_types::{
    HandlerError, HandlerResult, MAX_UPDATE_OUTPUT_BYTES, ProcessorResponse, load_linked_host_ids,
};

mod audit;
mod batch;
mod dispatch;
mod finalize;
mod lookups;
mod output;
mod ownership;
mod replay;
mod result;
mod started;
mod stdin;

#[cfg(test)]
pub(super) use replay::load_pending_update_records;
pub(super) use replay::{
    prepare_pending_replay_messages, recover_owned_updates_on_connect_with_dispatch_mode,
};

use dispatch::dispatch_next_queued_update;
pub(crate) use dispatch::{dispatch_next_batch_update, dispatch_next_batch_update_for_tenant};

pub(super) use batch::{
    emit_batch_progress_event, emit_batch_progress_from_db, handle_batch_completion,
    handle_batch_update_result,
};
pub(super) use output::handle_update_output;
pub(super) use result::handle_update_result;
pub(super) use started::handle_update_started;

pub(super) use stdin::handle_stdin_attention;

pub(super) use lookups::{resolve_host_name, resolve_software_item_name};
use ownership::validate_host_link_visibility;
use uptrakit_wire::ControllerMessage;

const RECOVERY_FINALIZATION_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
pub(super) enum ReconnectSuccessorDispatchMode {
    Immediate,
    ReplayPrepared,
}

struct ReplayPreparationNotifier;

#[async_trait::async_trait]
impl crate::ServiceNotifier for ReplayPreparationNotifier {
    async fn send_to_service(&self, _service_id: &uuid::Uuid, _msg: ControllerMessage) -> bool {
        false
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests;
