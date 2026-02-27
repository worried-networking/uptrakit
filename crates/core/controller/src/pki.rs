use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};
use rootcause::prelude::*;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    Set, TransactionTrait,
};
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{ca_certificate, setting};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api::SettingKey;
use uptrakit_web_api::pki_utils::{self, SanCollection};

/// Build the DER-encoded value for an Authority Information Access (AIA) extension.
///
/// The AIA extension (OID 1.3.6.1.5.5.7.1.1) contains:
/// - `id-ad-ocsp` access description pointing to the OCSP responder URL
/// - `id-ad-caIssuers` access description pointing to the CA certificate URL
fn build_aia_extension_der(ocsp_url: &str, ca_issuers_url: &str) -> Vec<u8> {
    let mut access_descriptions = Vec::new();

    // OCSP access description: SEQUENCE { OID(id-ad-ocsp), [6] URI }
    access_descriptions.extend_from_slice(&encode_access_description(
        &[0x06, 0x08, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x30, 0x01], // id-ad-ocsp OID
        ocsp_url,
    ));

    // CA Issuers access description: SEQUENCE { OID(id-ad-caIssuers), [6] URI }
    access_descriptions.extend_from_slice(&encode_access_description(
        &[0x06, 0x08, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x30, 0x02], // id-ad-caIssuers OID
        ca_issuers_url,
    ));

    // Wrap in SEQUENCE (AuthorityInfoAccessSyntax)
    encode_der_sequence(&access_descriptions)
}

/// Encode a single AccessDescription as a DER SEQUENCE.
fn encode_access_description(method_oid_der: &[u8], uri: &str) -> Vec<u8> {
    let uri_bytes = uri.as_bytes();
    // GeneralName uniformResourceIdentifier [6] IMPLICIT IA5String
    let mut general_name = vec![0x86]; // context tag 6, primitive
    general_name.extend_from_slice(&encode_der_length(uri_bytes.len()));
    general_name.extend_from_slice(uri_bytes);

    let mut content = Vec::new();
    content.extend_from_slice(method_oid_der);
    content.extend_from_slice(&general_name);

    encode_der_sequence(&content)
}

/// Encode a DER SEQUENCE tag + length + content.
fn encode_der_sequence(content: &[u8]) -> Vec<u8> {
    let mut result = vec![0x30]; // SEQUENCE tag
    result.extend_from_slice(&encode_der_length(content.len()));
    result.extend_from_slice(content);
    result
}

/// Encode a DER length in the minimum number of octets.
fn encode_der_length(len: usize) -> Vec<u8> {
    if len < 0x80 {
        vec![len as u8]
    } else if len < 0x100 {
        vec![0x81, len as u8]
    } else {
        vec![0x82, (len >> 8) as u8, len as u8]
    }
}

/// Add AIA and CDP extensions to certificate parameters when a backend URL is set.
pub fn add_pki_extensions(params: &mut CertificateParams, pki_addr: &str) {
    let ocsp_url = format!("{pki_addr}/api/v1/pki/ocsp");
    let ca_issuers_url = format!("{pki_addr}/api/v1/pki/ca.crt");
    let crl_url = format!("{pki_addr}/api/v1/pki/ca.crl");

    // AIA extension (OID 1.3.6.1.5.5.7.1.1)
    let aia_der = build_aia_extension_der(&ocsp_url, &ca_issuers_url);
    params
        .custom_extensions
        .push(rcgen::CustomExtension::from_oid_content(
            &[1, 3, 6, 1, 5, 5, 7, 1, 1],
            aia_der,
        ));

    // CDP extension (CRL Distribution Points)
    params.crl_distribution_points = vec![rcgen::CrlDistributionPoint {
        uris: vec![crl_url],
    }];
}

