//! Common message handlers, split across topical submodules by message family.
//! This facade wires the submodules and re-exports the surface consumed by
//! sibling handler modules.
//!
//! - [`certificate`] — certificate-renewal handler + renew audit
//! - [`hosts`] — host-report handler + host linking/notify
//! - [`version_check`] — version-check results handler + enrichment/finalize
//! - [`discovery`] — discovery-results handler + page processing/enrichment
//! - [`plugin_config`] — plugin-config report handler + config audit
//! - [`restart_progression`] — post-restart host progression
//! - [`shared`] — ping handler + service-inventory audit helper

use super::super::super::agent_operations::{
    find_or_create_host_and_link, revoke_certificate, revoke_system_certificate,
};
use super::super::protocol::{
    CertIdentity, record_service_activity, record_system_service_activity, send_pong,
};
use super::audit_service::{
    emit_service_certificate_renew_audit_event, ingest_service_audit_event,
};
use super::discovery::trigger_discovery_for_agent_host;
use super::message_processor::LoopAction;
use super::renewal::{sign_renewal_csr, sign_renewal_csr_system};
use super::shared_types::{ProcessorResponse, load_linked_host_ids};
use super::updates::{
    emit_batch_progress_event, emit_batch_progress_from_db, handle_batch_completion,
    resolve_host_name, resolve_software_item_name,
};

mod certificate;
mod discovery;
mod hosts;
mod plugin_config;
mod restart_progression;
mod shared;
mod version_check;

pub(super) use certificate::handle_renew_certificate;
pub(super) use discovery::handle_discovery_results;
pub(super) use hosts::handle_report_hosts;
pub(super) use plugin_config::handle_report_plugin_config;
use restart_progression::trigger_host_progression_after_awaiting_restart;
use shared::emit_service_inventory_audit;
pub(super) use shared::handle_ping;
pub(super) use version_check::handle_version_check_results;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "db-sqlite"))]
mod tests;
