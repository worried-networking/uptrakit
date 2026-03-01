use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// Cached signed CRL for a CA, keyed by CA fingerprint.
///
/// Persisted to avoid rebuilding CRLs on every controller startup (startup
/// efficiency) and to allow multi-instance HA deployments to serve a
/// consistent CRL before the first scheduled renewal fires.
///
/// No `tenant_id` — CRL is global PKI state.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "crl_cache")]
pub struct Model {
    /// SHA-256 fingerprint of the issuing CA certificate.
    #[sea_orm(primary_key, auto_increment = false)]
    pub ca_fingerprint: String,
    /// PEM-encoded signed CRL.
    pub crl_pem: String,
    /// Monotonically increasing CRL serial number (RFC 5280 cRLNumber extension).
    pub crl_number: i64,
    /// `thisUpdate` field from the CRL.
    pub this_update: OffsetDateTime,
    /// `nextUpdate` field from the CRL (validity expiry).
    pub next_update: OffsetDateTime,
    /// Wall-clock timestamp of the last upsert.
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
