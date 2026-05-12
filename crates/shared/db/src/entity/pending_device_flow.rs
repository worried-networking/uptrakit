use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use uptrakit_shared_types::DeviceAuthStatus;

/// A pending device-authorization flow (RFC 8628 §3.1).
///
/// Status transitions:
/// - `Pending` (initial) → `Authorized` (via `approve`) → row consumed by `poll`.
/// - `Pending` → `Denied` (via `deny`).
/// - `Pending` → `Expired` (background sweeper after `expires_at`).
///
/// Invariant: at most one of `user_id` (approver) and `denied_by` (denier) is `Some`.
/// A row in `Authorized` status has `user_id = Some(...)` and `denied_by = None`;
/// a row in `Denied` status has `user_id = None` and `denied_by = Some(...)`.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "pending_device_flows")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique, column_type = "Text")]
    pub device_code_hash: String,
    #[sea_orm(unique, column_type = "Text")]
    pub user_code: String,
    pub status: DeviceAuthStatus,
    /// User who approved this flow. `Some` only when `status = Authorized`.
    pub user_id: Option<Uuid>,
    /// User who denied this flow. `Some` only when `status = Denied`.
    pub denied_by: Option<Uuid>,
    pub client_name: Option<String>,
    /// Requested OAuth `scope` parameter (RFC 8628 §3.1). Echoed on token response,
    /// not yet enforced (Seam 2 in the spec's Future Migrations section).
    pub scope: Option<String>,
    /// Current polling interval in seconds. Initialised to 5; bumped by 5 each
    /// time a `slow_down` is returned to the caller.
    pub interval: i32,
    /// Timestamp of the most recent poll request. `None` until the first poll.
    pub last_polled_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
