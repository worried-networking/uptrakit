use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair};
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use der::asn1::{BitString, OctetString};
use der::{Decode, Encode};
use x509_ocsp::{
    BasicOcspResponse, CertId, CertStatus, OcspGeneralizedTime, OcspRequest, OcspResponse,
    OcspResponseStatus, ResponderId, ResponseData, SingleResponse, Version,
};

/// SHA-1 OID (`1.3.14.3.2.26`).
const SHA1_OID: const_oid::ObjectIdentifier =
    const_oid::ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
/// SHA-256 OID (`2.16.840.1.101.3.4.2.1`).
const SHA256_OID: const_oid::ObjectIdentifier =
    const_oid::ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");

/// Standalone plain-HTTP OCSP responder for integration testing.
///
/// Does NOT use the production OCSP code to avoid DB migrations.
/// Signs responses with the CA key using ECDSA P-256 SHA-256.
pub struct OcspResponder {
    port: u16,
    state: Arc<OcspState>,
    shutdown: tokio::sync::watch::Sender<bool>,
}

struct OcspState {
    /// Raw CA public key bytes (BIT STRING content) for key hash matching.
    ca_public_key_bytes: Vec<u8>,
    /// PKCS#8 DER-encoded CA private key for signing.
    ca_key_der: Vec<u8>,
    /// Serial numbers (lowercase hex) that should be reported as revoked.
    revoked_serials: Vec<String>,
    /// Number of OCSP requests processed.
    request_count: AtomicUsize,
}

impl OcspResponder {
    /// Start a test OCSP responder on a random port.
    ///
    /// - `ca_cert_pem`: PEM-encoded CA certificate
    /// - `ca_key_pem`: PEM-encoded CA private key
    /// - `revoked_serials`: serial numbers (lowercase hex) to report as revoked
    pub async fn start(ca_cert_pem: &str, ca_key_pem: &str, revoked_serials: Vec<String>) -> Self {
        let ca_public_key_bytes = extract_public_key_bytes(ca_cert_pem);
        let ca_key_der = pem_to_der(ca_key_pem);

        let state = Arc::new(OcspState {
            ca_public_key_bytes,
            ca_key_der,
            revoked_serials,
            request_count: AtomicUsize::new(0),
        });

        let router = Router::new()
            .route("/", post(handle_ocsp))
            .with_state(Arc::clone(&state));

        let listener = TcpListener::bind("0.0.0.0:0").expect("bind OCSP responder");
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();

        let tokio_listener =
            tokio::net::TcpListener::from_std(listener).expect("tokio listener from std");

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

        tokio::spawn(async move {
            axum::serve(tokio_listener, router.into_make_service())
                .with_graceful_shutdown(async move {
                    shutdown_rx.changed().await.ok();
                })
                .await
                .expect("OCSP responder serve error");
        });

        Self {
            port,
            state,
            shutdown: shutdown_tx,
        }
    }

    /// The port the responder is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Number of OCSP requests processed so far.
    pub fn request_count(&self) -> usize {
        self.state.request_count.load(Ordering::Relaxed)
    }

    /// Shut down the responder.
    pub fn shutdown(self) {
        let _ = self.shutdown.send(true);
    }
}

async fn handle_ocsp(State(state): State<Arc<OcspState>>, body: Bytes) -> impl IntoResponse {
    state.request_count.fetch_add(1, Ordering::Relaxed);

    let response_der = build_response(&body, &state);

    (
        StatusCode::OK,
        [("content-type", "application/ocsp-response")],
        response_der,
    )
}

