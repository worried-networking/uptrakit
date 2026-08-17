//! CIMD (Client Identifier Metadata Document) fetcher and cache manager.
//!
//! Per spec §11.3: fetch → raw-bytes persist → two-pass parse → upsert.
//! Parse failures preserve the existing cached row and never invalidate it,
//! so an upstream CIMD draft revision cannot trigger forced re-consent.
//!
//! Material-change detection compares a normalised hash (cosmetic fields
//! excluded) so updates to display-only fields like `tos_uri` or
//! `software_version` do not invalidate active consents.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;
use uptrakit_shared_db::begin_immediate;

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel, QueryFilter,
    Set,
};
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_audit_log::{AuditActionType, AuditEmitter, AuditEntry, AuditOutcome, Event};
use uptrakit_shared_db::entity::{oauth_client, oauth_consent};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_shared_types::ssrf::SsrfSafeResolver;

use uptrakit_web_api_auth::auth::rate_limit::{RateLimitOutcome, RateLimitStore};
use uptrakit_web_api_auth::{SettingKey, settings_store::load_global_setting};

use crate::oauth::cimd_parser::{self, CimdDocument, CimdParseError};
use crate::oauth::rate_limit::{EndpointKind, OAuthRateLimiter};

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
    /// Rate limit exceeded for this `(source_ip, metadata_url)` bucket.
    ///
    /// Per spec §14.2: 5 requests per minute per `(source_ip, metadata_url)`.
    #[error("rate limited: too many CIMD fetch attempts for this source IP + URL")]
    RateLimited,
    /// Database error during upsert or cascade-revalidation.
    #[error("database error")]
    Database(sea_orm::DbErr),
}

impl CimdFetchError {
    /// Returns `true` when this error is a rate-limit rejection.
    ///
    /// Callers in route handlers use this instead of `match .current_context()`
    /// to stay compatible with the project's `check_legacy_error_matches` gate.
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::RateLimited)
    }
}

impl_report_conversion!(sea_orm::DbErr => CimdFetchError::Database);
impl_report_conversion!(reqwest::Error => CimdFetchError::Http);

/// 64 KB body cap per spec §11.3 — protects against unbounded CIMD documents.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// Cosmetic fields excluded from the material-change hash per spec §11.3.
///
/// Changes to these fields do not invalidate active consents.
const COSMETIC_FIELDS: &[&str] = &["tos_uri", "policy_uri", "software_version", "software_id"];

