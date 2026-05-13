use crate::event_broadcaster::EventBroadcaster;
use crate::notification_service::NotificationService;

pub(crate) mod host_tags;
pub(crate) mod hosts;
pub(crate) mod services;
#[cfg(feature = "reset-data")]
pub(crate) mod settings;
pub(crate) mod software_items;
pub(crate) mod system_services;
pub(crate) mod update_batches;

/// Borrow-bundle carrying the two common side-effect handles shared by all
/// mutation actions. Domain-specific handles are passed as explicit parameters
/// to individual action functions.
pub(crate) struct MutationContext<'a> {
    pub notification_service: &'a NotificationService,
    pub event_broadcaster: &'a EventBroadcaster,
}
