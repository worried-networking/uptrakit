pub mod capabilities;
pub mod close_reason;
pub mod envelope;
mod extension;
pub mod limits;
pub mod messages;
pub mod paginate;
pub mod payloads;
pub mod report_tracker;
pub mod serde_helpers;
pub mod service_profile;
pub mod shared_types;
pub mod surfaces;
pub mod trace_context;
pub mod transport;

mod wire_validate_impls;

// Re-export all public items from submodules.
pub use capabilities::*;
pub use close_reason::CloseReason;
pub use envelope::*;
pub use messages::*;
pub use paginate::Paginatable;
pub use payloads::*;
pub use report_tracker::ReportTracker;
pub use service_profile::{ServiceProfile, parse_capabilities, serialize_capabilities};
pub use shared_types::*;
pub use trace_context::{TraceContext, current_trace_context};
pub use transport::{ServiceTransport, TransportClosePolicy, TransportError};

// Re-export shared types used directly in wire protocol messages.
pub use uptrakit_shared_types::{
    AttestationStatus, DiscoveredSoftware, DiscoveryTarget, HookShell, OutputStreamType,
    PluginRole, PluginTypeId, ReleaseAsset, ReleaseInfo, UpdateCategory, plugin_ids,
};
// Re-export `SecretString` for callers that need it for secret fields.
pub use uptrakit_shared_types::SecretString;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
