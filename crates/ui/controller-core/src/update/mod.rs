pub mod controller;

use async_trait::async_trait;
use time::OffsetDateTime;
use uuid::Uuid;

use uptrakit_shared_types::OutputStreamType;
use uptrakit_web_api_queries::queries::update_types::ActorType;

/// Groups actor identification for a dispatch request.
///
/// `#[non_exhaustive]`: future auth methods may add fields (e.g. `scope`).
/// External crates must use `ActorInfo::new(…)`.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ActorInfo {
    pub actor_type: ActorType,
    pub actor_id: String,
}

impl ActorInfo {
    pub fn new(actor_type: ActorType, actor_id: impl Into<String>) -> Self {
        Self {
            actor_type,
            actor_id: actor_id.into(),
        }
    }
}

/// Outcome of a dispatch attempt.
///
/// `#[non_exhaustive]`: future outcomes (e.g. `RateLimited`) may be added.
/// External match sites must include a wildcard arm with `tracing::warn!`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// Agent was connected and the dispatch message was delivered.
    Sent,
    /// Record created; agent offline — reconnect recovery will pick it up.
    Queued,
    /// Pre-dispatch validation or protection step failed.
    Failed,
}

/// Result returned by a successful `UpdateDispatcher::dispatch` call.
#[non_exhaustive]
#[derive(Debug)]
pub struct UpdateDispatchResult {
    pub update_history_id: Uuid,
    pub outcome: DispatchOutcome,
}

/// Domain errors from the update dispatch pipeline.
///
/// NOT a wire type — converted to adapter-specific errors at the HTTP/MCP boundary.
/// `#[non_exhaustive]`: new validation errors may be added. External match sites
/// must include a wildcard arm with `tracing::warn!`.
#[non_exhaustive]
#[derive(Debug)]
pub enum UpdateDispatchError {
    HostNotFound,
    SoftwareItemNotFound,
    UpdateAlreadyActive,
    NotConfigured,
    AgentUnavailable,
    Internal,
}

impl std::fmt::Display for UpdateDispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HostNotFound => write!(f, "host not found"),
            Self::SoftwareItemNotFound => write!(f, "software item not found"),
            Self::UpdateAlreadyActive => write!(f, "update already active for this host"),
            Self::NotConfigured => write!(f, "host not configured for updates"),
            Self::AgentUnavailable => write!(f, "no approved agent linked to host"),
            Self::Internal => write!(f, "internal error"),
        }
    }
}

impl std::error::Error for UpdateDispatchError {}

/// Parameters for triggering a software update via `UpdateDispatcher::dispatch`.
///
/// `#[non_exhaustive]`: new fields (e.g. `force`, `dry_run`) may be added.
/// External crates must construct via `UpdateDispatchParams::new(…)`.
#[non_exhaustive]
#[derive(Debug)]
pub struct UpdateDispatchParams {
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub to_version: String,
    pub actor: ActorInfo,
    /// Serialised release metadata; `None` if caller has no release context.
    pub release_info: Option<serde_json::Value>,
    pub interactive: bool,
}

impl UpdateDispatchParams {
    pub fn new(
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        to_version: String,
        actor: ActorInfo,
        release_info: Option<serde_json::Value>,
        interactive: bool,
    ) -> Self {
        Self {
            tenant_id,
            host_id,
            software_item_id,
            to_version,
            actor,
            release_info,
            interactive,
        }
    }
}

/// Abstraction over the SSE update-output broadcaster.
///
/// `ControllerUpdateDispatcher` calls this to stream protection/dispatch output
/// without knowing about Axum or SSE. `web-api` provides the concrete impl via
/// `UpdateOutputBroadcaster`.
#[async_trait]
pub trait UpdateOutputStream: Send + Sync {
    async fn create_channel(&self, update_id: Uuid);
    async fn send_line(
        &self,
        update_id: Uuid,
        line_id: Uuid,
        text: String,
        stream: OutputStreamType,
        ts: OffsetDateTime,
    );
    async fn send_completed(
        &self,
        update_id: Uuid,
        outcome: DispatchOutcome,
        error: Option<String>,
    );
}

/// Dispatches software update requests through the protection/agent pipeline.
///
/// Implemented by `ControllerUpdateDispatcher` (production) and
/// `NoopUpdateDispatcher` (tests). Both `AppState` and `McpState` hold
/// `Arc<dyn UpdateDispatcher>`.
#[async_trait]
pub trait UpdateDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        params: UpdateDispatchParams,
    ) -> Result<UpdateDispatchResult, rootcause::Report<UpdateDispatchError>>;
}

/// No-op dispatcher for tests that do not exercise update dispatch.
pub struct NoopUpdateDispatcher;

#[async_trait]
impl UpdateDispatcher for NoopUpdateDispatcher {
    async fn dispatch(
        &self,
        _params: UpdateDispatchParams,
    ) -> Result<UpdateDispatchResult, rootcause::Report<UpdateDispatchError>> {
        use rootcause::report;
        Err(report!(UpdateDispatchError::Internal))
    }
}