#[derive(Debug, Error)]
pub enum PkiError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("certificate generation error: {0}")]
    Rcgen(#[from] rcgen::Error),

    #[error("rustls error: {0}")]
    Rustls(#[from] rustls::Error),

    #[error("PEM parsing error")]
    PemParse,

    #[error("client verifier builder error: {0}")]
    VerifierBuilder(String),

    #[error("hostname resolution failed: {0}")]
    Hostname(String),

    #[error("timestamp error: {0}")]
    Timestamp(String),

    #[error("database error: {0}")]
    Database(String),

    #[error("directory operation failed")]
    Directory(#[from] uptrakit_directories::DirectoryError),

    #[error("encryption failed")]
    Crypto(#[from] uptrakit_crypto::CryptoError),

    #[error("CA validation failed: {0}")]
    CaValidation(String),
}

pub type Result<T> = std::result::Result<T, Report<PkiError>>;

impl_report_conversion! {
    std::io::Error => PkiError::Io,
    rcgen::Error => PkiError::Rcgen,
    rustls::Error => PkiError::Rustls,
    uptrakit_directories::DirectoryError => PkiError::Directory,
    uptrakit_crypto::CryptoError => PkiError::Crypto,
}

impl_report_conversion!(sea_orm::DbErr => PkiError, |e| PkiError::Database(e.to_string()));

impl_report_conversion!(pki_utils::PkiUtilError => PkiError, |e| match e {
    pki_utils::PkiUtilError::Hostname(s) => PkiError::Hostname(s),
    pki_utils::PkiUtilError::PemParse => PkiError::PemParse,
});

/// Loaded CA material.
pub struct CaBundle {
    pub cert_pem: String,
    pub key_pem: String,
    pub issuer: Issuer<'static, KeyPair>,
}

/// Loaded server certificate material.
pub struct ServerCertBundle {
    pub cert_pem: String,
    pub key_pem: String,
}

/// Active + optional previous CA state.
pub struct CaState {
    pub active: CaBundle,
    pub previous: Option<CaBundle>,
    pub trusted: Vec<CaBundle>,
    pub managed: bool,
}

/// Type alias for the canonical public CA snapshot type from the web-api crate.
pub type CaSnapshot = uptrakit_web_api::ca_snapshot::CaPublicSnapshot;

/// Type alias for the CA key store type from the web-api crate.
pub type CaKeyStore = uptrakit_web_api::ca_snapshot::CaKeyStore;

impl CaState {
    /// Build a PEM bundle of all trusted CA certs (active + historical).
    pub fn ca_bundle_pem(&self) -> String {
        let mut bundle = String::new();
        for ca in &self.trusted {
            if !bundle.is_empty() && !bundle.ends_with('\n') {
                bundle.push('\n');
            }
            bundle.push_str(&ca.cert_pem);
        }
        bundle
    }

    /// Build a shareable public snapshot and a private key store.
    pub fn to_snapshot(&self, pki_addr: Option<String>) -> Result<(CaSnapshot, CaKeyStore)> {
        let active_fingerprint = ca_fingerprint(&self.active.cert_pem)?;
        let previous_fingerprint = match &self.previous {
            Some(prev) => Some(ca_fingerprint(&prev.cert_pem)?),
            None => None,
        };
        let bundle_pem = self.ca_bundle_pem();
        let bundle_hash = sha256_hex(bundle_pem.as_bytes());
        let active_not_after = cert_not_after(&self.active.cert_pem)?;

        let mut trusted_cas_public = Vec::new();
        let mut trusted_ca_keys = Vec::new();
        for ca in &self.trusted {
            let fingerprint = ca_fingerprint(&ca.cert_pem)?;
            let not_after = cert_not_after(&ca.cert_pem)?;
            trusted_cas_public.push(uptrakit_web_api::ca_snapshot::TrustedCaPublic {
                cert_pem: ca.cert_pem.clone(),
                fingerprint: fingerprint.clone(),
                not_after,
            });
            trusted_ca_keys.push(uptrakit_web_api::ca_snapshot::TrustedCaKey {
                fingerprint,
                key_pem: zeroize::Zeroizing::new(ca.key_pem.clone()),
            });
        }

        let mut trusted_ca_cns = Vec::new();
        for ca in &self.trusted {
            if let Some(cn) = cert_common_name(&ca.cert_pem) {
                trusted_ca_cns.push(cn);
            }
        }

        Ok(uptrakit_web_api::ca_snapshot::split_snapshot(
            uptrakit_web_api::ca_snapshot::SplitSnapshotInput {
                active_cert_pem: self.active.cert_pem.clone(),
                active_key_pem: self.active.key_pem.clone(),
                active_fingerprint,
                previous_cert_pem: self.previous.as_ref().map(|p| p.cert_pem.clone()),
                previous_key_pem: self.previous.as_ref().map(|p| p.key_pem.clone()),
                previous_fingerprint,
                trusted_cas_public,
                trusted_ca_keys,
                trusted_ca_cns,
                bundle_pem,
                bundle_hash,
                managed: self.managed,
                active_not_after,
                pki_addr,
            },
        ))
    }
}

/// Ensure the PKI directory exists and return its path.
pub fn pki_dir(data_dir: &Path) -> Result<PathBuf> {
    let dir = data_dir.join("pki");
    fs::create_dir_all(&dir).context_to::<PkiError>()?;
    Ok(dir)
}

/// Load or initialize the managed CA from the database.
pub async fn load_or_init_managed_ca(
    db: &DatabaseConnection,
    tenant_id: uuid::Uuid,
    pki_addr: Option<&str>,
) -> Result<CaState> {
    let existing = load_active_ca_fingerprint(db, tenant_id).await?;
    if existing.is_some() {
        return load_managed_ca_state(db, tenant_id).await;
    }

    let tx = db.begin().await.context_to::<PkiError>()?;
    let current = load_active_ca_fingerprint(&tx, tenant_id).await?;
    if current.is_some() {
        tx.rollback().await.context_to::<PkiError>()?;
        return load_managed_ca_state(db, tenant_id).await;
    }

    let now = OffsetDateTime::now_utc();
    let bundle = generate_ca(pki_addr)?;
    let fingerprint = ca_fingerprint(&bundle.cert_pem)?;
    let not_before = cert_not_before(&bundle.cert_pem)?;
    let not_after = cert_not_after(&bundle.cert_pem)?;

    let inserted = insert_setting_string_if_absent(
        &tx,
        tenant_id,
        SettingKey::PkiActiveCaFingerprint,
        &fingerprint,
    )
    .await?;
    if !inserted {
        tx.rollback().await.context_to::<PkiError>()?;
        return load_managed_ca_state(db, tenant_id).await;
    }

    let encrypted_key =
        uptrakit_crypto::EncryptedString::new(bundle.key_pem.clone()).context_to()?;
    let cert_model = ca_certificate::ActiveModel {
        fingerprint: Set(fingerprint.clone()),
        cert_pem: Set(bundle.cert_pem.clone()),
        key_pem: Set(encrypted_key),
        not_before: Set(not_before),
        not_after: Set(not_after),
        activated_at: Set(now),
        deactivated_at: Set(None),
        created_at: Set(now),
    };
    ca_certificate::Entity::insert(cert_model)
        .exec(&tx)
        .await
        .context_to::<PkiError>()?;

    set_setting_i64(&tx, tenant_id, SettingKey::PkiCaVersion, 1).await?;

    tx.commit().await.context_to::<PkiError>()?;
    tracing::info!("generated new internal CA and stored in database");

    load_managed_ca_state(db, tenant_id).await
}

/// Load the full managed CA state from the database.
pub async fn load_managed_ca_state(
    db: &DatabaseConnection,
    tenant_id: uuid::Uuid,
) -> Result<CaState> {
    let active_fp = load_active_ca_fingerprint(db, tenant_id)
        .await?
        .ok_or_else(|| {
            report!(PkiError::CaValidation(
                "missing active CA fingerprint".into()
            ))
        })?;

    let active_model = ca_certificate::Entity::find_by_id(active_fp.clone())
        .one(db)
        .await
        .context_to::<PkiError>()?
        .ok_or_else(|| report!(PkiError::CaValidation("active CA record not found".into())))?;

    let now = OffsetDateTime::now_utc();
    let mut trusted_models = ca_certificate::Entity::find()
        .filter(ca_certificate::Column::NotAfter.gte(now))
        .all(db)
        .await
        .context_to::<PkiError>()?;

    if !trusted_models.iter().any(|m| m.fingerprint == active_fp) {
        bail!(PkiError::CaValidation(
            "active CA is expired or missing from trusted set".into()
        ));
    }

    trusted_models.sort_by(|a, b| {
        if a.fingerprint == active_fp && b.fingerprint != active_fp {
            return std::cmp::Ordering::Less;
        }
        if b.fingerprint == active_fp && a.fingerprint != active_fp {
            return std::cmp::Ordering::Greater;
        }

        let a_deact = a.deactivated_at;
        let b_deact = b.deactivated_at;
        match (a_deact, b_deact) {
            (Some(a_time), Some(b_time)) => b_time.cmp(&a_time),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => b.activated_at.cmp(&a.activated_at),
        }
        .then_with(|| b.activated_at.cmp(&a.activated_at))
        .then_with(|| a.fingerprint.cmp(&b.fingerprint))
    });

    let active = bundle_from_model(active_model)?;
    let previous = most_recent_deactivated(&trusted_models).and_then(|m| bundle_from_model(m).ok());

    let mut trusted = Vec::new();
    for model in trusted_models {
        let bundle = bundle_from_model(model)?;
        trusted.push(bundle);
    }

    Ok(CaState {
        active,
        previous,
        trusted,
        managed: true,
    })
}

/// Result of a CA rotation attempt.
pub struct RotationOutcome {
    pub rotated: bool,
    pub state: CaState,
}

/// Rotate the managed CA using a compare-and-swap guard in the database.
pub async fn rotate_managed_ca(
    db: &DatabaseConnection,
    tenant_id: uuid::Uuid,
    pki_addr: Option<&str>,
    expected_active_fp: &str,
) -> Result<RotationOutcome> {
    let tx = db.begin().await.context_to::<PkiError>()?;
    let current_active = load_active_ca_fingerprint(&tx, tenant_id).await?;
    let Some(current_active) = current_active else {
        tx.rollback().await.context_to::<PkiError>()?;
        let state = load_managed_ca_state(db, tenant_id).await?;
        return Ok(RotationOutcome {
            rotated: false,
            state,
        });
    };

    if current_active != expected_active_fp {
        tx.rollback().await.context_to::<PkiError>()?;
        let state = load_managed_ca_state(db, tenant_id).await?;
        return Ok(RotationOutcome {
            rotated: false,
            state,
        });
    }

    let now = OffsetDateTime::now_utc();
    let new_ca = generate_ca(pki_addr)?;
    let new_fp = ca_fingerprint(&new_ca.cert_pem)?;
    let not_before = cert_not_before(&new_ca.cert_pem)?;
    let not_after = cert_not_after(&new_ca.cert_pem)?;

    let encrypted_key =
        uptrakit_crypto::EncryptedString::new(new_ca.key_pem.clone()).context_to()?;
    let cert_model = ca_certificate::ActiveModel {
        fingerprint: Set(new_fp.clone()),
        cert_pem: Set(new_ca.cert_pem.clone()),
        key_pem: Set(encrypted_key),
        not_before: Set(not_before),
        not_after: Set(not_after),
        activated_at: Set(now),
        deactivated_at: Set(None),
        created_at: Set(now),
    };
    ca_certificate::Entity::insert(cert_model)
        .exec(&tx)
        .await
        .context_to::<PkiError>()?;

    let update_old = ca_certificate::Entity::update_many()
        .col_expr(ca_certificate::Column::DeactivatedAt, Expr::value(now))
        .filter(ca_certificate::Column::Fingerprint.eq(current_active.clone()))
        .exec(&tx)
        .await
        .context_to::<PkiError>()?;

    if update_old.rows_affected == 0 {
        tx.rollback().await.context_to::<PkiError>()?;
        let state = load_managed_ca_state(db, tenant_id).await?;
        return Ok(RotationOutcome {
            rotated: false,
            state,
        });
    }

    let updated = update_setting_string_cas(
        &tx,
        tenant_id,
        SettingKey::PkiActiveCaFingerprint,
        &current_active,
        &new_fp,
    )
    .await?;

    if !updated {
        tx.rollback().await.context_to::<PkiError>()?;
        let state = load_managed_ca_state(db, tenant_id).await?;
        return Ok(RotationOutcome {
            rotated: false,
            state,
        });
    }

    bump_setting_i64(&tx, tenant_id, SettingKey::PkiCaVersion).await?;

    tx.commit().await.context_to::<PkiError>()?;

    let state = load_managed_ca_state(db, tenant_id).await?;
    Ok(RotationOutcome {
        rotated: true,
        state,
    })
}

fn generate_ca(pki_addr: Option<&str>) -> Result<CaBundle> {
    let key_pair =
        KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).context_to::<PkiError>()?;

    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, "Uptrakit Internal CA");
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Uptrakit");
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.not_before = OffsetDateTime::now_utc();
    params.not_after = OffsetDateTime::now_utc() + time::Duration::days(1825);

    if let Some(url) = pki_addr {
        add_pki_extensions(&mut params, url);
    }

    let cert = params.self_signed(&key_pair).context_to::<PkiError>()?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    let issuer = Issuer::new(params, key_pair);

    Ok(CaBundle {
        cert_pem,
        key_pem,
        issuer,
    })
}

fn load_ca(cert_path: &Path, key_path: &Path) -> Result<CaBundle> {
    let cert_pem = fs::read_to_string(cert_path).context_to::<PkiError>()?;
    let key_pem = fs::read_to_string(key_path).context_to::<PkiError>()?;

    let key_pair = KeyPair::from_pem(&key_pem).context_to::<PkiError>()?;
    let issuer = Issuer::from_ca_cert_pem(&cert_pem, key_pair).context_to::<PkiError>()?;

    tracing::info!("loaded existing CA from {}", cert_path.display());
    Ok(CaBundle {
        cert_pem,
        key_pem,
        issuer,
    })
}

fn bundle_from_model(model: ca_certificate::Model) -> Result<CaBundle> {
    let key_pem_str = model.key_pem.expose_secret().to_string();
    let key_pair = KeyPair::from_pem(&key_pem_str).context_to::<PkiError>()?;
    let issuer = Issuer::from_ca_cert_pem(&model.cert_pem, key_pair).context_to::<PkiError>()?;
    Ok(CaBundle {
        cert_pem: model.cert_pem,
        key_pem: key_pem_str,
        issuer,
    })
}

pub fn bundle_from_pem(cert_pem: String, key_pem: String) -> Result<CaBundle> {
    let key_pair = KeyPair::from_pem(&key_pem).context_to::<PkiError>()?;
    let issuer = Issuer::from_ca_cert_pem(&cert_pem, key_pair).context_to::<PkiError>()?;
    Ok(CaBundle {
        cert_pem,
        key_pem,
        issuer,
    })
}

fn most_recent_deactivated(models: &[ca_certificate::Model]) -> Option<ca_certificate::Model> {
    let mut candidates: Vec<_> = models
        .iter()
        .filter(|m| m.deactivated_at.is_some())
        .cloned()
        .collect();
    candidates.sort_by(|a, b| {
        b.deactivated_at
            .cmp(&a.deactivated_at)
            .then_with(|| b.activated_at.cmp(&a.activated_at))
            .then_with(|| a.fingerprint.cmp(&b.fingerprint))
    });
    candidates.into_iter().next()
}

async fn load_active_ca_fingerprint(
    db: &impl ConnectionTrait,
    tenant_id: uuid::Uuid,
) -> Result<Option<String>> {
    let row = setting::Entity::find_by_id((
        tenant_id,
        SettingKey::PkiActiveCaFingerprint.as_str().to_string(),
    ))
    .one(db)
    .await
    .context_to::<PkiError>()?;

    let value = row.and_then(|r| r.value.as_str().map(String::from));
    Ok(value)
}

async fn insert_setting_string_if_absent(
    db: &impl ConnectionTrait,
    tenant_id: uuid::Uuid,
    key: SettingKey,
    value: &str,
) -> Result<bool> {
    let now = OffsetDateTime::now_utc();
    let model = setting::ActiveModel {
        tenant_id: Set(tenant_id),
        key: Set(key.as_str().to_string()),
        value: Set(serde_json::Value::String(value.to_string())),
        updated_at: Set(now),
    };

    let _ = setting::Entity::insert(model)
        .on_conflict(
            OnConflict::columns([setting::Column::TenantId, setting::Column::Key])
                .do_nothing()
                .to_owned(),
        )
        .exec(db)
        .await
        .context_to::<PkiError>()?;

    let current = load_active_ca_fingerprint(db, tenant_id).await?;
    Ok(current.as_deref() == Some(value))
}

async fn set_setting_i64(
    db: &impl ConnectionTrait,
    tenant_id: uuid::Uuid,
    key: SettingKey,
    value: i64,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let existing = setting::Entity::find_by_id((tenant_id, key.as_str().to_string()))
        .one(db)
        .await
        .context_to::<PkiError>()?;

    if let Some(existing) = existing {
        let mut model: setting::ActiveModel = existing.into();
        model.value = Set(serde_json::Value::from(value));
        model.updated_at = Set(now);
        model.update(db).await.context_to::<PkiError>()?;
    } else {
        let model = setting::ActiveModel {
            tenant_id: Set(tenant_id),
            key: Set(key.as_str().to_string()),
            value: Set(serde_json::Value::from(value)),
            updated_at: Set(now),
        };
        model.insert(db).await.context_to::<PkiError>()?;
    }
    Ok(())
}

async fn update_setting_string_cas(
    db: &impl ConnectionTrait,
    tenant_id: uuid::Uuid,
    key: SettingKey,
    expected: &str,
    new_value: &str,
) -> Result<bool> {
    let now = OffsetDateTime::now_utc();
    let result = setting::Entity::update_many()
        .col_expr(
            setting::Column::Value,
            Expr::value(serde_json::Value::String(new_value.to_string())),
        )
        .col_expr(setting::Column::UpdatedAt, Expr::value(now))
        .filter(setting::Column::TenantId.eq(tenant_id))
        .filter(setting::Column::Key.eq(key.as_str()))
        .filter(setting::Column::Value.eq(serde_json::Value::String(expected.to_string())))
        .exec(db)
        .await
        .context_to::<PkiError>()?;

    Ok(result.rows_affected > 0)
}

async fn bump_setting_i64(
    db: &impl ConnectionTrait,
    tenant_id: uuid::Uuid,
    key: SettingKey,
) -> Result<i64> {
    let current = setting::Entity::find_by_id((tenant_id, key.as_str().to_string()))
        .one(db)
        .await
        .context_to::<PkiError>()?
        .and_then(|row| row.value.as_i64())
        .unwrap_or(0);

    let next = current.saturating_add(1);
    set_setting_i64(db, tenant_id, key, next).await?;
    Ok(next)
}

pub async fn load_ca_version(db: &DatabaseConnection, tenant_id: uuid::Uuid) -> Result<i64> {
    let row =
        setting::Entity::find_by_id((tenant_id, SettingKey::PkiCaVersion.as_str().to_string()))
            .one(db)
            .await
            .context_to::<PkiError>()?;
    Ok(row.and_then(|r| r.value.as_i64()).unwrap_or(0))
}

/// Load a CA from user-provided (external) paths.
pub fn load_external_ca(cert_path: &Path, key_path: &Path) -> Result<CaBundle> {
    let bundle = load_ca(cert_path, key_path)?;
    tracing::info!("using external CA certificate from {}", cert_path.display());
    Ok(bundle)
}

/// Load a server certificate from user-provided paths.
pub fn load_external_cert(cert_path: &Path, key_path: &Path) -> Result<ServerCertBundle> {
    let cert_pem = fs::read_to_string(cert_path).context_to::<PkiError>()?;
    let key_pem = fs::read_to_string(key_path).context_to::<PkiError>()?;
    tracing::info!(
        "using external TLS certificate from {}",
        cert_path.display()
    );
    Ok(ServerCertBundle { cert_pem, key_pem })
}

/// Load or generate a server certificate signed by the internal CA.
pub async fn load_or_generate_server_cert(
    pki: &Path,
    ca: &CaBundle,
    extra_sans: &[String],
) -> Result<ServerCertBundle> {
    let cert_path = pki.join("server.crt");
    let key_path = pki.join("server.key");

    if cert_path.exists() && key_path.exists() {
        let cert_pem = fs::read_to_string(&cert_path).context_to::<PkiError>()?;
        let key_pem = fs::read_to_string(&key_path).context_to::<PkiError>()?;

        if !is_cert_expired(&cert_pem) {
            tracing::info!(
                "loaded existing server certificate from {}",
                cert_path.display()
            );
            return Ok(ServerCertBundle { cert_pem, key_pem });
        }
        tracing::warn!("server certificate expired, regenerating");
    }

    let bundle = generate_server_cert(ca, extra_sans)?;
    uptrakit_directories::write_secure_file_str(&cert_path, &bundle.cert_pem)
        .await
        .context_to::<PkiError>()?;
    uptrakit_directories::write_secure_file_str(&key_path, &bundle.key_pem)
        .await
        .context_to::<PkiError>()?;
    tracing::info!("generated new server certificate at {}", pki.display());
    Ok(bundle)
}

fn generate_server_cert(ca: &CaBundle, extra_sans: &[String]) -> Result<ServerCertBundle> {
    let key_pair =
        KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).context_to::<PkiError>()?;

    let sans = pki_utils::collect_sans(extra_sans).context_to::<PkiError>()?;

    let mut params = CertificateParams::new(sans.dns_names.clone()).context_to::<PkiError>()?;
    for ip in &sans.ip_addrs {
        params
            .subject_alt_names
            .push(rcgen::SanType::IpAddress(*ip));
    }
    params
        .distinguished_name
        .push(DnType::CommonName, "Uptrakit Controller");
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Uptrakit");
    params.not_before = OffsetDateTime::now_utc();
    params.not_after = OffsetDateTime::now_utc()
        + time::Duration::days(crate::durations::SERVER_CERT_VALIDITY_DAYS);

    let cert = params
        .signed_by(&key_pair, &ca.issuer)
        .context_to::<PkiError>()?;

    Ok(ServerCertBundle {
        cert_pem: cert.pem(),
        key_pem: key_pair.serialize_pem(),
    })
}

