use crate::SettingKey;
use crate::auth::{password, token};
use crate::cert_signer::SignedCertBundle;
use crate::settings_store::load_setting;
use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, sea_query::Expr};
use std::net::IpAddr;
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_internal_wire::HostInfo;
use uptrakit_shared_db::entity::prelude::{
    RevocationReason, ServiceCertificate as AgentCertificate, ServiceHost as AgentHost,
};
use uptrakit_shared_db::entity::{
    host, prelude::Host, service as agent, service_certificate as agent_certificate,
    service_host as agent_host,
};
use uptrakit_shared_macros::impl_report_conversion;

pub use uptrakit_web_api_types::agents::AgentStatus;

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
    Database(#[from] sea_orm::DbErr),

    #[error("certificate signing error")]
    CertSigning,
}

impl_report_conversion!(sea_orm::DbErr => AgentRouteError::Database);

// --- Shared enrollment helpers (used by WS handlers) ---

/// Result of a successful enrollment.
pub(crate) struct EnrollResult {
    pub agent: agent::Model,
    pub enrollment_secret: String,
    pub status: AgentStatus,
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
    pub host_info: Option<&'a HostInfo>,
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
        host_info,
    } = params;
    if hostname.trim().is_empty() {
        return Err(report!(AgentRouteError::BadRequest(
            "hostname must not be empty".into()
        )));
    }

    // Generate agent_id server-side (single source of truth)
    let agent_id = uuid::Uuid::now_v7();

    // Determine status based on enrollment token
    let status = if let Some(enrollment_token) = enrollment_token {
        let token_hash = match load_setting(db, tenant_id, SettingKey::EnrollmentTokenHash).await {
            Ok(Some(v)) => match v.as_str() {
                Some(hash) => hash.to_string(),
                None => {
                    return Err(report!(AgentRouteError::Forbidden(
                        "No enrollment token configured".into()
                    )));
                }
            },
            Ok(None) => {
                return Err(report!(AgentRouteError::Forbidden(
                    "No enrollment token configured".into()
                )));
            }
            Err(e) => {
                tracing::error!("Failed to load enrollment token hash: {:?}", e);
                return Err(report!(AgentRouteError::Internal(
                    "Internal server error".into()
                )));
            }
        };

        match password::verify_password(enrollment_token, &token_hash) {
            Ok(true) => AgentStatus::Approved,
            Ok(false) => {
                return Err(report!(AgentRouteError::Forbidden(
                    "Invalid enrollment token".into()
                )));
            }
            Err(e) => {
                tracing::error!("Token verification error: {:?}", e);
                return Err(report!(AgentRouteError::Internal(
                    "Internal server error".into()
                )));
            }
        }
    } else {
        AgentStatus::Pending
    };

    let enrollment_secret = match token::generate_secure_token() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to generate enrollment secret: {:?}", e);
            return Err(report!(AgentRouteError::Internal(
                "Internal server error".into()
            )));
        }
    };
    let secret_hash = token::hash_token(&enrollment_secret);

    let ip_str = ip_address.map(|ip| ip.to_string());
    let _ = settings; // settings available for future use

    let now = OffsetDateTime::now_utc();
    let db_status = match status {
        AgentStatus::Pending => agent::ServiceStatus::Pending,
        AgentStatus::Approved => agent::ServiceStatus::Approved,
        AgentStatus::Rejected => agent::ServiceStatus::Rejected,
    };
    let model = agent::ActiveModel {
        id: Set(agent_id),
        tenant_id: Set(tenant_id),
        service_type: Set(agent::ServiceType::Agent),
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
    };

    let inserted = model.insert(db).await.context_to::<AgentRouteError>()?;

    // Link agent to host (non-fatal on failure)
    if let Some(info) = host_info
        && let Err(e) = find_or_create_host_and_link(
            db,
            tenant_id,
            inserted.id,
            info,
            hostname,
            ip_str.as_deref(),
        )
        .await
    {
        tracing::warn!(error = %e, "failed to link agent to host during enrollment");
    }

    Ok(EnrollResult {
        agent: inserted,
        enrollment_secret,
        status,
    })
}

