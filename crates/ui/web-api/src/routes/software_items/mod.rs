//! HTTP route handlers for `/api/v1/software-items`.
//!
//! Controller-side fetch orchestration lives in [`controller_fetch`].
//! Version-check context loading and agent dispatch live in [`version_check_dispatch`].

mod audit;
mod batch;
mod controller_fetch;
mod crud;
mod host_assignments;
mod merge;
mod updates;
mod version_check;
mod version_check_dispatch;

pub use crud::{
    approve_software_item, create_software_item, delete_software_item, get_software_item,
    list_software_items, update_software_item,
};
// Re-export utoipa `__path_*` types so `routes!(crate::routes::software_items::<handler>)`
// in router.rs resolves them at the facade's public path.
pub use batch::__path_batch_software_items;
pub use batch::batch_software_items;
pub use crud::{
    __path_approve_software_item, __path_create_software_item, __path_delete_software_item,
    __path_get_software_item, __path_list_software_items, __path_update_software_item,
};
pub use host_assignments::{
    __path_assign_hosts, __path_delete_plugin_assignment, __path_unassign_host,
    __path_update_host_assignment,
};
pub use host_assignments::{
    DeleteHostAssignmentParams, assign_hosts, delete_plugin_assignment, unassign_host,
    update_host_assignment,
};
pub use merge::{__path_execute_software_item_merge, __path_preview_software_item_merge};
pub use merge::{execute_software_item_merge, preview_software_item_merge};
pub use updates::__path_trigger_update;
pub use updates::trigger_update;
pub use version_check::{__path_check_versions, __path_check_versions_host};
pub use version_check::{check_versions, check_versions_host};

pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
pub use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, ListSoftwareItemsParams,
    MergeSoftwareItemsExecuteRequest, MergeSoftwareItemsExecuteResponse,
    MergeSoftwareItemsPreviewRequest, MergeSoftwareItemsPreviewResponse,
    SoftwareItemDetailResponse, SoftwareItemHostSummary, SoftwareItemResponse,
    TriggerUpdateRequest, TriggerUpdateResponse, TriggerUpdateStatus, TriggerVersionCheckResponse,
    UpdateHostAssignmentRequest, UpdateSoftwareItemRequest,
};

#[cfg(all(test, feature = "db-sqlite"))]
mod audit_tests;
#[cfg(all(test, feature = "db-sqlite"))]
mod tests;