// --- CA fingerprint ---

/// Compute SHA-256 hex fingerprint of a PEM-encoded certificate.
pub fn ca_fingerprint(cert_pem: &str) -> Result<String> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|_| report!(PkiError::PemParse))?;
    Ok(sha256_hex(&pem_block.contents))
}

/// SHA-256 hex digest of arbitrary bytes.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    uptrakit_shared_types::hex::encode(hasher.finalize())
}

// --- Cert introspection ---

/// Check if a PEM-encoded certificate is expired.
/// Returns `true` if the certificate is expired or unparseable.
pub fn is_cert_expired(pem: &str) -> bool {
    let Ok((_, pem_block)) = x509_parser::pem::parse_x509_pem(pem.as_bytes()) else {
        return true;
    };
    let Ok(cert) = pem_block.parse_x509() else {
        return true;
    };
    !cert.validity().is_valid()
}

/// Extract the `not_after` timestamp from a PEM-encoded certificate.
pub fn cert_not_after(pem: &str) -> Result<OffsetDateTime> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|_| report!(PkiError::PemParse))?;
    let cert = pem_block
        .parse_x509()
        .map_err(|_| report!(PkiError::PemParse))?;
    OffsetDateTime::from_unix_timestamp(cert.validity().not_after.timestamp())
        .map_err(|e| report!(PkiError::Timestamp(e.to_string())))
}

