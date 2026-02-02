use der::asn1::{BitString, OctetString};
use der::{Decode, Encode};
use sea_orm::{ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use x509_cert::ext::pkix::CrlReason;
use x509_ocsp::{
    BasicOcspResponse, CertId, CertStatus, OcspGeneralizedTime, OcspRequest, OcspResponse,
    OcspResponseStatus, ResponderId, ResponseData, SingleResponse, Version,
};

use crate::ca_snapshot::CaSnapshotData;

/// Build an OCSP response for the given DER-encoded OCSP request.
///
/// Queries the `agent_certificates` table to determine certificate status.
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

    // Build responder ID (by key hash of CA public key)
    let (responder_id, ca_key_hash) = match build_responder_id(&ca_snapshot.active_cert_pem) {
        Ok(v) => v,
        Err(_) => return build_error_response(OcspResponseStatus::InternalError),
    };

    // Optionally compute previous CA key hash
    let prev_key_hash = ca_snapshot
        .previous_cert_pem
        .as_deref()
        .and_then(|pem| extract_ca_key_hash(pem).ok());

    // Process each request in the list
    let mut single_responses = Vec::new();

    for request in &ocsp_request.tbs_request.request_list {
        let cert_id = &request.req_cert;

        // Validate that the issuer key hash matches our CA (active or previous)
        let matches_active = cert_id.issuer_key_hash.as_bytes() == ca_key_hash.as_slice();
        let matches_previous = prev_key_hash
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

/// Build the responder ID from the CA certificate (by key hash).
fn build_responder_id(
    ca_cert_pem: &str,
) -> Result<(ResponderId, Vec<u8>), Box<dyn std::error::Error>> {
    let key_hash = extract_ca_key_hash(ca_cert_pem)?;
    let responder_id = ResponderId::ByKey(
        OctetString::new(key_hash.clone()).map_err(|e| format!("OctetString error: {e}"))?,
    );
    Ok((responder_id, key_hash))
}

/// Extract the SHA-256 hash of the CA's public key (SubjectPublicKeyInfo.subjectPublicKey).
fn extract_ca_key_hash(ca_cert_pem: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let (_, pem_block) =
        x509_parser::pem::parse_x509_pem(ca_cert_pem.as_bytes()).map_err(|_| "PEM parse error")?;
    let cert = pem_block.parse_x509().map_err(|_| "X.509 parse error")?;

    // Hash the raw public key bytes (the BIT STRING content, not the SPKI wrapper)
    let pub_key_bytes = cert.tbs_certificate.subject_pki.subject_public_key.data;

    let mut hasher = Sha256::new();
    hasher.update(pub_key_bytes);
    Ok(hasher.finalize().to_vec())
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
) -> Result<CertStatus, Box<dyn std::error::Error>> {
    use uptrakit_shared_db::entity::agent_certificate;

    // Search by serial number across both active and previous CA fingerprints
    let mut condition = Condition::any().add(
        Condition::all()
            .add(agent_certificate::Column::SerialNumber.eq(serial_hex))
            .add(agent_certificate::Column::CaFingerprint.eq(&ca_snapshot.active_fingerprint)),
    );

    if let Some(prev_fp) = &ca_snapshot.previous_fingerprint {
        condition = condition.add(
            Condition::all()
                .add(agent_certificate::Column::SerialNumber.eq(serial_hex))
                .add(agent_certificate::Column::CaFingerprint.eq(prev_fp)),
        );
    }

    let cert = agent_certificate::Entity::find()
        .filter(condition)
        .one(db)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

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
            agent_certificate::RevocationReason::CertificateRenewed => CrlReason::Superseded,
            agent_certificate::RevocationReason::AgentDeactivated => {
                CrlReason::CessationOfOperation
            }
            agent_certificate::RevocationReason::AgentMerged => CrlReason::Superseded,
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
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Encode the response data to DER for signing
    let tbs_der = response_data
        .to_der()
        .map_err(|e| format!("DER encode error: {e}"))?;

    // Parse the CA private key
    let key_pem = &ca_snapshot.active_key_pem;
    let key_der = pem_to_der_key(key_pem)?;

    // Sign with ECDSA P-256 SHA-256 using aws-lc-rs
    let signing_key = aws_lc_rs::signature::EcdsaKeyPair::from_pkcs8(
        &aws_lc_rs::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &key_der,
    )
    .map_err(|e| format!("Key parse error: {e}"))?;

    let signature_bytes = signing_key
        .sign(&aws_lc_rs::rand::SystemRandom::new(), &tbs_der)
        .map_err(|e| format!("Signing error: {e}"))?;

    let signature =
        BitString::from_bytes(signature_bytes.as_ref()).map_err(|e| format!("BitString: {e}"))?;

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
        .map_err(|e| format!("OcspResponse construction error: {e}"))?;

    response
        .to_der()
        .map_err(|e| format!("OcspResponse DER encode error: {e}").into())
}

/// Extract a PKCS#8 DER private key from PEM.
fn pem_to_der_key(pem: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
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
                    .map_err(|e| format!("Base64 decode error: {e}"))?,
            );
        }
    }

    if der_data.is_empty() {
        return Err("empty PEM data".into());
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
    fn extract_ca_key_hash_works() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert = params.self_signed(&key_pair).unwrap();
        let pem = cert.pem();

        let hash = extract_ca_key_hash(&pem).unwrap();
        assert_eq!(hash.len(), 32); // SHA-256 is 32 bytes

        // Should be deterministic
        let hash2 = extract_ca_key_hash(&pem).unwrap();
        assert_eq!(hash, hash2);
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
                backend_url: None,
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
