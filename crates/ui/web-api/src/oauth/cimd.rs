//! CIMD (Client Identifier Metadata Document) fetcher and cache manager.
//!
//! Per spec §11.3: fetch → raw-bytes persist → two-pass parse → upsert.
//! Parse failures preserve the existing cached row and never invalidate it,
//! so an upstream CIMD draft revision cannot trigger forced re-consent.
//!
//! Material-change detection compares a normalised hash (cosmetic fields
//! excluded) so updates to display-only fields like `tos_uri` or
//! `software_version` do not invalidate active consents.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel, QueryFilter,
    Set, SqliteTransactionMode, TransactionOptions, TransactionTrait,
};
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_audit_log::{AuditActionType, AuditEmitter, AuditEntry, AuditOutcome, Event};
use uptrakit_shared_db::entity::{oauth_client, oauth_consent};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_shared_types::ssrf::SsrfSafeResolver;

use crate::oauth::cimd_parser::{self, CimdDocument, CimdParseError};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from [`CimdFetcher::fetch_and_upsert`].
///
/// Parse failures are *not* errors at this layer: the existing cached row is
/// preserved per spec §11.3.  Parse failures only return [`CimdFetchError`]
/// when no existing row exists (i.e. first fetch ever).
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum CimdFetchError {
    /// Underlying HTTP fetch failed (connect error, timeout, TLS, etc.).
    #[error("HTTP fetch failed")]
    Http(reqwest::Error),
    /// Response body exceeded the 64 KB cap mandated by spec §11.3.
    #[error("response body exceeded 64 KB")]
    BodyTooLarge,
    /// First fetch ever returned non-JSON or otherwise unparseable bytes.
    ///
    /// On a *re-fetch* the parse failure is swallowed and the existing row
    /// is preserved; this variant is only returned when no prior row exists.
    #[error("first CIMD fetch produced unparseable document")]
    UnparseableFirstFetch,
    /// CIMD document's `client_id` field does not equal the fetch URL.
    #[error("client_id in CIMD document does not match fetch URL")]
    ClientIdMismatch,
    /// Database error during upsert or cascade-revalidation.
    #[error("database error")]
    Database(sea_orm::DbErr),
}

impl_report_conversion!(sea_orm::DbErr => CimdFetchError::Database);
impl_report_conversion!(reqwest::Error => CimdFetchError::Http);

/// 64 KB body cap per spec §11.3 — protects against unbounded CIMD documents.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// Cosmetic fields excluded from the material-change hash per spec §11.3.
///
/// Changes to these fields do not invalidate active consents.
const COSMETIC_FIELDS: &[&str] = &["tos_uri", "policy_uri", "software_version", "software_id"];

// ---------------------------------------------------------------------------
// Fetcher
// ---------------------------------------------------------------------------

/// Fetches CIMD documents and maintains the `oauth_clients` cached row.
///
/// `#[non_exhaustive]` is not required because this is a private newtype
/// constructed via [`CimdFetcher::new`] / [`CimdFetcher::new_permissive`].
pub struct CimdFetcher {
    client: reqwest::Client,
    clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    db: DatabaseConnection,
    audit_emitter: Arc<AuditEmitter>,
}

impl CimdFetcher {
    /// Build a fetcher with the SSRF-safe DNS resolver and bounded timeouts.
    ///
    /// Per the coding standards: `.connect_timeout(10s)` + `.timeout(60s)` +
    /// `SsrfSafeResolver::new()` for user-controlled URLs.
    ///
    /// # Errors
    /// Returns [`reqwest::Error`] if the TLS backend cannot be initialised.
    pub fn new(
        db: DatabaseConnection,
        clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
        audit_emitter: Arc<AuditEmitter>,
    ) -> std::result::Result<Self, reqwest::Error> {
        Self::build(db, clock, audit_emitter, SsrfSafeResolver::new())
    }

    /// Build a fetcher that allows private/loopback addresses (tests only).
    ///
    /// Equivalent to [`Self::new`] but uses [`SsrfSafeResolver::permissive`]
    /// so tests can target `127.0.0.1` mock servers.  Per the coding
    /// standards this constructor is also acceptable for self-hosted
    /// deployments that intentionally allow private URLs.
    ///
    /// # Errors
    /// Returns [`reqwest::Error`] if the TLS backend cannot be initialised.
    pub fn new_permissive(
        db: DatabaseConnection,
        clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
        audit_emitter: Arc<AuditEmitter>,
    ) -> std::result::Result<Self, reqwest::Error> {
        Self::build(db, clock, audit_emitter, SsrfSafeResolver::permissive())
    }

