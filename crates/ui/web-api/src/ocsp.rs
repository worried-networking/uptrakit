use der::asn1::{BitString, OctetString};
use der::{Decode, Encode};
use rootcause::prelude::*;
use sea_orm::{ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_shared_macros::impl_report_conversion;
use x509_cert::ext::pkix::CrlReason;
use x509_ocsp::{
    BasicOcspResponse, CertId, CertStatus, OcspGeneralizedTime, OcspRequest, OcspResponse,
    OcspResponseStatus, ResponderId, ResponseData, SingleResponse, Version,
};

use crate::ca_snapshot::CaSnapshotData;

/// Errors that can occur during OCSP response construction.
#[derive(Debug, Error)]
enum OcspError {
    #[error("PEM parse error")]
    PemParse,

    #[error("X.509 parse error")]
    X509Parse,

    #[error("failed to compute key hash")]
    KeyHash,

    #[error("ASN.1 construction error: {0}")]
    Construction(String),

    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("DER encoding error: {0}")]
    DerEncode(String),

    #[error("key parse error: {0}")]
    KeyParse(String),

    #[error("signing error: {0}")]
    Signing(String),

    #[error("base64 decode error: {0}")]
    Base64Decode(String),

    #[error("empty PEM data")]
    EmptyPemData,
}

type OcspResult<T> = std::result::Result<T, Report<OcspError>>;

impl_report_conversion!(sea_orm::DbErr => OcspError::Database);

/// SHA-1 OID (`1.3.14.3.2.26`) — used by Nginx/OpenSSL in OCSP requests.
const SHA1_OID: const_oid::ObjectIdentifier =
    const_oid::ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
/// SHA-256 OID (`2.16.840.1.101.3.4.2.1`).
const SHA256_OID: const_oid::ObjectIdentifier =
    const_oid::ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");

/// Build an OCSP response for the given DER-encoded OCSP request.
///
/// Queries the `agent_certificates` table to determine certificate status.
/// Supports both SHA-1 and SHA-256 hash algorithms in requests per RFC 6960.
pub async fn build_ocsp_response(
    request_der: &[u8],
    ca_snapshot: &CaSnapshotData,
    db: &DatabaseConnection,
) -> Vec<u8> {
    // Parse the request
    let ocsp_request = match OcspRequest::from_der(request_der) {
        Ok(req) => req,
        Err(_) => return build_error_response(OcspResponseStatus::MalformedRequest),
    };

    // Extract the raw CA public key bytes (used to compute hashes on demand)
    let active_pub_key = match extract_ca_public_key_bytes(&ca_snapshot.active_cert_pem) {
        Ok(v) => v,
        Err(_) => return build_error_response(OcspResponseStatus::InternalError),
    };

    let prev_pub_key = ca_snapshot
        .previous_cert_pem
        .as_deref()
        .and_then(|pem| extract_ca_public_key_bytes(pem).ok());

    // Build responder ID using SHA-1 per RFC 6960 Section 2.3
    let responder_id = match build_responder_id(&active_pub_key) {
        Ok(v) => v,
        Err(_) => return build_error_response(OcspResponseStatus::InternalError),
    };

    // Process each request in the list
    let mut single_responses = Vec::new();

    for request in &ocsp_request.tbs_request.request_list {
        let cert_id = &request.req_cert;
        let hash_oid = &cert_id.hash_algorithm.oid;

        // Compute the CA key hash using the same algorithm the client used
        let active_hash = match compute_key_hash(&active_pub_key, hash_oid) {
            Some(h) => h,
            None => {
                // Unsupported hash algorithm — return unknown
                single_responses.push(build_single_response(
                    cert_id.clone(),
                    CertStatus::unknown(),
                ));
                continue;
            }
        };

        let prev_hash = prev_pub_key
            .as_ref()
            .and_then(|pk| compute_key_hash(pk, hash_oid));

        // Validate that the issuer key hash matches our CA (active or previous)
        let matches_active = cert_id.issuer_key_hash.as_bytes() == active_hash.as_slice();
        let matches_previous = prev_hash
            .as_ref()
            .is_some_and(|h| cert_id.issuer_key_hash.as_bytes() == h.as_slice());

        if !matches_active && !matches_previous {
            single_responses.push(build_single_response(
                cert_id.clone(),
                CertStatus::unknown(),
            ));
            continue;
        }

        // Extract serial number for DB lookup (hex-encoded)
        let serial_hex = format_serial_hex(cert_id.serial_number.as_bytes());

        // Query certificate status from DB
        let status = match lookup_cert_status(db, &serial_hex, ca_snapshot).await {
            Ok(s) => s,
            Err(_) => return build_error_response(OcspResponseStatus::InternalError),
        };

        single_responses.push(build_single_response(cert_id.clone(), status));
    }

    // Build the response data
    let now = make_ocsp_time(OffsetDateTime::now_utc());

    let response_data = ResponseData {
        version: Version::V1,
        responder_id,
        produced_at: now,
        responses: single_responses,
        response_extensions: None,
    };

    // Sign the response
    match sign_response(&response_data, ca_snapshot) {
        Ok(response_bytes) => response_bytes,
        Err(_) => build_error_response(OcspResponseStatus::InternalError),
    }
}