/// Extract the `not_before` timestamp from a PEM-encoded certificate.
pub fn cert_not_before(pem: &str) -> Result<OffsetDateTime> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|_| report!(PkiError::PemParse))?;
    let cert = pem_block
        .parse_x509()
        .map_err(|_| report!(PkiError::PemParse))?;
    OffsetDateTime::from_unix_timestamp(cert.validity().not_before.timestamp())
        .map_err(|e| report!(PkiError::Timestamp(e.to_string())))
}

fn cert_common_name(pem: &str) -> Option<String> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(pem.as_bytes()).ok()?;
    let cert = pem_block.parse_x509().ok()?;
    let cn = cert.subject().iter_common_name().next()?.as_str().ok()?;
    Some(cn.to_string())
}

// --- Rotation helpers ---

/// Returns `true` if the CA certificate expires within 183 days (6 months).
pub fn should_rotate_ca(cert_pem: &str) -> bool {
    let Ok(not_after) = cert_not_after(cert_pem) else {
        return true;
    };
    let threshold =
        OffsetDateTime::now_utc() + time::Duration::days(crate::durations::CA_ROTATION_WINDOW_DAYS);
    not_after <= threshold
}

// --- Server cert renewal ---

/// Returns `true` if the server certificate expires within 30 days.
pub fn should_renew_server_cert(cert_pem: &str) -> bool {
    let Ok(not_after) = cert_not_after(cert_pem) else {
        return true;
    };
    let threshold = OffsetDateTime::now_utc()
        + time::Duration::days(crate::durations::SERVER_CERT_RENEWAL_WINDOW_DAYS);
    not_after <= threshold
}