    fn build(
        db: DatabaseConnection,
        clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
        audit_emitter: Arc<AuditEmitter>,
        resolver: SsrfSafeResolver,
    ) -> std::result::Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .dns_resolver(Arc::new(resolver))
            .build()?;
        Ok(Self {
            client,
            clock,
            db,
            audit_emitter,
        })
    }

    /// Fetch a CIMD document and upsert the cached row.
    ///
    /// On success returns the final cached [`oauth_client::Model`].
    ///
    /// Parse failures of a *re-fetch* are soft failures: the existing cached
    /// row is preserved with only `metadata_parse_error` /
    /// `metadata_parse_error_at` updated, and an
    /// [`AuditActionType::OAUTH_CIMD_PARSE_FAILED`] audit entry is emitted.
    ///
    /// Material-change detection: when the new content's normalised hash
    /// (excluding cosmetic fields like `tos_uri`) differs from the previous
    /// one, all active consents for this client are marked
    /// `revalidation_required_at = now` and an
    /// [`AuditActionType::OAUTH_CLIENT_METADATA_CHANGED_MATERIALLY`] audit
    /// entry is emitted.
    ///
    /// # Errors
    /// - [`CimdFetchError::Http`] — network fetch failed
    /// - [`CimdFetchError::BodyTooLarge`] — response exceeded 64 KB
    /// - [`CimdFetchError::UnparseableFirstFetch`] — first-ever fetch produced
    ///   unparseable bytes
    /// - [`CimdFetchError::ClientIdMismatch`] — document's `client_id` differs
    ///   from the fetch URL
    /// - [`CimdFetchError::Database`] — DB upsert failed
    pub async fn fetch_and_upsert(
        &self,
        url: &str,
    ) -> std::result::Result<oauth_client::Model, Report<CimdFetchError>> {
        // -------------------------------------------------------------------
        // Step 1: HTTP fetch
        // -------------------------------------------------------------------
        let response = self.client.get(url).send().await.context_to()?;
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let bytes = response.bytes().await.context_to()?;
        if bytes.len() > MAX_BODY_BYTES {
            bail!(CimdFetchError::BodyTooLarge);
        }

        // -------------------------------------------------------------------
        // Step 2: hash + raw-bytes capture
        // -------------------------------------------------------------------
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let content_hash = format!("{:x}", hasher.finalize());

        let raw_str = String::from_utf8_lossy(&bytes).into_owned();
        let now = (self.clock)();

        // -------------------------------------------------------------------
        // Step 3: two-pass parse
        // -------------------------------------------------------------------
        let parsed_value: Option<serde_json::Value> = serde_json::from_str(&raw_str).ok();
        let parse_result: std::result::Result<CimdDocument, CimdParseErrorRepr> =
            match &parsed_value {
                Some(v) => cimd_parser::extract(v).map_err(CimdParseErrorRepr::from),
                None => Err(CimdParseErrorRepr::InvalidJson),
            };

        // -------------------------------------------------------------------
        // Step 4: BEGIN IMMEDIATE — read existing, then write
        // -------------------------------------------------------------------
        let txn = self
            .db
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                ..Default::default()
            })
            .await
            .context_to()?;

        let existing = oauth_client::Entity::find_by_id(url.to_owned())
            .one(&txn)
            .await
            .context_to()?;

        match parse_result {
            Ok(doc) => {
                // -----------------------------------------------------------
                // Validate client_id matches URL.
                // -----------------------------------------------------------
                if doc.client_id != url {
                    drop(txn);
                    bail!(CimdFetchError::ClientIdMismatch);
                }

                let is_first_use = existing.is_none();

                // Material-change detection — compare normalised hashes.
                let new_material_hash = parsed_value
                    .as_ref()
                    .map(compute_material_hash)
                    .unwrap_or_default();
                let previous_material_hash = existing
                    .as_ref()
                    .and_then(|m| m.metadata_raw.as_deref())
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .as_ref()
                    .map(compute_material_hash);
                let is_material_change = previous_material_hash
                    .as_deref()
                    .is_some_and(|prev| prev != new_material_hash);

                if is_material_change {
                    oauth_consent::Entity::update_many()
                        .col_expr(
                            oauth_consent::Column::RevalidationRequiredAt,
                            sea_orm::sea_query::Expr::value(Some(now)),
                        )
                        .filter(oauth_consent::Column::ClientId.eq(url))
                        .filter(oauth_consent::Column::RevokedAt.is_null())
                        .filter(oauth_consent::Column::RevalidationRequiredAt.is_null())
                        .exec(&txn)
                        .await
                        .context_to()?;
                }

                let redirect_uris_json =
                    serde_json::to_string(&doc.redirect_uris).unwrap_or_else(|_| "[]".to_owned());

                // Upsert: prefer update over insert when row exists, since
                // SeaORM's `save()` semantics depend on primary-key state and
                // can be subtle for TEXT primary keys.
                let final_model = match existing.clone() {
                    Some(existing_row) => {
                        let mut active = existing_row.into_active_model();
                        active.client_name = Set(doc.client_name.clone());
                        active.redirect_uris = Set(redirect_uris_json);
                        active.metadata_cached_at = Set(Some(now));
                        active.metadata_etag = Set(etag);
                        active.metadata_content_hash = Set(Some(content_hash));
                        active.metadata_raw = Set(Some(raw_str));
                        active.metadata_parse_error = Set(None);
                        active.metadata_parse_error_at = Set(None);
                        active.update(&txn).await.context_to()?
                    }
                    None => {
                        let active = oauth_client::ActiveModel {
                            id: Set(url.to_owned()),
                            client_name: Set(doc.client_name.clone()),
                            client_uri: Set(None),
                            logo_uri: Set(None),
                            redirect_uris: Set(redirect_uris_json),
                            default_scope: Set(String::new()),
                            grant_types: Set("[\"authorization_code\"]".to_owned()),
                            response_types: Set("[\"code\"]".to_owned()),
                            token_endpoint_auth_method: Set("none".to_owned()),
                            client_secret_hash: Set(None),
                            registration_access_token_hash: Set(None),
                            created_via: Set("cimd_cache".to_owned()),
                            created_at: Set(now),
                            last_used_at: Set(None),
                            revoked_at: Set(None),
                            metadata_cached_at: Set(Some(now)),
                            metadata_etag: Set(etag),
                            metadata_content_hash: Set(Some(content_hash)),
                            metadata_raw: Set(Some(raw_str)),
                            metadata_parse_error: Set(None),
                            metadata_parse_error_at: Set(None),
                            trusted_at: Set(None),
                        };
                        active.insert(&txn).await.context_to()?
                    }
                };

                txn.commit().await.context_to()?;

                // ---- Audit emission after commit ---------------------------
                if is_first_use {
                    self.emit_audit_event(
                        AuditActionType::OAUTH_CLIENT_FIRST_USE,
                        AuditOutcome::Success,
                        serde_json::json!({ "client_id": url }),
                    );
                } else {
                    self.emit_audit_event(
                        AuditActionType::OAUTH_CLIENT_METADATA_REFRESHED,
                        AuditOutcome::Success,
                        serde_json::json!({ "client_id": url }),
                    );
                }
                if is_material_change {
                    self.emit_audit_event(
                        AuditActionType::OAUTH_CLIENT_METADATA_CHANGED_MATERIALLY,
                        AuditOutcome::Success,
                        serde_json::json!({ "client_id": url }),
                    );
                }

                Ok(final_model)
            }
            Err(parse_err) => {
                // -----------------------------------------------------------
                // Parse failure: preserve existing row, update only error
                // bookkeeping columns. If no existing row exists, fail hard.
                // -----------------------------------------------------------
                let parse_err_str = parse_err.to_string();

                let final_model = match existing {
                    Some(existing_row) => {
                        let mut active = existing_row.into_active_model();
                        active.metadata_parse_error = Set(Some(parse_err_str.clone()));
                        active.metadata_parse_error_at = Set(Some(now));
                        let model = active.update(&txn).await.context_to()?;
                        txn.commit().await.context_to()?;
                        model
                    }
                    None => {
                        drop(txn);
                        self.emit_audit_event(
                            AuditActionType::OAUTH_CIMD_PARSE_FAILED,
                            AuditOutcome::Denied,
                            serde_json::json!({
                                "client_id": url,
                                "error": parse_err_str,
                            }),
                        );
                        bail!(CimdFetchError::UnparseableFirstFetch);
                    }
                };

                self.emit_audit_event(
                    AuditActionType::OAUTH_CIMD_PARSE_FAILED,
                    AuditOutcome::Denied,
                    serde_json::json!({
                        "client_id": url,
                        "error": parse_err_str,
                    }),
                );

                Ok(final_model)
            }
        }
    }

    fn emit_audit_event(
        &self,
        action: impl Into<AuditActionType>,
        outcome: AuditOutcome,
        details: serde_json::Value,
    ) {
        match AuditEntry::<Event>::builder_event(action)
            .actor_system()
            .outcome(outcome)
            .details(details)
            .build()
        {
            Ok(entry) => self.audit_emitter.emit_event(entry),
            Err(err) => {
                tracing::warn!(error = %err, "dropping invalid CIMD audit entry");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal representation of parse failure
// ---------------------------------------------------------------------------

/// Combines [`serde_json::Error`] and [`CimdParseError`] into one
/// `Display`-only enum for audit detail strings.
#[derive(Debug)]
enum CimdParseErrorRepr {
    InvalidJson,
    Cimd(CimdParseError),
}

impl From<CimdParseError> for CimdParseErrorRepr {
    fn from(e: CimdParseError) -> Self {
        Self::Cimd(e)
    }
}

impl std::fmt::Display for CimdParseErrorRepr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson => f.write_str("invalid JSON"),
            Self::Cimd(e) => std::fmt::Display::fmt(e, f),
        }
    }
}

