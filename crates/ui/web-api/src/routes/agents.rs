#![expect(
    clippy::map_err_ignore,
    reason = "original parse errors carry no useful context; replaced with contextual messages"
)]

use crate::auth::{password, token};
use crate::cert_signer::SignedCertBundle;
use crate::queries::enrollment_tokens as et_queries;
use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, sea_query::Expr};
use std::net::IpAddr;
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::{RevocationReason, ServiceCertificate, ServiceHost};
use uptrakit_shared_db::entity::{
    host, prelude::Host, service, service_certificate, service_host, system_service,
    system_service_certificate,
};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_wire::HostInfo;

pub(crate) use uptrakit_web_api_types::services::ServiceStatus;

// ---------------------------------------------------------------------------
// System credential capability guard
// ---------------------------------------------------------------------------

/// Capabilities that require the `system_service` capability to request.
const SYSTEM_CREDENTIAL_CAPS: &[&str] = &[
    "database_access",
    "nats_access",
    "master_key_access",
    "ca_management",
];

// --- Agent route error type ---

#[derive(Debug, Error)]
pub(crate) enum AgentRouteError {
    #[error("{0}")]
    BadRequest(String),

    #[error("{0}")]
    Forbidden(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("database error: {0}")]
    Database(sea_orm::DbErr),

    #[error("certificate signing error")]
    CertSigning,
}

impl_report_conversion!(sea_orm::DbErr => AgentRouteError::Database);
impl_report_conversion!(
    crate::queries::system_enrollment_tokens::SystemEnrollmentTokenError => AgentRouteError,
    |e| AgentRouteError::Internal(e.to_string())
);

// --- Shared enrollment helpers (used by WS handlers) ---

/// Result of a successful enrollment.
pub(crate) struct EnrollResult {
    pub service: service::Model,
    pub enrollment_secret: String,
    pub status: ServiceStatus,
}

/// Parameters for [`do_enroll`].
pub(crate) struct EnrollParams<'a> {
    pub db: &'a sea_orm::DatabaseConnection,
    pub settings: &'a crate::settings::Settings,
    pub tenant_id: uuid::Uuid,
    pub hostname: &'a str,
    pub friendly_name: &'a str,
    pub enrollment_token: Option<&'a str>,
    pub ip_address: Option<IpAddr>,
    /// Serialized JSON capability set for the new service.
    pub capabilities_json: String,
    /// The binary/crate name of the enrolling service.
    pub service_app_name: String,
}