/// Find or create a host by machine_id, then link it to the given agent.
///
/// Skips silently when `machine_id == "unknown"`.
pub(crate) async fn find_or_create_host_and_link(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    agent_id: uuid::Uuid,
    host_info: &HostInfo,
    hostname: &str,
    ip_address: Option<&str>,
) -> Result<(), Report<AgentRouteError>> {
    if host_info.machine_id == "unknown" {
        return Ok(());
    }

    let now = OffsetDateTime::now_utc();

    let existing = Host::find()
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::MachineId.eq(&host_info.machine_id))
        .one(db)
        .await
        .context_to::<AgentRouteError>()?;

    let host_id = if let Some(existing_host) = existing {
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
        active.last_seen_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(db).await.context_to::<AgentRouteError>()?;
        existing_host.id
    } else {
        // Create new host
        let host_id = token::generate_uuid();
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
            last_seen_at: Set(Some(now)),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        new_host.insert(db).await.context_to::<AgentRouteError>()?;
        host_id
    };

    // Upsert agent_host link — insert if not exists
    let existing_link = AgentHost::find_by_id((agent_id, host_id))
        .one(db)
        .await
        .context_to::<AgentRouteError>()?;

    if existing_link.is_none() {
        let link = agent_host::ActiveModel {
            service_id: Set(agent_id),
            host_id: Set(host_id),
            linked_at: Set(now),
        };
        link.insert(db).await.context_to::<AgentRouteError>()?;
    }

    Ok(())
}

/// Sign a certificate from the agent's CSR, invalidate enrollment secret.
pub(crate) async fn do_sign_csr(
    cert_signer: &dyn crate::cert_signer::AgentCertSigner,
    settings: &crate::settings::Settings,
    db: &sea_orm::DatabaseConnection,
    agent: agent::Model,
    csr_pem: &str,
) -> Result<SignedCertBundle, Report<AgentRouteError>> {
    if agent.status != agent::ServiceStatus::Approved {
        return Err(report!(AgentRouteError::Forbidden(
            "Agent is not approved".into()
        )));
    }

    let lifetime = time::Duration::days(i64::from(settings.agent_cert_lifetime_days().await));

    let ca_fp = cert_signer.active_ca_fingerprint();

    let bundle = cert_signer
        .sign_agent_csr(csr_pem, &agent.id, lifetime)
        .await
        .map_err(|e| {
            tracing::error!("Failed to sign agent certificate: {e}");
            report!(AgentRouteError::CertSigning)
        })?;

    // Record certificate in DB for revocation tracking
    if let Err(e) = record_certificate(db, agent.id, &bundle.cert_pem, &ca_fp).await {
        tracing::error!("Failed to record agent certificate: {:?}", e);
        return Err(report!(AgentRouteError::Internal(
            "Internal server error".into()
        )));
    }

    // Invalidate enrollment secret
    let invalidated_hash = token::hash_token(&token::generate_uuid().to_string());
    let now = OffsetDateTime::now_utc();
    let mut active: agent::ActiveModel = agent.into();
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

    let record = agent_certificate::ActiveModel {
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
    AgentCertificate::update_many()
        .col_expr(
            agent_certificate::Column::RevokedAt,
            Expr::value(Some(OffsetDateTime::now_utc())),
        )
        .col_expr(
            agent_certificate::Column::RevocationReason,
            Expr::value(Some(reason)),
        )
        .filter(agent_certificate::Column::CaFingerprint.eq(ca_fingerprint))
        .filter(agent_certificate::Column::SerialNumber.eq(serial_number))
        .filter(agent_certificate::Column::RevokedAt.is_null())
        .exec(db)
        .await
        .context_to::<CertRecordError>()?;
    Ok(())
}

fn parse_cert_metadata(
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