/// Generate a new server cert signed by the given CA and save to the PKI directory.
pub async fn renew_server_cert(
    pki: &Path,
    ca: &CaBundle,
    extra_sans: &[String],
) -> Result<ServerCertBundle> {
    let bundle = generate_server_cert(ca, extra_sans)?;
    let cert_path = pki.join("server.crt");
    let key_path = pki.join("server.key");
    uptrakit_directories::write_secure_file_str(&cert_path, &bundle.cert_pem)
        .await
        .context_to::<PkiError>()?;
    uptrakit_directories::write_secure_file_str(&key_path, &bundle.key_pem)
        .await
        .context_to::<PkiError>()?;
    tracing::info!("server certificate renewed at {}", pki.display());
    Ok(bundle)
}

// --- SAN sanity checks ---

/// Extract Subject Alternative Names from a PEM-encoded certificate.
///
/// Returns a `SanCollection` with the DNS names and IP addresses found in
/// the certificate's SAN extension.
pub fn extract_sans_from_cert(cert_pem: &str) -> Result<SanCollection> {
    use x509_parser::extensions::GeneralName;

    let (_, pem_block) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|_| report!(PkiError::PemParse))?;
    let cert = pem_block
        .parse_x509()
        .map_err(|_| report!(PkiError::PemParse))?;

    let mut dns_names = Vec::new();
    let mut ip_addrs = Vec::new();

    if let Ok(Some(san_ext)) = cert.tbs_certificate.subject_alternative_name() {
        for name in &san_ext.value.general_names {
            match name {
                GeneralName::DNSName(dns) => {
                    dns_names.push((*dns).to_string());
                }
                GeneralName::IPAddress(ip_bytes) => {
                    match ip_bytes.len() {
                        4 => {
                            if let Ok(octets) = <[u8; 4]>::try_from(*ip_bytes) {
                                ip_addrs.push(IpAddr::V4(std::net::Ipv4Addr::from(octets)));
                            }
                        }
                        16 => {
                            if let Ok(octets) = <[u8; 16]>::try_from(*ip_bytes) {
                                ip_addrs.push(IpAddr::V6(std::net::Ipv6Addr::from(octets)));
                            }
                        }
                        _ => {} // skip malformed IP entries
                    }
                }
                _ => {}
            }
        }
    }

    dns_names.sort();
    dns_names.dedup();
    ip_addrs.sort();
    ip_addrs.dedup();

    Ok(SanCollection {
        dns_names,
        ip_addrs,
    })
}

/// Check whether the server certificate needs to be regenerated because its
/// SANs do not include all the requested `extra_sans`.
///
/// Returns `false` if `extra_sans` is empty (no user-requested SANs to check).
/// Otherwise computes the expected SAN set via `collect_sans(extra_sans)` and
/// compares against the certificate's actual SANs.
pub fn server_cert_needs_san_update(cert_pem: &str, extra_sans: &[String]) -> Result<bool> {
    if extra_sans.is_empty() {
        return Ok(false);
    }

    let expected = pki_utils::collect_sans(extra_sans).context_to::<PkiError>()?;
    let actual = extract_sans_from_cert(cert_pem)?;

    let mut expected_dns = expected.dns_names;
    expected_dns.sort();
    let mut actual_dns = actual.dns_names;
    actual_dns.sort();

    let mut expected_ips = expected.ip_addrs;
    expected_ips.sort();
    let mut actual_ips = actual.ip_addrs;
    actual_ips.sort();

    // Check that every expected SAN is present in actual
    let dns_match = expected_dns.iter().all(|name| actual_dns.contains(name));
    let ip_match = expected_ips.iter().all(|ip| actual_ips.contains(ip));

    Ok(!dns_match || !ip_match)
}

/// Check if a certificate was signed by the given CA.
///
/// Thin wrapper around `pki_utils::cert_signed_by_ca` that converts errors
/// to `PkiError`.
pub fn cert_signed_by_ca(cert_pem: &str, ca_pem: &str) -> Result<bool> {
    pki_utils::cert_signed_by_ca(cert_pem, ca_pem).context_to::<PkiError>()
}

// --- PKI URL extraction and validation ---

/// URLs extracted from a certificate's AIA and CDP extensions.
#[derive(Debug, Default, PartialEq)]
pub struct CertPkiUrls {
    pub ocsp_url: Option<String>,
    pub ca_issuers_url: Option<String>,
    pub crl_url: Option<String>,
}

impl CertPkiUrls {
    /// Returns `true` if the certificate has any AIA or CDP extensions.
    pub fn has_extensions(&self) -> bool {
        self.ocsp_url.is_some() || self.ca_issuers_url.is_some() || self.crl_url.is_some()
    }
}