/// Core enrollment logic: creates agent record, returns model + plaintext secret.
///
/// The controller generates a UUIDv7 `agent_id` for the enrolling agent.
pub(crate) async fn do_enroll(
    params: EnrollParams<'_>,
) -> Result<EnrollResult, Report<AgentRouteError>> {
    let EnrollParams {
        db,
        settings,
        tenant_id,
        hostname,
        friendly_name,
        enrollment_token,
        ip_address,
        capabilities_json,
        service_app_name,
    } = params;

    // Credential guard: system credentials require the system_service capability.
    {
        let caps: Vec<String> = serde_json::from_str(&capabilities_json).unwrap_or_default();
        let has_system_service = caps.iter().any(|c| c == "system_service");
        let requests_system_creds = caps
            .iter()
            .any(|c| SYSTEM_CREDENTIAL_CAPS.contains(&c.as_str()));
        if requests_system_creds && !has_system_service {
            bail!(AgentRouteError::Forbidden(
                "system credentials (database_access, nats_access, master_key_access, \
                 ca_management) require the system_service capability"
                    .into(),
            ));
        }
    }

    if hostname.trim().is_empty() {
        bail!(AgentRouteError::BadRequest(
            "hostname must not be empty".into()
        ));
    }

    // Generate agent_id server-side (single source of truth)
    let agent_id = uuid::Uuid::now_v7();

    // Determine status based on enrollment token.
    // When a token is provided, iterate over all active tokens for the tenant
    // and verify the plaintext against each Argon2 hash. On match, check
    // capability intersection and auto-approve.
    let (status, enrollment_token_id) = if let Some(provided_token) = enrollment_token {
        let active_tokens = et_queries::find_active_tokens(db, tenant_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to load active enrollment tokens: {:?}", e);
                report!(AgentRouteError::Internal("Internal server error".into()))
            })?;

        if active_tokens.is_empty() {
            bail!(AgentRouteError::Forbidden(
                "No active enrollment tokens configured".into()
            ));
        }

        let mut matched_token = None;
        for tok in &active_tokens {
            match password::verify_password(provided_token, &tok.token_hash) {
                Ok(true) => {
                    matched_token = Some(tok);
                    break;
                }
                Ok(false) => continue,
                Err(e) => {
                    tracing::error!("Token verification error: {:?}", e);
                    bail!(AgentRouteError::Internal("Internal server error".into()));
                }
            }
        }

        let tok = match matched_token {
            Some(t) => t,
            None => {
                bail!(AgentRouteError::Forbidden(
                    "Invalid enrollment token".into()
                ));
            }
        };

        // Check capability intersection: if the token has allowed_capabilities,
        // at least one must overlap with the service's capabilities. NULL = wildcard.
        if let Some(ref allowed_caps_json) = tok.allowed_capabilities {
            let allowed: Vec<String> = serde_json::from_str(allowed_caps_json).unwrap_or_default();
            if !allowed.is_empty() {
                let service_caps: Vec<String> =
                    serde_json::from_str(&capabilities_json).unwrap_or_default();
                let has_overlap = allowed.iter().any(|a| service_caps.contains(a));
                if !has_overlap {
                    bail!(AgentRouteError::Forbidden(
                        "Enrollment token does not permit this service type".into()
                    ));
                }
            }
        }

        // Increment usage counter
        if let Err(e) = et_queries::increment_token_uses(db, tok.id).await {
            tracing::error!("Failed to increment token uses: {:?}", e);
            bail!(AgentRouteError::Internal("Internal server error".into()));
        }

        (ServiceStatus::Approved, Some(tok.id))
    } else {
        (ServiceStatus::Pending, None)
    };

    let enrollment_secret = match token::generate_secure_token() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to generate enrollment secret: {:?}", e);
            bail!(AgentRouteError::Internal("Internal server error".into()));
        }
    };
    let secret_hash = token::hash_token(&enrollment_secret);

    let ip_str = ip_address.map(|ip| ip.to_string());
    let _ = settings; // settings available for future use

    let now = OffsetDateTime::now_utc();
    let db_status = match status {
        ServiceStatus::Pending => service::ServiceStatus::Pending,
        ServiceStatus::Approved => service::ServiceStatus::Approved,
        ServiceStatus::Rejected => service::ServiceStatus::Rejected,
        ServiceStatus::Deactivated => service::ServiceStatus::Deactivated,
        _ => bail!(AgentRouteError::Internal(
            "unknown service status variant".into()
        )),
    };
    let model = service::ActiveModel {
        id: Set(agent_id),
        tenant_id: Set(tenant_id),
        capabilities: Set(capabilities_json),
        hostname: Set(hostname.to_string()),
        friendly_name: Set(friendly_name.to_string()),
        ip_address: Set(ip_str.clone()),
        status: Set(db_status),
        enrollment_secret_hash: Set(secret_hash),
        client_version: Set(None),
        last_seen_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        ping_interval_seconds: Set(None),
        enrollment_token_id: Set(enrollment_token_id),
        cert_lifetime_hours: Set(None),
        service_app_name: Set(Some(service_app_name)),
        is_embedded: Set(false),
        embedded_owner_key: Set(None),
    };

    let inserted = model.insert(db).await.context_to::<AgentRouteError>()?;

    Ok(EnrollResult {
        service: inserted,
        enrollment_secret,
        status,
    })
}