/// Format a serial number byte slice as a lowercase hex string.
fn format_serial_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
}

/// Build a DER-encoded error OCSP response (no response body).
fn build_error_response(status: OcspResponseStatus) -> Vec<u8> {
    let response = match status {
        OcspResponseStatus::MalformedRequest => OcspResponse::malformed_request(),
        OcspResponseStatus::InternalError => OcspResponse::internal_error(),
        _ => OcspResponse::internal_error(),
    };
    response.to_der().unwrap_or_default()
}

/// Build the responder ID from the CA public key bytes.
///
/// Per RFC 6960 Section 2.3, `ResponderID::ByKey` MUST use SHA-1.
fn build_responder_id(pub_key_bytes: &[u8]) -> OcspResult<ResponderId> {
    let key_hash =
        compute_key_hash(pub_key_bytes, &SHA1_OID).ok_or_else(|| report!(OcspError::KeyHash))?;
    let responder_id = ResponderId::ByKey(
        OctetString::new(key_hash)
            .map_err(|e| report!(OcspError::Construction(format!("OctetString error: {e}"))))?,
    );
    Ok(responder_id)
}

/// Extract the raw public key bytes (BIT STRING content) from a PEM-encoded certificate.
fn extract_ca_public_key_bytes(ca_cert_pem: &str) -> OcspResult<Vec<u8>> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(ca_cert_pem.as_bytes())
        .map_err(|_| report!(OcspError::PemParse))?;
    let cert = pem_block
        .parse_x509()
        .map_err(|_| report!(OcspError::X509Parse))?;
    Ok(cert
        .tbs_certificate
        .subject_pki
        .subject_public_key
        .data
        .to_vec())
}

/// Compute a hash of the given bytes using the algorithm identified by `oid`.
///
/// Supports SHA-1 (used by Nginx/OpenSSL) and SHA-256.
/// Returns `None` for unsupported algorithms.
fn compute_key_hash(data: &[u8], oid: &const_oid::ObjectIdentifier) -> Option<Vec<u8>> {
    if *oid == SHA1_OID {
        let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY, data);
        Some(digest.as_ref().to_vec())
    } else if *oid == SHA256_OID {
        let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, data);
        Some(digest.as_ref().to_vec())
    } else {
        None
    }
}

/// Create an OcspGeneralizedTime from an OffsetDateTime.
fn make_ocsp_time(dt: OffsetDateTime) -> OcspGeneralizedTime {
    OcspGeneralizedTime::from(der::asn1::GeneralizedTime::from_date_time(
        der::DateTime::new(
            dt.year() as u16,
            dt.month() as u8,
            dt.day(),
            dt.hour(),
            dt.minute(),
            dt.second(),
        )
        .expect("valid datetime"),
    ))
}

fn build_single_response(cert_id: CertId, status: CertStatus) -> SingleResponse {
    let now = make_ocsp_time(OffsetDateTime::now_utc());
    let next_update_time = OffsetDateTime::now_utc() + time::Duration::hours(1);
    let next_update = make_ocsp_time(next_update_time);

    SingleResponse {
        cert_id,
        cert_status: status,
        this_update: now,
        next_update: Some(next_update),
        single_extensions: None,
    }
}