/// Extract AIA and CDP URLs from a PEM-encoded certificate.
pub fn extract_cert_pki_urls(cert_pem: &str) -> Result<CertPkiUrls> {
    use x509_parser::extensions::ParsedExtension;

    let (_, pem_block) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|_| report!(PkiError::PemParse))?;
    let cert = pem_block
        .parse_x509()
        .map_err(|_| report!(PkiError::PemParse))?;

    let mut urls = CertPkiUrls::default();

    for ext in cert.extensions() {
        match ext.parsed_extension() {
            ParsedExtension::AuthorityInfoAccess(aia) => {
                for desc in &aia.accessdescs {
                    // id-ad-ocsp = 1.3.6.1.5.5.7.48.1
                    if desc.access_method.to_id_string() == "1.3.6.1.5.5.7.48.1"
                        && let x509_parser::extensions::GeneralName::URI(uri) = desc.access_location
                    {
                        urls.ocsp_url = Some(uri.to_string());
                    }
                    // id-ad-caIssuers = 1.3.6.1.5.5.7.48.2
                    if desc.access_method.to_id_string() == "1.3.6.1.5.5.7.48.2"
                        && let x509_parser::extensions::GeneralName::URI(uri) = desc.access_location
                    {
                        urls.ca_issuers_url = Some(uri.to_string());
                    }
                }
            }
            ParsedExtension::CRLDistributionPoints(cdp) => {
                for point in cdp.iter() {
                    if let Some(name) = &point.distribution_point
                        && let x509_parser::extensions::DistributionPointName::FullName(names) =
                            name
                    {
                        for general_name in names {
                            if let x509_parser::extensions::GeneralName::URI(uri) = general_name {
                                urls.crl_url = Some(uri.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(urls)
}

/// Validate that the existing CA certificate's AIA/CDP extensions match the
/// reconciled `pki_addr`. Only applies to managed CAs.
///
/// Call this after loading CA state and before building the snapshot.
/// Mismatch causes a hard startup failure with descriptive error.
pub fn validate_ca_pki_addr(cert_pem: &str, pki_addr: Option<&str>) -> Result<()> {
    let cert_urls = extract_cert_pki_urls(cert_pem)?;
    let has_extensions = cert_urls.has_extensions();

    match (pki_addr, has_extensions) {
        // pki_addr set, CA has extensions — check they match
        (Some(url), true) => {
            let expected_ocsp = format!("{url}/api/v1/pki/ocsp");
            let expected_ca_issuers = format!("{url}/api/v1/pki/ca.crt");
            let expected_crl = format!("{url}/api/v1/pki/ca.crl");

            let mismatch = cert_urls.ocsp_url.as_deref() != Some(&expected_ocsp)
                || cert_urls.ca_issuers_url.as_deref() != Some(&expected_ca_issuers)
                || cert_urls.crl_url.as_deref() != Some(&expected_crl);

            if mismatch {
                bail!(PkiError::CaValidation(format!(
                    "The CA certificate's AIA/CDP URLs do not match --pki-addr ({url}).\n\
                     \n\
                     CA certificate contains:\n\
                     \x20 OCSP:       {}\n\
                     \x20 CA Issuers: {}\n\
                     \x20 CRL:        {}\n\
                     \n\
                     Expected (from --pki-addr):\n\
                     \x20 OCSP:       {expected_ocsp}\n\
                     \x20 CA Issuers: {expected_ca_issuers}\n\
                     \x20 CRL:        {expected_crl}\n\
                     \n\
                     To fix this, either:\n\
                     \x20 1. Update --pki-addr to match the CA certificate's URLs, or\n\
                     \x20 2. Rotate the CA to regenerate it with the new URL",
                    cert_urls.ocsp_url.as_deref().unwrap_or("<none>"),
                    cert_urls.ca_issuers_url.as_deref().unwrap_or("<none>"),
                    cert_urls.crl_url.as_deref().unwrap_or("<none>"),
                )));
            }
            Ok(())
        }
        // pki_addr set, CA has no extensions — need to regenerate
        (Some(url), false) => Err(report!(PkiError::CaValidation(format!(
            "The CA certificate has no AIA/CDP extensions, but --pki-addr ({url}) is set.\n\
                 \n\
                 The CA needs to be regenerated with the backend URL to embed OCSP, CA Issuers,\n\
                 and CRL Distribution Point URLs in certificates.\n\
                 \n\
                 To fix this, rotate the CA so it is regenerated with the correct extensions.",
        )))),
        // pki_addr not set, CA has extensions — unexpected
        (None, true) => Err(report!(PkiError::CaValidation(format!(
            "The CA certificate contains AIA/CDP extensions but no --pki-addr is configured.\n\
                 \n\
                 CA certificate contains:\n\
                 \x20 OCSP:       {}\n\
                 \x20 CA Issuers: {}\n\
                 \x20 CRL:        {}\n\
                 \n\
                 To fix this, either:\n\
                 \x20 1. Provide --pki-addr matching the URLs in the CA certificate, or\n\
                 \x20 2. Rotate the CA to regenerate without extensions",
            cert_urls.ocsp_url.as_deref().unwrap_or("<none>"),
            cert_urls.ca_issuers_url.as_deref().unwrap_or("<none>"),
            cert_urls.crl_url.as_deref().unwrap_or("<none>"),
        )))),
        // pki_addr not set, CA has no extensions — OK
        (None, false) => Ok(()),
    }
}

// --- TLS config builders ---

/// Build a `rustls::ServerConfig` from PEM-encoded cert and key (no client auth).
#[cfg(test)]
pub fn build_rustls_config(cert_pem: &str, key_pem: &str) -> Result<rustls::ServerConfig> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    let certs: Vec<_> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_transform(|_| PkiError::PemParse)?;

    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
        .context_transform(|_| PkiError::PemParse)?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context_to::<PkiError>()?;

    Ok(config)
}

/// Build a `rustls::ServerConfig` with mTLS client authentication and multiple CRLs.
///
/// Each CA in the bundle gets its own CRL. The verifier checks client certificates
/// against all supplied CRLs.
pub fn build_rustls_config_with_client_auth_and_crls(
    cert_pem: &str,
    key_pem: &str,
    ca_bundle_pem: &str,
    crls: Vec<rustls::pki_types::CertificateRevocationListDer<'static>>,
) -> Result<rustls::ServerConfig> {
    use rustls::RootCertStore;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls::server::WebPkiClientVerifier;

    let certs: Vec<_> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_transform(|_| PkiError::PemParse)?;

    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
        .context_transform(|_| PkiError::PemParse)?;

    let ca_certs: Vec<_> = CertificateDer::pem_slice_iter(ca_bundle_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_transform(|_| PkiError::PemParse)?;

    let mut root_store = RootCertStore::empty();
    for ca_cert in ca_certs {
        root_store.add(ca_cert).context_to::<PkiError>()?;
    }

    // allow_unauthenticated() is intentional.
    //
    // The controller exposes a single mTLS listener for both enrolled agents
    // (which present a client certificate) and agents that have not yet
    // enrolled (which have no certificate).  At the TLS layer the handshake
    // succeeds for both.  The `MtlsAcceptor` then extracts the peer
    // certificate CN into an `Option<ServiceIdentity>`:
    //
    //   - `Some(ServiceIdentity)` → established agent, authenticated by cert.
    //   - `None`                  → unenrolled agent, authenticated later via
    //                              the enrollment secret bearer token.
    //
    // Removing allow_unauthenticated() would break the enrollment flow
    // entirely, because the agent cannot present a certificate it has not yet
    // received.  Application-level handlers guard their endpoints against
    // `None` identities as appropriate.
    let verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
        .with_crls(crls)
        .allow_unauthenticated()
        .only_check_end_entity_revocation()
        .build()
        .map_err(|e| report!(PkiError::VerifierBuilder(e.to_string())))?;

    let config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .context_to::<PkiError>()?;

    Ok(config)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn ca_generation_produces_valid_material() {
        let ca = generate_ca(None).unwrap();
        assert!(ca.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(ca.key_pem.contains("BEGIN"));
    }

    #[test]
    fn server_cert_signed_by_ca() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let ca = generate_ca(None).unwrap();
        let bundle = generate_server_cert(&ca, &[]).unwrap();
        assert!(bundle.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(bundle.key_pem.contains("BEGIN"));

        // Should build a valid rustls config
        build_rustls_config(&bundle.cert_pem, &bundle.key_pem).unwrap();
    }

    #[test]
    fn server_cert_includes_localhost() {
        let sans = pki_utils::collect_sans(&[]).unwrap();
        assert!(sans.dns_names.contains(&"localhost".to_string()));
    }

    #[test]
    fn server_cert_includes_extra_sans() {
        let extras = vec![
            "myhost.example.com".to_string(),
            "192.168.1.1".to_string(),
            "::1".to_string(),
        ];
        let sans = pki_utils::collect_sans(&extras).unwrap();
        assert!(sans.dns_names.contains(&"myhost.example.com".to_string()));
        assert!(
            sans.ip_addrs
                .contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))
        );
        assert!(sans.ip_addrs.contains(&IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn hostname_deduplication() {
        let hostname = hostname::get().unwrap().to_string_lossy().to_string();
        let extras = vec![hostname.clone()];
        let sans = pki_utils::collect_sans(&extras).unwrap();
        let count = sans.dns_names.iter().filter(|n| **n == hostname).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn expired_cert_detection() {
        let ca = generate_ca(None).unwrap();
        assert!(!is_cert_expired(&ca.cert_pem));
    }

    #[test]
    fn malformed_pem_is_expired() {
        assert!(is_cert_expired("not a cert"));
    }

    #[tokio::test]
    async fn server_cert_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let pki = dir.path();

        let ca = generate_ca(None).unwrap();
        let b1 = load_or_generate_server_cert(pki, &ca, &[]).await.unwrap();
        let b2 = load_or_generate_server_cert(pki, &ca, &[]).await.unwrap();

        assert_eq!(b1.cert_pem, b2.cert_pem);
    }

    #[test]
    fn san_ipv6_address() {
        let extras = vec!["fd00::1".to_string()];
        let sans = pki_utils::collect_sans(&extras).unwrap();
        assert!(
            sans.ip_addrs
                .contains(&IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1)))
        );
    }

    #[test]
    fn ca_fingerprint_deterministic() {
        let ca = generate_ca(None).unwrap();
        let fp1 = ca_fingerprint(&ca.cert_pem).unwrap();
        let fp2 = ca_fingerprint(&ca.cert_pem).unwrap();
        assert_eq!(fp1, fp2);
        assert_eq!(fp1.len(), 64); // SHA-256 hex is 64 chars
    }

    #[test]
    fn ca_fingerprint_differs_between_cas() {
        let ca1 = generate_ca(None).unwrap();
        let ca2 = generate_ca(None).unwrap();
        let fp1 = ca_fingerprint(&ca1.cert_pem).unwrap();
        let fp2 = ca_fingerprint(&ca2.cert_pem).unwrap();
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn should_rotate_ca_not_yet() {
        let ca = generate_ca(None).unwrap();
        // Fresh CA with 5 year validity should not need rotation
        assert!(!should_rotate_ca(&ca.cert_pem));
    }

    #[test]
    fn should_renew_server_cert_not_yet() {
        let ca = generate_ca(None).unwrap();
        let server = generate_server_cert(&ca, &[]).unwrap();
        // Fresh server cert with 90 day validity should not need renewal
        assert!(!should_renew_server_cert(&server.cert_pem));
    }

    #[test]
    fn ca_state_bundle_pem() {
        let ca1 = generate_ca(None).unwrap();
        let ca2 = generate_ca(None).unwrap();
        let state = CaState {
            active: bundle_from_pem(ca1.cert_pem.clone(), ca1.key_pem.clone()).unwrap(),
            previous: Some(bundle_from_pem(ca2.cert_pem.clone(), ca2.key_pem.clone()).unwrap()),
            trusted: vec![ca1, ca2],
            managed: true,
        };
        let bundle = state.ca_bundle_pem();
        // Bundle should contain two certificates
        assert_eq!(bundle.matches("BEGIN CERTIFICATE").count(), 2);
    }

    #[test]
    fn ca_snapshot_roundtrip() {
        let ca = generate_ca(None).unwrap();
        let state = CaState {
            active: bundle_from_pem(ca.cert_pem.clone(), ca.key_pem.clone()).unwrap(),
            previous: None,
            trusted: vec![ca],
            managed: true,
        };
        let (snapshot, key_store) = state.to_snapshot(None).unwrap();
        assert!(!snapshot.active_fingerprint.is_empty());
        assert!(snapshot.previous_fingerprint.is_none());
        assert!(!snapshot.bundle_hash.is_empty());
        assert!(!key_store.active_key_pem.is_empty());
        assert!(key_store.previous_key_pem.is_none());
        assert_eq!(key_store.trusted_ca_keys.len(), 1);
    }

    #[test]
    fn extract_sans_dns_only() {
        let ca = generate_ca(None).unwrap();
        let server = generate_server_cert(&ca, &[]).unwrap();
        let sans = extract_sans_from_cert(&server.cert_pem).unwrap();
        assert!(sans.dns_names.contains(&"localhost".to_string()));
        assert!(sans.ip_addrs.is_empty());
    }

    #[test]
    fn extract_sans_dns_and_ip() {
        let ca = generate_ca(None).unwrap();
        let extras = vec!["192.168.1.1".to_string(), "myhost.example.com".to_string()];
        let server = generate_server_cert(&ca, &extras).unwrap();
        let sans = extract_sans_from_cert(&server.cert_pem).unwrap();
        assert!(sans.dns_names.contains(&"localhost".to_string()));
        assert!(sans.dns_names.contains(&"myhost.example.com".to_string()));
        assert!(
            sans.ip_addrs
                .contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))
        );
    }

    #[test]
    fn server_cert_needs_san_update_empty_extra() {
        let ca = generate_ca(None).unwrap();
        let server = generate_server_cert(&ca, &[]).unwrap();
        // Empty extra_sans always returns false
        assert!(!server_cert_needs_san_update(&server.cert_pem, &[]).unwrap());
    }

    #[test]
    fn server_cert_needs_san_update_matching() {
        let ca = generate_ca(None).unwrap();
        let extras = vec!["myhost.example.com".to_string()];
        let server = generate_server_cert(&ca, &extras).unwrap();
        // Cert already includes the requested SAN
        assert!(!server_cert_needs_san_update(&server.cert_pem, &extras).unwrap());
    }

    #[test]
    fn server_cert_needs_san_update_mismatched() {
        let ca = generate_ca(None).unwrap();
        let server = generate_server_cert(&ca, &[]).unwrap();
        let extras = vec!["new-host.example.com".to_string()];
        // Cert does not include the requested SAN
        assert!(server_cert_needs_san_update(&server.cert_pem, &extras).unwrap());
    }

    #[test]
    fn server_cert_needs_san_update_ip_missing() {
        let ca = generate_ca(None).unwrap();
        let server = generate_server_cert(&ca, &[]).unwrap();
        let extras = vec!["10.0.0.1".to_string()];
        // Cert does not include the requested IP SAN
        assert!(server_cert_needs_san_update(&server.cert_pem, &extras).unwrap());
    }

    #[test]
    fn cert_signed_by_ca_same() {
        let ca = generate_ca(None).unwrap();
        let server = generate_server_cert(&ca, &[]).unwrap();
        assert!(cert_signed_by_ca(&server.cert_pem, &ca.cert_pem).unwrap());
    }

    #[test]
    fn cert_signed_by_ca_different() {
        // Use CAs with different DNs so issuer check can distinguish them
        let key1 = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params1 = CertificateParams::default();
        params1
            .distinguished_name
            .push(DnType::CommonName, "Test CA 1");
        params1.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        let _ca1_cert = params1.self_signed(&key1).unwrap();
        let issuer1 = Issuer::new(params1, key1);

        let server_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let server_params = CertificateParams::new(vec!["localhost".into()]).unwrap();
        let server_cert = server_params.signed_by(&server_key, &issuer1).unwrap();
        let server_pem = server_cert.pem();

        let key2 = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params2 = CertificateParams::default();
        params2
            .distinguished_name
            .push(DnType::CommonName, "Test CA 2");
        params2.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        let ca2_cert = params2.self_signed(&key2).unwrap();
        let ca2_pem = ca2_cert.pem();

        assert!(!cert_signed_by_ca(&server_pem, &ca2_pem).unwrap());
    }

    #[tokio::test]
    async fn managed_ca_init_and_rotation() -> std::result::Result<(), String> {
        use sea_orm::{ColumnTrait, ConnectOptions, Database, EntityTrait, QueryFilter};
        use uptrakit_shared_db::entity::tenant;

        // EncryptedString requires a master key for DB writes
        let _ = uptrakit_crypto::init_master_key(zeroize::Zeroizing::new([0x42u8; 32]));

        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.map_err(|e| e.to_string())?;
        crate::migration::run_migrations(&db)
            .await
            .map_err(|e| format!("{e:?}"))?;

        let tenant = tenant::Entity::find()
            .filter(tenant::Column::IsDefault.eq(true))
            .one(&db)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "default tenant not found".to_string())?;

        let state = load_or_init_managed_ca(&db, tenant.id, None)
            .await
            .map_err(|e| format!("{e:?}"))?;
        let active_fp = ca_fingerprint(&state.active.cert_pem).map_err(|e| format!("{e:?}"))?;

        let version = load_ca_version(&db, tenant.id)
            .await
            .map_err(|e| format!("{e:?}"))?;
        if version != 1 {
            return Err(format!("expected CA version 1, got {version}"));
        }

        let rotation = rotate_managed_ca(&db, tenant.id, None, &active_fp)
            .await
            .map_err(|e| format!("{e:?}"))?;
        if !rotation.rotated {
            return Err("rotation should succeed".to_string());
        }

        let new_fp =
            ca_fingerprint(&rotation.state.active.cert_pem).map_err(|e| format!("{e:?}"))?;
        if new_fp == active_fp {
            return Err("rotation did not update active CA".to_string());
        }

        let version = load_ca_version(&db, tenant.id)
            .await
            .map_err(|e| format!("{e:?}"))?;
        if version != 2 {
            return Err(format!("expected CA version 2, got {version}"));
        }

        Ok(())
    }

    // --- AIA/CDP extension tests ---

    #[test]
    fn ca_with_pki_addr_has_aia_cdp() {
        let ca = generate_ca(Some("https://controller.example.com")).unwrap();
        let urls = extract_cert_pki_urls(&ca.cert_pem).unwrap();
        assert!(urls.has_extensions());
        assert_eq!(
            urls.ocsp_url.as_deref(),
            Some("https://controller.example.com/api/v1/pki/ocsp")
        );
        assert_eq!(
            urls.ca_issuers_url.as_deref(),
            Some("https://controller.example.com/api/v1/pki/ca.crt")
        );
        assert_eq!(
            urls.crl_url.as_deref(),
            Some("https://controller.example.com/api/v1/pki/ca.crl")
        );
    }

    #[test]
    fn ca_without_pki_addr_has_no_aia_cdp() {
        let ca = generate_ca(None).unwrap();
        let urls = extract_cert_pki_urls(&ca.cert_pem).unwrap();
        assert!(!urls.has_extensions());
        assert!(urls.ocsp_url.is_none());
        assert!(urls.ca_issuers_url.is_none());
        assert!(urls.crl_url.is_none());
    }

    #[test]
    fn extract_pki_urls_roundtrip() {
        let pki_addr = "https://my-controller:8443";
        let ca = generate_ca(Some(pki_addr)).unwrap();
        let urls = extract_cert_pki_urls(&ca.cert_pem).unwrap();
        assert_eq!(
            urls,
            CertPkiUrls {
                ocsp_url: Some(format!("{pki_addr}/api/v1/pki/ocsp")),
                ca_issuers_url: Some(format!("{pki_addr}/api/v1/pki/ca.crt")),
                crl_url: Some(format!("{pki_addr}/api/v1/pki/ca.crl")),
            }
        );
    }

    #[test]
    fn validate_ca_pki_addr_matching() {
        let url = "https://controller.example.com";
        let ca = generate_ca(Some(url)).unwrap();
        assert!(validate_ca_pki_addr(&ca.cert_pem, Some(url)).is_ok());
    }

    #[test]
    fn validate_ca_pki_addr_mismatched() {
        let ca = generate_ca(Some("https://old-url.example.com")).unwrap();
        let result = validate_ca_pki_addr(&ca.cert_pem, Some("https://new-url.example.com"));
        assert!(result.is_err());
        let err = format!("{:?}", result.unwrap_err());
        assert!(err.contains("do not match"));
    }

    #[test]
    fn validate_ca_pki_addr_set_but_no_extensions() {
        let ca = generate_ca(None).unwrap();
        let result = validate_ca_pki_addr(&ca.cert_pem, Some("https://example.com"));
        assert!(result.is_err());
        let err = format!("{:?}", result.unwrap_err());
        assert!(err.contains("no AIA/CDP extensions"));
    }

    #[test]
    fn validate_ca_pki_addr_not_set_but_has_extensions() {
        let ca = generate_ca(Some("https://example.com")).unwrap();
        let result = validate_ca_pki_addr(&ca.cert_pem, None);
        assert!(result.is_err());
        let err = format!("{:?}", result.unwrap_err());
        assert!(err.contains("no --pki-addr is configured"));
    }

    #[test]
    fn validate_ca_pki_addr_neither_set() {
        let ca = generate_ca(None).unwrap();
        assert!(validate_ca_pki_addr(&ca.cert_pem, None).is_ok());
    }

    #[test]
    fn ca_basic_constraints_path_len_is_zero() {
        use x509_parser::extensions::ParsedExtension;

        let ca = generate_ca(None).unwrap();
        let (_, pem_block) =
            x509_parser::pem::parse_x509_pem(ca.cert_pem.as_bytes()).expect("parse PEM");
        let cert = pem_block.parse_x509().expect("parse X.509");

        let mut found = false;
        for ext in cert.extensions() {
            if let ParsedExtension::BasicConstraints(bc) = ext.parsed_extension() {
                assert!(bc.ca, "CA flag must be set");
                assert_eq!(
                    bc.path_len_constraint,
                    Some(0),
                    "path_len must be 0 to prevent subordinate CA issuance"
                );
                found = true;
            }
        }
        assert!(found, "BasicConstraints extension not present");
    }

    #[test]
    fn build_aia_der_produces_valid_extension() {
        // Generate a CA with extensions and verify parsing
        let ca = generate_ca(Some("https://test.example.com")).unwrap();
        let urls = extract_cert_pki_urls(&ca.cert_pem).unwrap();

        // Verify all three URLs are present and correctly formatted
        assert_eq!(
            urls.ocsp_url.unwrap(),
            "https://test.example.com/api/v1/pki/ocsp"
        );
        assert_eq!(
            urls.ca_issuers_url.unwrap(),
            "https://test.example.com/api/v1/pki/ca.crt"
        );
        assert_eq!(
            urls.crl_url.unwrap(),
            "https://test.example.com/api/v1/pki/ca.crl"
        );
    }
}