/// Find or create a host by machine_id, then link it to the given agent.
///
/// Returns `Ok(None)` when `machine_id == "unknown"` (skipped silently).
/// Returns `Ok(Some((host_id, is_new)))` where `is_new` is `true` when a new
/// host row was inserted and `false` when an existing host was updated.
pub(crate) async fn find_or_create_host_and_link(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    agent_id: uuid::Uuid,
    host_info: &HostInfo,
    hostname: &str,
    ip_address: Option<&str>,
) -> Result<Option<(uuid::Uuid, bool)>, Report<AgentRouteError>> {
    if host_info.machine_id == "unknown" {
        return Ok(None);
    }

    let now = OffsetDateTime::now_utc();

    let existing = Host::find()
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::MachineId.eq(&host_info.machine_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to::<AgentRouteError>()?;

    let (host_id, is_new) = if let Some(existing_host) = existing {
        // Update mutable fields
        let mut active: host::ActiveModel = existing_host.clone().into();
        active.hostname = Set(hostname.to_string());
        if let Some(ip) = ip_address {
            active.ip_address = Set(Some(ip.to_string()));
        }
        if let Some(ref os_type) = host_info.os_type {
            active.os_type = Set(Some(os_type.clone()));
        }
        if let Some(ref os_version) = host_info.os_version {
            active.os_version = Set(Some(os_version.clone()));
        }
        if let Some(ref architecture) = host_info.architecture {
            active.architecture = Set(Some(architecture.clone()));
        }
        if let Some(ref features) = host_info.features {
            active.host_features = Set(Some(serde_json::to_string(features).unwrap_or_default()));
        }
        active.last_seen_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(db).await.context_to::<AgentRouteError>()?;
        (existing_host.id, false)
    } else {
        // Create new host — prefer the agent-supplied UUID so agent-local and
        // controller UUIDs stay in sync (required for Proxmox FK operations).
        let host_id = host_info.agent_host_id.unwrap_or_else(token::generate_uuid);
        let new_host = host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set(host_info.machine_id.clone()),
            hostname: Set(hostname.to_string()),
            friendly_name: Set(hostname.to_string()),
            os_type: Set(host_info.os_type.clone()),
            os_version: Set(host_info.os_version.clone()),
            architecture: Set(host_info.architecture.clone()),
            ip_address: Set(ip_address.map(|s| s.to_string())),
            host_features: Set(host_info
                .features
                .as_ref()
                .and_then(|f| serde_json::to_string(f).ok())),
            last_seen_at: Set(Some(now)),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        new_host.insert(db).await.context_to::<AgentRouteError>()?;
        (host_id, true)
    };

    // Upsert agent_host link — insert if not exists
    let existing_link = ServiceHost::find_by_id((agent_id, host_id))
        .one(db)
        .await
        .context_to::<AgentRouteError>()?;

    if existing_link.is_none() {
        let link = service_host::ActiveModel {
            service_id: Set(agent_id),
            host_id: Set(host_id),
            linked_at: Set(now),
        };
        link.insert(db).await.context_to::<AgentRouteError>()?;
    }

    Ok(Some((host_id, is_new)))
}

/// Sign a certificate from the agent's CSR, invalidate enrollment secret.
pub(crate) async fn do_sign_csr(
    cert_signer: &dyn crate::cert_signer::AgentCertSigner,
    settings: &crate::settings::Settings,
    db: &sea_orm::DatabaseConnection,
    service: service::Model,
    csr_pem: &str,
) -> Result<SignedCertBundle, Report<AgentRouteError>> {
    if service.status != service::ServiceStatus::Approved {
        bail!(AgentRouteError::Forbidden("Agent is not approved".into()));
    }

    let effective_hours = service
        .cert_lifetime_hours
        .map(|h| h as u32)
        .unwrap_or_else(|| settings.agent_cert_lifetime_hours());
    let lifetime = time::Duration::hours(i64::from(effective_hours));

    let ca_fp = cert_signer.active_ca_fingerprint();

    let bundle = cert_signer
        .sign_agent_csr(csr_pem, &service.id, lifetime)
        .await
        .map_err(|e| {
            tracing::error!("Failed to sign agent certificate: {e}");
            report!(AgentRouteError::CertSigning)
        })?;

    // Record certificate in DB for revocation tracking
    if let Err(e) = record_certificate(db, service.id, &bundle.cert_pem, &ca_fp).await {
        tracing::error!("Failed to record agent certificate: {:?}", e);
        bail!(AgentRouteError::Internal("Internal server error".into()));
    }

    // Invalidate enrollment secret
    let invalidated_hash = token::hash_token(&token::generate_uuid().to_string());
    let now = OffsetDateTime::now_utc();
    let mut active: service::ActiveModel = service.into();
    active.enrollment_secret_hash = Set(invalidated_hash);
    active.last_seen_at = Set(Some(now));
    active.updated_at = Set(now);
    active.update(db).await.context_to::<AgentRouteError>()?;

    Ok(bundle)
}

// --- Certificate recording error type ---

#[derive(Debug, Error)]
pub(crate) enum CertRecordError {
    #[error("failed to parse PEM data")]
    PemParse,

    #[error("failed to parse X.509 certificate")]
    X509Parse,

    #[error("invalid certificate timestamp: {0}")]
    Timestamp(#[from] time::error::ComponentRange),

    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
}

impl_report_conversion! {
    sea_orm::DbErr              => CertRecordError::Database,
    time::error::ComponentRange => CertRecordError::Timestamp,
}

async fn record_certificate(
    db: &sea_orm::DatabaseConnection,
    agent_id: uuid::Uuid,
    cert_pem: &str,
    ca_fingerprint: &str,
) -> Result<(), Report<CertRecordError>> {
    let (serial, not_before, not_after) = parse_cert_metadata(cert_pem)?;

    let record = service_certificate::ActiveModel {
        ca_fingerprint: Set(ca_fingerprint.to_string()),
        serial_number: Set(serial),
        service_id: Set(agent_id),
        not_before: Set(not_before),
        not_after: Set(not_after),
        revoked_at: Set(None),
        revocation_reason: Set(None),
        created_at: Set(OffsetDateTime::now_utc()),
        last_seen_at: Set(None),
    };

    record.insert(db).await.context_to::<CertRecordError>()?;

    Ok(())
}

/// Revoke a certificate by serial number and CA fingerprint.
pub(crate) async fn revoke_certificate(
    db: &sea_orm::DatabaseConnection,
    serial_number: &str,
    ca_fingerprint: &str,
    reason: RevocationReason,
) -> Result<(), Report<CertRecordError>> {
    ServiceCertificate::update_many()
        .col_expr(
            service_certificate::Column::RevokedAt,
            Expr::value(Some(OffsetDateTime::now_utc())),
        )
        .col_expr(
            service_certificate::Column::RevocationReason,
            Expr::value(Some(reason)),
        )
        .filter(service_certificate::Column::CaFingerprint.eq(ca_fingerprint))
        .filter(service_certificate::Column::SerialNumber.eq(serial_number))
        .filter(service_certificate::Column::RevokedAt.is_null())
        .exec(db)
        .await
        .context_to::<CertRecordError>()?;
    Ok(())
}

pub(crate) fn parse_cert_metadata(
    pem: &str,
) -> Result<(String, OffsetDateTime, OffsetDateTime), Report<CertRecordError>> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|_| report!(CertRecordError::PemParse))?;
    let cert = pem_block
        .parse_x509()
        .map_err(|_| report!(CertRecordError::X509Parse))?;

    let serial = cert.raw_serial_as_string();
    let validity = cert.validity();
    let not_before = OffsetDateTime::from_unix_timestamp(validity.not_before.timestamp())
        .context_to::<CertRecordError>()?;
    let not_after = OffsetDateTime::from_unix_timestamp(validity.not_after.timestamp())
        .context_to::<CertRecordError>()?;

    Ok((serial, not_before, not_after))
}