/// Draft version hint included in `OAUTH_CIMD_PARSE_FAILED` audit entries.
const CIMD_DRAFT_VERSION_HINT: &str = "draft-ietf-oauth-client-id-metadata-document-00";

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
    rate_limiter: OAuthRateLimiter,
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
        let rate_limiter = OAuthRateLimiter::new(RateLimitStore::new(db.clone()));
        Ok(Self {
            client,
            clock,
            db,
            audit_emitter,
            rate_limiter,
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
    /// - [`CimdFetchError::RateLimited`] — rate limit exceeded for `(source_ip, url)` bucket
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
        source_ip: Option<&str>,
    ) -> std::result::Result<oauth_client::Model, Report<CimdFetchError>> {
        // -------------------------------------------------------------------
        // Step 0: Rate-limit check per spec §14.2 — 5/min per (source_ip, url).
        // -------------------------------------------------------------------
        let bucket = format!("{}:{}", source_ip.unwrap_or("unknown"), url);
        match self
            .rate_limiter
            .check(EndpointKind::CimdFetch, &bucket)
            .await
        {
            Ok(RateLimitOutcome::Limited { .. }) => bail!(CimdFetchError::RateLimited),
            Ok(RateLimitOutcome::Allowed) => {}
            Err(e) => {
                tracing::error!(error = %e, "CIMD rate limit check failed; allowing request");
            }
        }

        // -------------------------------------------------------------------
        // Step 0b: Load operator-configured cosmetic field allowlist.
        // -------------------------------------------------------------------
        let extra_cosmetic: Vec<String> = match load_global_setting(
            &self.db,
            SettingKey::OauthCimdCosmeticFieldAllowlist,
        )
        .await
        {
            Ok(Some(v)) => v
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| e.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            Ok(None) => vec![],
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to load oauth.cimd_cosmetic_field_allowlist; using defaults"
                );
                vec![]
            }
        };

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
        let content_hash = hex::encode(hasher.finalize());

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
        let txn = begin_immediate(&self.db).await.context_to()?;

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
                    .map(|v| compute_material_hash(v, &extra_cosmetic))
                    .unwrap_or_default();
                let previous_material_value: Option<serde_json::Value> = existing
                    .as_ref()
                    .and_then(|m| m.metadata_raw.as_deref())
                    .and_then(|raw| serde_json::from_str(raw).ok());
                let previous_material_hash = previous_material_value
                    .as_ref()
                    .map(|v| compute_material_hash(v, &extra_cosmetic));
                let is_material_change = previous_material_hash
                    .as_deref()
                    .is_some_and(|prev| prev != new_material_hash);

                // Build a per-field diff for the material-change audit entry
                // (spec §14.1). Computed now so it is available after commit.
                let material_diff = if is_material_change {
                    let prev_obj = previous_material_value.as_ref().and_then(|v| v.as_object());
                    let new_obj = parsed_value.as_ref().and_then(|v| v.as_object());
                    let mut all_keys = BTreeSet::new();
                    if let Some(m) = prev_obj {
                        all_keys.extend(m.keys().map(String::as_str));
                    }
                    if let Some(m) = new_obj {
                        all_keys.extend(m.keys().map(String::as_str));
                    }
                    let mut diff = serde_json::Map::new();
                    for key in all_keys.into_iter().filter(|k| {
                        !COSMETIC_FIELDS.contains(k)
                            && !extra_cosmetic.iter().any(|e| e.as_str() == *k)
                    }) {
                        let before = prev_obj.and_then(|m| m.get(key));
                        let after = new_obj.and_then(|m| m.get(key));
                        if before != after {
                            diff.insert(
                                key.to_owned(),
                                serde_json::json!({ "before": before, "after": after }),
                            );
                        }
                    }
                    serde_json::Value::Object(diff)
                } else {
                    serde_json::Value::Object(serde_json::Map::new())
                };

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
                    serde_json::to_string(&doc.redirect_uris).unwrap_or_else(|e| {
                        tracing::warn!(
                            error = %e,
                            client_id = %url,
                            "failed to serialise redirect_uris; defaulting to empty array"
                        );
                        "[]".to_owned()
                    });

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
                        serde_json::json!({ "client_id": url, "changed_fields": material_diff }),
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
                                "draft_version_hint": CIMD_DRAFT_VERSION_HINT,
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
                        "draft_version_hint": CIMD_DRAFT_VERSION_HINT,
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
fn compute_material_hash(value: &serde_json::Value, extra_cosmetic: &[String]) -> String {
    let serde_json::Value::Object(map) = value else {
        return String::new();
    };

    // BTreeMap gives a deterministic key ordering for the serialised form,
    // which serde_json::to_string preserves in turn.
    let filtered: BTreeMap<&str, &serde_json::Value> = map
        .iter()
        .filter(|(k, _)| {
            !COSMETIC_FIELDS.contains(&k.as_str())
                && !extra_cosmetic.iter().any(|e| e.as_str() == k.as_str())
        })
        .map(|(k, v)| (k.as_str(), v))
        .collect();

    let normalised = match serde_json::to_string(&filtered) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to serialise material-hash input; returning empty hash");
            return String::new();
        }
    };
    let mut hasher = Sha256::new();
    hasher.update(normalised.as_bytes());
    hex::encode(hasher.finalize())
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
    use crate::oauth::rate_limit::EndpointKind;
    use crate::test_harness::setup_migrated_db;
    use httpmock::prelude::*;
    use parking_lot::Mutex;
    use sea_orm::ActiveModelTrait;
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
            .fetch_and_upsert(&base, None)
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
            .fetch_and_upsert(&server.base_url(), None)
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
            .fetch_and_upsert(&server.base_url(), None)
            .await
            .expect_err("mismatched client_id should be rejected");
        assert!(matches!(
            err.current_context(),
            CimdFetchError::ClientIdMismatch
        ));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Task 9 — parse failure preservation
    // ──────────────────────────────────────────────────────────────────────

    /// Helper: pre-seed a cached row so we can re-fetch and observe
    /// the parse-failure preservation branch.
    async fn seed_cached_row(
        db: &DatabaseConnection,
        url: &str,
        now: OffsetDateTime,
    ) -> oauth_client::Model {
        let active = oauth_client::ActiveModel {
            id: Set(url.to_owned()),
            client_name: Set("Seeded App".to_owned()),
            client_uri: Set(None),
            logo_uri: Set(None),
            redirect_uris: Set("[\"https://seeded.example.com/callback\"]".to_owned()),
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
            metadata_etag: Set(None),
            metadata_content_hash: Set(Some("seed-hash".to_owned())),
            metadata_raw: Set(Some(
                serde_json::json!({
                    "client_id": url,
                    "redirect_uris": ["https://seeded.example.com/callback"],
                    "client_name": "Seeded App",
                })
                .to_string(),
            )),
            metadata_parse_error: Set(None),
            metadata_parse_error_at: Set(None),
            trusted_at: Set(None),
        };
        active.insert(db).await.expect("seed row")
    }

    #[tokio::test]
    async fn fetch_parse_failure_preserves_existing_row_on_refetch() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let fetcher = make_fetcher(db.clone(), Arc::clone(&clock_cell));

        let server = MockServer::start_async().await;
        let url = server.base_url();
        let now = *clock_cell.lock();
        let original = seed_cached_row(&db, &url, now).await;

        // Re-fetch produces garbage bytes.
        let _mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/");
                then.status(200).body("this is not json {");
            })
            .await;

        let refetched = fetcher
            .fetch_and_upsert(&url, None)
            .await
            .expect("parse failure on re-fetch should not error");

        // Existing fields preserved.
        assert_eq!(refetched.client_name, original.client_name);
        assert_eq!(refetched.redirect_uris, original.redirect_uris);
        assert_eq!(
            refetched.metadata_content_hash, original.metadata_content_hash,
            "content hash must not be replaced on parse failure"
        );
        assert_eq!(
            refetched.metadata_raw, original.metadata_raw,
            "raw must not be replaced on parse failure (preserve prior good value)"
        );
        // Error bookkeeping is updated.
        assert!(refetched.metadata_parse_error.is_some());
        assert!(refetched.metadata_parse_error_at.is_some());
    }

    #[tokio::test]
    async fn fetch_parse_failure_first_fetch_returns_error() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let fetcher = make_fetcher(db.clone(), clock_cell);

        let server = MockServer::start_async().await;
        let _mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/");
                then.status(200).body("not-json");
            })
            .await;

        let err = fetcher
            .fetch_and_upsert(&server.base_url(), None)
            .await
            .expect_err("first fetch with unparseable bytes must error");
        assert!(matches!(
            err.current_context(),
            CimdFetchError::UnparseableFirstFetch
        ));

        // No row was created.
        let count = oauth_client::Entity::find()
            .all(&db)
            .await
            .expect("query")
            .len();
        assert_eq!(count, 0);
    }

    // ──────────────────────────────────────────────────────────────────────
    // Task 10 — material-change detection forces re-consent
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn material_change_marks_active_consents_for_revalidation() {
        use uptrakit_shared_db::entity::user;
        use uuid::Uuid;

        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let now = *clock_cell.lock();
        let fetcher = make_fetcher(db.clone(), Arc::clone(&clock_cell));

        let server = MockServer::start_async().await;
        let url = server.base_url();
        seed_cached_row(&db, &url, now).await;

        // Insert a user (FK target) and a consent for this client.
        let user_id = Uuid::now_v7();
        user::ActiveModel {
            id: Set(user_id),
            email: Set(uptrakit_shared_types::MaskedEmail::new(format!(
                "u-{user_id}@example.com"
            ))),
            first_name: Set("U".into()),
            last_name: Set("Ser".into()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert user");

        let consent_id = Uuid::now_v7();
        oauth_consent::ActiveModel {
            id: Set(consent_id),
            user_id: Set(user_id),
            client_id: Set(url.clone()),
            scopes: Set("mcp:read".to_owned()),
            cimd_content_hash_at_grant: Set(None),
            revalidation_required_at: Set(None),
            granted_at: Set(now),
            revoked_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert consent");

        // Now re-fetch with a *materially* different client_name field.
        let new_body = serde_json::json!({
            "client_id": url,
            "redirect_uris": ["https://seeded.example.com/callback"],
            "client_name": "RENAMED",
        });
        let body_str = new_body.to_string();
        let _mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/");
                then.status(200).body(&body_str);
            })
            .await;

        let refetched = fetcher
            .fetch_and_upsert(&url, None)
            .await
            .expect("refetch should succeed");
        assert_eq!(refetched.client_name, "RENAMED");

        // The consent row must now have revalidation_required_at set.
        let consent_after = oauth_consent::Entity::find_by_id(consent_id)
            .one(&db)
            .await
            .expect("query")
            .expect("consent row");
        assert!(
            consent_after.revalidation_required_at.is_some(),
            "active consent should be marked for revalidation on material change"
        );
    }

    #[tokio::test]
    async fn cosmetic_change_does_not_force_revalidation() {
        use uptrakit_shared_db::entity::user;
        use uuid::Uuid;

        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let now = *clock_cell.lock();
        let fetcher = make_fetcher(db.clone(), Arc::clone(&clock_cell));

        let server = MockServer::start_async().await;
        let url = server.base_url();

        // Seed with a row whose metadata_raw contains exactly the same
        // material fields we'll fetch — only a cosmetic `tos_uri` will differ.
        let seed_body = serde_json::json!({
            "client_id": url,
            "redirect_uris": [format!("{url}/callback")],
            "client_name": "App",
            "tos_uri": "https://old.example.com/tos",
        });
        let active = oauth_client::ActiveModel {
            id: Set(url.clone()),
            client_name: Set("App".to_owned()),
            client_uri: Set(None),
            logo_uri: Set(None),
            redirect_uris: Set(format!("[\"{url}/callback\"]")),
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
            metadata_etag: Set(None),
            metadata_content_hash: Set(Some("seed-hash".to_owned())),
            metadata_raw: Set(Some(seed_body.to_string())),
            metadata_parse_error: Set(None),
            metadata_parse_error_at: Set(None),
            trusted_at: Set(None),
        };
        active.insert(&db).await.expect("seed row");

        // Insert user and consent.
        let user_id = Uuid::now_v7();
        user::ActiveModel {
            id: Set(user_id),
            email: Set(uptrakit_shared_types::MaskedEmail::new(format!(
                "u-{user_id}@example.com"
            ))),
            first_name: Set("U".into()),
            last_name: Set("Ser".into()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert user");

        let consent_id = Uuid::now_v7();
        oauth_consent::ActiveModel {
            id: Set(consent_id),
            user_id: Set(user_id),
            client_id: Set(url.clone()),
            scopes: Set("mcp:read".to_owned()),
            cimd_content_hash_at_grant: Set(None),
            revalidation_required_at: Set(None),
            granted_at: Set(now),
            revoked_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert consent");

        // Re-fetch with the same material fields but a different `tos_uri`.
        let refetched_body = serde_json::json!({
            "client_id": url,
            "redirect_uris": [format!("{url}/callback")],
            "client_name": "App",
            "tos_uri": "https://NEW.example.com/tos",
        });
        let body_str = refetched_body.to_string();
        let _mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/");
                then.status(200).body(&body_str);
            })
            .await;

        fetcher
            .fetch_and_upsert(&url, None)
            .await
            .expect("refetch should succeed");

        // Consent must still be active (no revalidation_required_at).
        let consent_after = oauth_consent::Entity::find_by_id(consent_id)
            .one(&db)
            .await
            .expect("query")
            .expect("consent row");
        assert!(
            consent_after.revalidation_required_at.is_none(),
            "cosmetic-only change must NOT force revalidation"
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // Task 12 — CIMD rate-limit per spec §14.2
    // ──────────────────────────────────────────────────────────────────────

    /// Exhaust the per-(source_ip, url) bucket and verify the (N+1)-th call
    /// returns `CimdFetchError::RateLimited` without hitting the network.
    ///
    /// The `CimdFetch` window is 5 requests per minute. We issue 5 calls with
    /// a fixed source IP + URL, then verify the 6th is rate-limited.
    #[tokio::test]
    async fn fetch_and_upsert_rate_limited_after_window_exhausted() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let fetcher = make_fetcher(db.clone(), Arc::clone(&clock_cell));

        let server = MockServer::start_async().await;
        let url = server.base_url();
        let body = cimd_body(&url, "Rate Test App");
        let body_str = body.to_string();

        // Serve a valid CIMD document for all successful requests.
        let _mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(&body_str);
            })
            .await;

        let source_ip = "203.0.113.50";
        let limit = EndpointKind::CimdFetch.default_per_window() as usize;

        // Exhaust the window.
        for i in 0..limit {
            fetcher
                .fetch_and_upsert(&url, Some(source_ip))
                .await
                .unwrap_or_else(|e| panic!("request {}/{limit} should succeed: {e}", i + 1));
        }

        // The (limit+1)-th request must be rate-limited.
        let err = fetcher
            .fetch_and_upsert(&url, Some(source_ip))
            .await
            .expect_err("request beyond window must be rate-limited");
        assert!(
            matches!(err.current_context(), CimdFetchError::RateLimited),
            "expected RateLimited, got: {:?}",
            err.current_context()
        );
    }

    /// A different source IP must not be affected by another IP's exhausted bucket.
    #[tokio::test]
    async fn fetch_and_upsert_different_ips_have_independent_buckets() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let fetcher = make_fetcher(db.clone(), Arc::clone(&clock_cell));

        let server = MockServer::start_async().await;
        let url = server.base_url();
        let body = cimd_body(&url, "Bucket Test App");
        let body_str = body.to_string();

        let _mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(&body_str);
            })
            .await;

        let ip_a = "203.0.113.60";
        let ip_b = "203.0.113.61";
        let limit = EndpointKind::CimdFetch.default_per_window() as usize;

        // Exhaust ip_a's bucket.
        for _ in 0..limit {
            drop(fetcher.fetch_and_upsert(&url, Some(ip_a)).await);
        }
        let rate_limited = fetcher
            .fetch_and_upsert(&url, Some(ip_a))
            .await
            .expect_err("ip_a should be rate-limited");
        assert!(matches!(
            rate_limited.current_context(),
            CimdFetchError::RateLimited
        ));

        // ip_b's first request must still be allowed.
        fetcher
            .fetch_and_upsert(&url, Some(ip_b))
            .await
            .expect("ip_b should not be affected by ip_a's exhausted bucket");
    }
}