/// Look up certificate status in the database.
async fn lookup_cert_status(
    db: &DatabaseConnection,
    serial_hex: &str,
    ca_snapshot: &CaSnapshotData,
) -> OcspResult<CertStatus> {
    use uptrakit_shared_db::entity::service_certificate;

    // Search by serial number across both active and previous CA fingerprints
    let mut condition = Condition::any().add(
        Condition::all()
            .add(service_certificate::Column::SerialNumber.eq(serial_hex))
            .add(service_certificate::Column::CaFingerprint.eq(&ca_snapshot.active_fingerprint)),
    );

    if let Some(prev_fp) = &ca_snapshot.previous_fingerprint {
        condition = condition.add(
            Condition::all()
                .add(service_certificate::Column::SerialNumber.eq(serial_hex))
                .add(service_certificate::Column::CaFingerprint.eq(prev_fp)),
        );
    }

    let cert = service_certificate::Entity::find()
        .filter(condition)
        .one(db)
        .await
        .context_to::<OcspError>()?;

    let Some(cert) = cert else {
        return Ok(CertStatus::unknown());
    };

    // Check if expired
    if cert.not_after < OffsetDateTime::now_utc() {
        return Ok(CertStatus::unknown());
    }

    // Check if revoked
    if let Some(revoked_at) = cert.revoked_at {
        let revocation_time = make_ocsp_time(revoked_at);

        // Map our revocation reasons to CRL reasons
        let reason = cert.revocation_reason.as_ref().map(|r| match r {
            service_certificate::RevocationReason::CertificateRenewed => CrlReason::Superseded,
            service_certificate::RevocationReason::ServiceDeactivated => {
                CrlReason::CessationOfOperation
            }
            service_certificate::RevocationReason::ServiceMerged => CrlReason::Superseded,
        });

        let revoked_info = x509_ocsp::RevokedInfo {
            revocation_time,
            revocation_reason: reason,
        };

        return Ok(CertStatus::revoked(revoked_info));
    }

    Ok(CertStatus::good())
}

/// Sign the response data and produce a DER-encoded OcspResponse.
fn sign_response(
    response_data: &ResponseData,
    ca_snapshot: &CaSnapshotData,
) -> OcspResult<Vec<u8>> {
    // Encode the response data to DER for signing
    let tbs_der = response_data
        .to_der()
        .map_err(|e| report!(OcspError::DerEncode(e.to_string())))?;

    // Parse the CA private key
    let key_pem = &ca_snapshot.active_key_pem;
    let key_der = pem_to_der_key(key_pem)?;

    // Sign with ECDSA P-256 SHA-256 using aws-lc-rs
    let signing_key = aws_lc_rs::signature::EcdsaKeyPair::from_pkcs8(
        &aws_lc_rs::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &key_der,
    )
    .map_err(|e| report!(OcspError::KeyParse(e.to_string())))?;

    let signature_bytes = signing_key
        .sign(&aws_lc_rs::rand::SystemRandom::new(), &tbs_der)
        .map_err(|e| report!(OcspError::Signing(e.to_string())))?;

    let signature = BitString::from_bytes(signature_bytes.as_ref())
        .map_err(|e| report!(OcspError::Construction(e.to_string())))?;

    // Algorithm identifier for ECDSA with SHA-256
    let algorithm = spki::AlgorithmIdentifierOwned {
        oid: const_oid::db::rfc5912::ECDSA_WITH_SHA_256,
        parameters: None,
    };

    // Build BasicOcspResponse
    let basic_response = BasicOcspResponse {
        tbs_response_data: response_data.clone(),
        signature_algorithm: algorithm,
        signature,
        certs: None,
    };

    // Wrap in OcspResponse
    let response = OcspResponse::successful(basic_response)
        .map_err(|e| report!(OcspError::Construction(e.to_string())))?;

    response
        .to_der()
        .map_err(|e| report!(OcspError::DerEncode(e.to_string())))
}