// ---------------------------------------------------------------------------
// System service enrollment
// ---------------------------------------------------------------------------

/// Parameters for [`do_enroll_system_service`].
pub(crate) struct SystemServiceEnrollParams<'a> {
    pub db: &'a sea_orm::DatabaseConnection,
    pub hostname: &'a str,
    pub friendly_name: &'a str,
    pub enrollment_token: Option<&'a str>,
    pub ip_address: Option<IpAddr>,
    pub capabilities_json: String,
    pub service_app_name: String,
}

/// Result of a successful system service enrollment.
pub(crate) struct EnrollSystemServiceResult {
    pub system_service: system_service::Model,
    pub enrollment_secret: String,
    pub status: ServiceStatus,
}

/// Core enrollment logic for system (tenant-agnostic) services.
///
/// Inserts into `system_services` table. Token comparison uses Argon2id against
/// active system enrollment tokens stored in the `system_enrollment_tokens` table.
///
/// When a valid token is provided and matches, the service is auto-approved.
/// When no token is provided, the service is placed in Pending status.
/// An incorrect or invalid token is rejected with `Forbidden`.
pub(crate) async fn do_enroll_system_service(
    params: SystemServiceEnrollParams<'_>,
) -> Result<EnrollSystemServiceResult, Report<AgentRouteError>> {
    let SystemServiceEnrollParams {
        db,
        hostname,
        friendly_name,
        enrollment_token,
        ip_address,
        capabilities_json,
        service_app_name,
    } = params;

    if hostname.trim().is_empty() {
        bail!(AgentRouteError::BadRequest(
            "hostname must not be empty".into()
        ));
    }

    // Token comparison: Argon2id verify against all active system enrollment tokens.
    let (status, matched_token_id) = if let Some(provided_token) = enrollment_token {
        let active_tokens = crate::queries::system_enrollment_tokens::find_active_system_tokens(db)
            .await
            .context_to::<AgentRouteError>()?;

        let matched = active_tokens
            .iter()
            .find(|t| password::verify_password(provided_token, &t.token_hash).unwrap_or(false));

        if let Some(t) = matched {
            if let Err(e) =
                crate::queries::system_enrollment_tokens::increment_system_token_uses(db, t.id)
                    .await
            {
                tracing::error!("Failed to increment system enrollment token uses: {e}");
            }
            (ServiceStatus::Approved, Some(t.id))
        } else {
            bail!(AgentRouteError::Forbidden(
                "Invalid system enrollment token".into()
            ));
        }
    } else {
        (ServiceStatus::Pending, None)
    };

    let service_id = uuid::Uuid::now_v7();
    let enrollment_secret = match token::generate_secure_token() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to generate enrollment secret: {:?}", e);
            bail!(AgentRouteError::Internal("Internal server error".into()));
        }
    };
    let secret_hash = token::hash_token(&enrollment_secret);
    let ip_str = ip_address.map(|ip| ip.to_string());
    let now = OffsetDateTime::now_utc();

    let db_status = match status {
        ServiceStatus::Pending => system_service::SystemServiceStatus::Pending,
        ServiceStatus::Approved => system_service::SystemServiceStatus::Approved,
        ServiceStatus::Rejected => system_service::SystemServiceStatus::Rejected,
        ServiceStatus::Deactivated => system_service::SystemServiceStatus::Deactivated,
        _ => bail!(AgentRouteError::Internal(
            "unknown service status variant".into()
        )),
    };

    let model = system_service::ActiveModel {
        id: Set(service_id),
        capabilities: Set(capabilities_json),
        hostname: Set(hostname.to_string()),
        friendly_name: Set(friendly_name.to_string()),
        ip_address: Set(ip_str),
        status: Set(db_status),
        enrollment_secret_hash: Set(secret_hash),
        client_version: Set(None),
        last_seen_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        ping_interval_seconds: Set(None),
        cert_lifetime_hours: Set(None),
        system_enrollment_token_id: Set(matched_token_id),
        service_app_name: Set(Some(service_app_name)),
        is_embedded: Set(false),
        embedded_owner_key: Set(None),
    };

    let inserted = model.insert(db).await.context_to::<AgentRouteError>()?;

    Ok(EnrollSystemServiceResult {
        system_service: inserted,
        enrollment_secret,
        status,
    })
}