fn build_response(request_der: &[u8], state: &OcspState) -> Vec<u8> {
    let ocsp_request = match OcspRequest::from_der(request_der) {
        Ok(req) => req,
        Err(_) => return build_error_response(OcspResponseStatus::MalformedRequest),
    };

    // Build responder ID using SHA-1 per RFC 6960 Section 2.3
    let responder_id = match build_responder_id(&state.ca_public_key_bytes) {
        Ok(id) => id,
        Err(_) => return build_error_response(OcspResponseStatus::InternalError),
    };

    let mut single_responses = Vec::new();

    for request in &ocsp_request.tbs_request.request_list {
        let cert_id = &request.req_cert;
        let hash_oid = &cert_id.hash_algorithm.oid;

        // Compute issuer key hash using the client's algorithm
        let expected_hash = match compute_key_hash(&state.ca_public_key_bytes, hash_oid) {
            Some(h) => h,
            None => {
                single_responses.push(build_single_response(
                    cert_id.clone(),
                    CertStatus::unknown(),
                ));
                continue;
            }
        };

        // Verify issuer key hash matches our CA
        if cert_id.issuer_key_hash.as_bytes() != expected_hash.as_slice() {
            single_responses.push(build_single_response(
                cert_id.clone(),
                CertStatus::unknown(),
            ));
            continue;
        }

        // Check serial against revoked list
        let serial_hex = format_serial_hex(cert_id.serial_number.as_bytes());
        let status = if state.revoked_serials.contains(&serial_hex) {
            let now = make_ocsp_time(time::OffsetDateTime::now_utc());
            CertStatus::revoked(x509_ocsp::RevokedInfo {
                revocation_time: now,
                revocation_reason: None,
            })
        } else {
            CertStatus::good()
        };

        single_responses.push(build_single_response(cert_id.clone(), status));
    }

    let now = make_ocsp_time(time::OffsetDateTime::now_utc());
    let response_data = ResponseData {
        version: Version::V1,
        responder_id,
        produced_at: now,
        responses: single_responses,
        response_extensions: None,
    };

    match sign_response(&response_data, &state.ca_key_der) {
        Ok(der) => der,
        Err(_) => build_error_response(OcspResponseStatus::InternalError),
    }
}

fn build_responder_id(pub_key_bytes: &[u8]) -> Result<ResponderId, String> {
    let key_hash =
        compute_key_hash(pub_key_bytes, &SHA1_OID).ok_or("SHA-1 hash failed".to_string())?;
    let octet = OctetString::new(key_hash).map_err(|e| format!("OctetString error: {e}"))?;
    Ok(ResponderId::ByKey(octet))
}

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

fn format_serial_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
}

fn make_ocsp_time(dt: time::OffsetDateTime) -> OcspGeneralizedTime {
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
    let now = make_ocsp_time(time::OffsetDateTime::now_utc());
    let next = make_ocsp_time(time::OffsetDateTime::now_utc() + time::Duration::hours(1));
    SingleResponse {
        cert_id,
        cert_status: status,
        this_update: now,
        next_update: Some(next),
        single_extensions: None,
    }
}

fn build_error_response(status: OcspResponseStatus) -> Vec<u8> {
    let resp = match status {
        OcspResponseStatus::MalformedRequest => OcspResponse::malformed_request(),
        _ => OcspResponse::internal_error(),
    };
    resp.to_der().unwrap_or_default()
}

fn sign_response(response_data: &ResponseData, ca_key_der: &[u8]) -> Result<Vec<u8>, String> {
    let tbs_der = response_data
        .to_der()
        .map_err(|e| format!("DER encode: {e}"))?;

    let signing_key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, ca_key_der)
        .map_err(|e| format!("key parse: {e}"))?;

    let sig = signing_key
        .sign(&SystemRandom::new(), &tbs_der)
        .map_err(|e| format!("sign: {e}"))?;

    let signature = BitString::from_bytes(sig.as_ref()).map_err(|e| format!("BitString: {e}"))?;

    let algorithm = spki::AlgorithmIdentifierOwned {
        oid: const_oid::db::rfc5912::ECDSA_WITH_SHA_256,
        parameters: None,
    };

    let basic = BasicOcspResponse {
        tbs_response_data: response_data.clone(),
        signature_algorithm: algorithm,
        signature,
        certs: None,
    };

    let response = OcspResponse::successful(basic).map_err(|e| format!("OcspResponse: {e}"))?;

    response
        .to_der()
        .map_err(|e| format!("DER encode response: {e}"))
}

/// Extract raw public key bytes from a PEM certificate.
fn extract_public_key_bytes(cert_pem: &str) -> Vec<u8> {
    let (_, pem_block) =
        x509_parser::pem::parse_x509_pem(cert_pem.as_bytes()).expect("parse CA cert PEM");
    let cert = pem_block.parse_x509().expect("parse CA X.509");
    cert.tbs_certificate
        .subject_pki
        .subject_public_key
        .data
        .to_vec()
}

/// Decode a PEM block to DER bytes.
fn pem_to_der(pem_str: &str) -> Vec<u8> {
    use base64::Engine;
    let b64: String = pem_str
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("base64 decode PEM")
}
