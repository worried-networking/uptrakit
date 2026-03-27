use crate::event_broadcaster::EventBroadcaster;
use crate::notification_service::NotificationService;
use crate::notifications::dispatcher::NotificationDispatcher;

pub(crate) mod host_tags;
pub(crate) mod hosts;
pub(crate) mod services;
#[cfg(feature = "reset-data")]
pub(crate) mod settings;
pub(crate) mod software_items;
pub(crate) mod system_services;
pub(crate) mod update_batches;

/// Borrow-bundle carrying the three common side-effect handles shared by all
/// mutation actions. Domain-specific handles are passed as explicit parameters
/// to individual action functions.
pub(crate) struct MutationContext<'a> {
    pub notification_service: &'a NotificationService,
    pub notification_dispatcher: &'a NotificationDispatcher,
    pub event_broadcaster: &'a EventBroadcaster,
}