/// Sign a CSR for a system service at initial enrollment (first certificate
/// issuance). Records the cert in `system_service_certificates` and invalidates
/// the enrollment secret hash.
pub(crate) async fn do_sign_csr_for_system_service(
    cert_signer: &dyn crate::cert_signer::AgentCertSigner,
    settings: &crate::settings::Settings,
    db: &sea_orm::DatabaseConnection,
    svc: system_service::Model,
    csr_pem: &str,
) -> Result<SignedCertBundle, Report<AgentRouteError>> {
    if svc.status != system_service::SystemServiceStatus::Approved {
        bail!(AgentRouteError::Forbidden(
            "System service is not approved".into()
        ));
    }

    let effective_hours = svc
        .cert_lifetime_hours
        .map(|h| h as u32)
        .unwrap_or_else(|| settings.agent_cert_lifetime_hours());
    let lifetime = time::Duration::hours(i64::from(effective_hours));
    let ca_fp = cert_signer.active_ca_fingerprint();

    let bundle = cert_signer
        .sign_agent_csr(csr_pem, &svc.id, lifetime)
        .await
        .map_err(|e| {
            tracing::error!("Failed to sign system service certificate: {e}");
            report!(AgentRouteError::CertSigning)
        })?;

    // Record certificate in system_service_certificates.
    let (serial, not_before, not_after) = parse_cert_metadata(&bundle.cert_pem).map_err(|e| {
        tracing::error!("Failed to parse cert metadata: {e}");
        report!(AgentRouteError::Internal(
            "cert metadata parse failed".into()
        ))
    })?;

    let record = system_service_certificate::ActiveModel {
        ca_fingerprint: Set(ca_fp),
        serial_number: Set(serial),
        system_service_id: Set(svc.id),
        not_before: Set(not_before),
        not_after: Set(not_after),
        revoked_at: Set(None),
        revocation_reason: Set(None),
        created_at: Set(OffsetDateTime::now_utc()),
        last_seen_at: Set(None),
    };
    record.insert(db).await.context_to::<AgentRouteError>()?;

    // Invalidate enrollment secret.
    let invalidated_hash = token::hash_token(&token::generate_uuid().to_string());
    let now = OffsetDateTime::now_utc();
    let mut active: system_service::ActiveModel = svc.into();
    active.enrollment_secret_hash = Set(invalidated_hash);
    active.last_seen_at = Set(Some(now));
    active.updated_at = Set(now);
    active.update(db).await.context_to::<AgentRouteError>()?;

    Ok(bundle)
}

/// Revoke a system service certificate (marks as `CertificateRenewed`).
pub(crate) async fn revoke_system_certificate(
    db: &sea_orm::DatabaseConnection,
    serial_number: &str,
    ca_fingerprint: &str,
) -> Result<(), Report<CertRecordError>> {
    use uptrakit_shared_db::entity::system_service_certificate::SystemRevocationReason;

    system_service_certificate::Entity::update_many()
        .col_expr(
            system_service_certificate::Column::RevokedAt,
            Expr::value(Some(OffsetDateTime::now_utc())),
        )
        .col_expr(
            system_service_certificate::Column::RevocationReason,
            Expr::value(Some(SystemRevocationReason::CertificateRenewed)),
        )
        .filter(system_service_certificate::Column::CaFingerprint.eq(ca_fingerprint))
        .filter(system_service_certificate::Column::SerialNumber.eq(serial_number))
        .filter(system_service_certificate::Column::RevokedAt.is_null())
        .exec(db)
        .await
        .context_to::<CertRecordError>()?;
    Ok(())
}