// ---------------------------------------------------------------------------
// Material-change hashing
// ---------------------------------------------------------------------------

/// Compute a deterministic SHA-256 hash of the CIMD document with cosmetic
/// fields stripped per spec §11.3.
///
/// Returns an empty string if `value` is not a JSON object.
fn compute_material_hash(value: &serde_json::Value) -> String {
    let serde_json::Value::Object(map) = value else {
        return String::new();
    };

    // BTreeMap gives a deterministic key ordering for the serialised form,
    // which serde_json::to_string preserves in turn.
    let filtered: BTreeMap<&str, &serde_json::Value> = map
        .iter()
        .filter(|(k, _)| !COSMETIC_FIELDS.contains(&k.as_str()))
        .map(|(k, v)| (k.as_str(), v))
        .collect();

    let normalised = serde_json::to_string(&filtered).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(normalised.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test assertions — panics on setup failure are acceptable in tests"
    )]

    use super::*;
    use crate::test_harness::setup_migrated_db;
    use httpmock::prelude::*;
    use parking_lot::Mutex;
    use uptrakit_audit_log::{AuditEmitter, AuditLogDispatcher, NoopBackend};

    fn make_emitter() -> Arc<AuditEmitter> {
        let dispatcher = AuditLogDispatcher::new(Arc::new(NoopBackend));
        Arc::new(AuditEmitter::new(dispatcher))
    }

    fn make_clock(
        cell: Arc<Mutex<OffsetDateTime>>,
    ) -> Arc<dyn Fn() -> OffsetDateTime + Send + Sync> {
        Arc::new(move || *cell.lock())
    }

    fn make_fetcher(db: DatabaseConnection, clock_cell: Arc<Mutex<OffsetDateTime>>) -> CimdFetcher {
        CimdFetcher::new_permissive(db, make_clock(clock_cell), make_emitter())
            .expect("build CimdFetcher")
    }

    fn cimd_body(url: &str, name: &str) -> serde_json::Value {
        serde_json::json!({
            "client_id": url,
            "redirect_uris": [format!("{url}/callback")],
            "client_name": name,
        })
    }

    // ──────────────────────────────────────────────────────────────────────
    // Task 8 — happy path
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_and_upsert_happy_path_inserts_row() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let fetcher = make_fetcher(db.clone(), clock_cell);

        let server = MockServer::start_async().await;
        let base = server.base_url();
        let body = cimd_body(&base, "Test App");
        let body_str = body.to_string();

        let mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/");
                then.status(200)
                    .header("content-type", "application/json")
                    .header("etag", "\"abc\"")
                    .body(&body_str);
            })
            .await;

        let model = fetcher
            .fetch_and_upsert(&base)
            .await
            .expect("happy path should succeed");

        mock.assert_async().await;
        assert_eq!(model.id, base);
        assert_eq!(model.created_via, "cimd_cache");
        assert_eq!(model.client_name, "Test App");
        assert!(model.metadata_content_hash.is_some());
        assert!(model.metadata_raw.is_some());
        assert!(model.metadata_cached_at.is_some());
        assert_eq!(model.metadata_etag.as_deref(), Some("\"abc\""));
        assert!(model.metadata_parse_error.is_none());
        assert!(model.metadata_parse_error_at.is_none());
    }

    #[tokio::test]
    async fn fetch_and_upsert_rejects_oversized_body() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let fetcher = make_fetcher(db, clock_cell);

        let server = MockServer::start_async().await;
        let huge_body = "x".repeat(MAX_BODY_BYTES + 1);
        let _mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/");
                then.status(200).body(&huge_body);
            })
            .await;

        let err = fetcher
            .fetch_and_upsert(&server.base_url())
            .await
            .expect_err("oversized body should be rejected");
        assert!(matches!(
            err.current_context(),
            CimdFetchError::BodyTooLarge
        ));
    }

    #[tokio::test]
    async fn fetch_and_upsert_rejects_client_id_mismatch() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let fetcher = make_fetcher(db, clock_cell);

        let server = MockServer::start_async().await;
        let bogus = cimd_body("https://other.example.com", "Imposter");
        let body_str = bogus.to_string();
        let _mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/");
                then.status(200).body(&body_str);
            })
            .await;

        let err = fetcher
            .fetch_and_upsert(&server.base_url())
            .await
            .expect_err("mismatched client_id should be rejected");
        assert!(matches!(
            err.current_context(),
            CimdFetchError::ClientIdMismatch
        ));
    }
}