/// Extract a PKCS#8 DER private key from PEM.
fn pem_to_der_key(pem: &str) -> OcspResult<Vec<u8>> {
    let mut der_data = Vec::new();
    let mut in_block = false;

    for line in pem.lines() {
        if line.starts_with("-----BEGIN") {
            in_block = true;
            continue;
        }
        if line.starts_with("-----END") {
            break;
        }
        if in_block {
            use base64::Engine;
            der_data.extend_from_slice(
                &base64::engine::general_purpose::STANDARD
                    .decode(line.trim())
                    .map_err(|e| report!(OcspError::Base64Decode(e.to_string())))?,
            );
        }
    }

    if der_data.is_empty() {
        return Err(report!(OcspError::EmptyPemData));
    }

    Ok(der_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_response_malformed_request() {
        let response_der = build_error_response(OcspResponseStatus::MalformedRequest);
        assert!(!response_der.is_empty());
        let response = OcspResponse::from_der(&response_der).unwrap();
        assert_eq!(
            response.response_status,
            OcspResponseStatus::MalformedRequest
        );
        assert!(response.response_bytes.is_none());
    }

    #[test]
    fn error_response_internal_error() {
        let response_der = build_error_response(OcspResponseStatus::InternalError);
        assert!(!response_der.is_empty());
        let response = OcspResponse::from_der(&response_der).unwrap();
        assert_eq!(response.response_status, OcspResponseStatus::InternalError);
    }

    #[test]
    fn pem_to_der_key_works() {
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let pem = key_pair.serialize_pem();
        let der = pem_to_der_key(&pem).unwrap();
        assert!(!der.is_empty());
    }

    #[test]
    fn extract_ca_public_key_and_hash_works() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert = params.self_signed(&key_pair).unwrap();
        let pem = cert.pem();

        let pub_key = extract_ca_public_key_bytes(&pem).unwrap();
        assert!(!pub_key.is_empty());

        // SHA-256 hash should be 32 bytes
        let hash_256 = compute_key_hash(&pub_key, &SHA256_OID).unwrap();
        assert_eq!(hash_256.len(), 32);

        // SHA-1 hash should be 20 bytes
        let hash_1 = compute_key_hash(&pub_key, &SHA1_OID).unwrap();
        assert_eq!(hash_1.len(), 20);

        // Hashes should be deterministic
        let hash_256_2 = compute_key_hash(&pub_key, &SHA256_OID).unwrap();
        assert_eq!(hash_256, hash_256_2);

        let hash_1_2 = compute_key_hash(&pub_key, &SHA1_OID).unwrap();
        assert_eq!(hash_1, hash_1_2);

        // SHA-1 and SHA-256 should produce different hashes
        assert_ne!(hash_1.as_slice(), hash_256.as_slice());
    }

    #[test]
    fn unsupported_hash_algorithm_returns_none() {
        let unknown_oid = const_oid::ObjectIdentifier::new_unwrap("1.2.3.4.5.6.7.8.9");
        assert!(compute_key_hash(b"some data", &unknown_oid).is_none());
    }

    #[test]
    fn responder_id_uses_sha1() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert = params.self_signed(&key_pair).unwrap();
        let pem = cert.pem();

        let pub_key = extract_ca_public_key_bytes(&pem).unwrap();
        let responder_id = build_responder_id(&pub_key).unwrap();

        // ResponderID::ByKey should contain a SHA-1 hash (20 bytes)
        match responder_id {
            ResponderId::ByKey(key_hash) => {
                assert_eq!(
                    key_hash.as_bytes().len(),
                    20,
                    "ResponderID must use SHA-1 (20 bytes)"
                );
                let expected = compute_key_hash(&pub_key, &SHA1_OID).unwrap();
                assert_eq!(key_hash.as_bytes(), expected.as_slice());
            }
            _ => panic!("expected ResponderID::ByKey"),
        }
    }

    #[test]
    fn malformed_request_returns_error() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let snapshot = CaSnapshotData {
                active_cert_pem: String::new(),
                active_key_pem: String::new(),
                active_fingerprint: String::new(),
                previous_cert_pem: None,
                previous_key_pem: None,
                previous_fingerprint: None,
                bundle_pem: String::new(),
                bundle_hash: String::new(),
                managed: true,
                active_not_after: OffsetDateTime::now_utc(),
                pki_addr: None,
            };

            let db = {
                let opt = sea_orm::ConnectOptions::new("sqlite::memory:".to_owned());
                sea_orm::Database::connect(opt).await.unwrap()
            };

            let response_der = build_ocsp_response(b"not valid der", &snapshot, &db).await;
            let response = OcspResponse::from_der(&response_der).unwrap();
            assert_eq!(
                response.response_status,
                OcspResponseStatus::MalformedRequest
            );
        });
    }

    #[test]
    fn format_serial_hex_works() {
        assert_eq!(format_serial_hex(&[0x01, 0x0a, 0xff]), "010aff");
        assert_eq!(format_serial_hex(&[0x00]), "00");
    }
}
