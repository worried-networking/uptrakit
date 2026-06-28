//! Service route handlers.
//!
//! Facade: module wiring + handler/type re-exports only. Implementation lives in:
//! - `audit`     — `AuditContext` + audit-emit helpers (fire-and-forget `emit_event`)
//! - `crud`      — list / get / update service handlers
//! - `lifecycle` — approve / reject / deactivate / set-update-freeze handlers
//! - `merge`     — service merge handler
//! - `batch`     — batch service-action handler

mod audit;
mod batch;
mod crud;
mod lifecycle;
mod merge;

pub use batch::{__path_batch_services, batch_services};
pub use crud::{
    __path_get_service, __path_list_services, __path_update_service, get_service, list_services,
    update_service,
};
pub use lifecycle::{
    __path_approve_service, __path_deactivate_service, __path_reject_service,
    __path_set_update_freeze, approve_service, deactivate_service, reject_service,
    set_update_freeze,
};
pub use merge::{__path_merge_service, merge_service};

pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::services::{
    ListServicesQuery, MergeAgentRequest, MessageResponse, ServiceResponse, ServiceStatus,
    SetUpdateFreezeRequest, UpdateServiceRequest,
};

#[cfg(test)]
mod tests;
