use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
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

/// Standalone HTTP and HTTPS OCSP responder for integration testing.
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

/// Build the shared Axum router for both HTTP and HTTPS modes.
///
/// Handles:
/// - `POST /` and `POST /api/v1/pki/ocsp` — standard OCSP POST requests
/// - `GET /healthz` — health check
/// - Any other request — treated as GET OCSP (RFC 6960 Appendix A.1)
///   where the base64-encoded DER request is in the URL path
fn build_router(state: Arc<OcspState>) -> Router {
    Router::new()
        .route("/", post(handle_ocsp))
        .route("/api/v1/pki/ocsp", post(handle_ocsp))
        .route("/healthz", get(|| async { "ok" }))
        .fallback(handle_ocsp_get)
        .with_state(state)
}

fn build_ocsp_state(
    ca_cert_pem: &str,
    ca_key_pem: &str,
    revoked_serials: Vec<String>,
) -> Arc<OcspState> {
    let ca_public_key_bytes = extract_public_key_bytes(ca_cert_pem);
    let ca_key_der = pem_to_der(ca_key_pem);

    Arc::new(OcspState {
        ca_public_key_bytes,
        ca_key_der,
        revoked_serials,
        request_count: AtomicUsize::new(0),
    })
}

impl OcspResponder {
    /// Start a test OCSP responder on a random port (plain HTTP).
    ///
    /// - `ca_cert_pem`: PEM-encoded CA certificate
    /// - `ca_key_pem`: PEM-encoded CA private key
    /// - `revoked_serials`: serial numbers (lowercase hex) to report as revoked
    pub async fn start(ca_cert_pem: &str, ca_key_pem: &str, revoked_serials: Vec<String>) -> Self {
        let listener = TcpListener::bind("0.0.0.0:0").expect("bind OCSP responder");
        Self::start_http_with_listener(listener, ca_cert_pem, ca_key_pem, revoked_serials).await
    }

    /// Start a test OCSP responder on a specific port (plain HTTP).
    ///
    /// Panics if the port cannot be bound.
    pub async fn start_on_port(
        port: u16,
        ca_cert_pem: &str,
        ca_key_pem: &str,
        revoked_serials: Vec<String>,
    ) -> Self {
        let listener = TcpListener::bind(format!("0.0.0.0:{port}"))
            .unwrap_or_else(|e| panic!("failed to bind OCSP responder to port {port}: {e}"));
        Self::start_http_with_listener(listener, ca_cert_pem, ca_key_pem, revoked_serials).await
    }

    async fn start_http_with_listener(
        listener: TcpListener,
        ca_cert_pem: &str,
        ca_key_pem: &str,
        revoked_serials: Vec<String>,
    ) -> Self {
        let state = build_ocsp_state(ca_cert_pem, ca_key_pem, revoked_serials);
        let router = build_router(Arc::clone(&state));

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

    /// Start a test OCSP responder on a specific port (HTTPS / TLS).
    ///
    /// Panics if the port cannot be bound.
    pub async fn start_https_on_port(
        port: u16,
        ca_cert_pem: &str,
        ca_key_pem: &str,
        server_cert_pem: &str,
        server_key_pem: &str,
        revoked_serials: Vec<String>,
    ) -> Self {
        let listener = TcpListener::bind(format!("0.0.0.0:{port}"))
            .unwrap_or_else(|e| panic!("failed to bind HTTPS OCSP responder to port {port}: {e}"));
        Self::start_https_with_listener(
            listener,
            ca_cert_pem,
            ca_key_pem,
            server_cert_pem,
            server_key_pem,
            revoked_serials,
        )
        .await
    }

    async fn start_https_with_listener(
        listener: TcpListener,
        ca_cert_pem: &str,
        ca_key_pem: &str,
        server_cert_pem: &str,
        server_key_pem: &str,
        revoked_serials: Vec<String>,
    ) -> Self {
        let state = build_ocsp_state(ca_cert_pem, ca_key_pem, revoked_serials);
        let router = build_router(Arc::clone(&state));

        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();

        let rustls_config = build_rustls_config(server_cert_pem, server_key_pem);

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

        let handle = axum_server::Handle::new();
        let server_handle = handle.clone();

        tokio::spawn(async move {
            axum_server::from_tcp_rustls(listener, rustls_config)
                .expect("from_tcp_rustls OCSP")
                .handle(server_handle)
                .serve(router.into_make_service())
                .await
                .expect("HTTPS OCSP responder serve error");
        });

        // Bridge watch-based shutdown to axum_server Handle
        let shutdown_handle = handle.clone();
        tokio::spawn(async move {
            shutdown_rx.changed().await.ok();
            shutdown_handle.graceful_shutdown(None);
        });

        // Wait until the server is actually listening.
        handle.listening().await;

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

fn build_rustls_config(
    server_cert_pem: &str,
    server_key_pem: &str,
) -> axum_server::tls_rustls::RustlsConfig {
    let (_, cert_pem) = x509_parser::pem::parse_x509_pem(server_cert_pem.as_bytes())
        .expect("parse OCSP server cert PEM");
    let cert_der = rustls::pki_types::CertificateDer::from(cert_pem.contents);

    let key_der = rustls::pki_types::PrivateKeyDer::try_from(pem_to_der(server_key_pem))
        .expect("OCSP server key DER");

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .expect("rustls OCSP server config");

    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
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

/// Handle GET-based OCSP requests (RFC 6960 Appendix A.1).
///
/// Nginx sends OCSP requests as GET with the URL path containing the
/// URL-encoded base64-encoded DER OCSP request:
///   GET /{url-encoding of base-64 encoding of the DER encoding of OCSPRequest}
async fn handle_ocsp_get(
    State(state): State<Arc<OcspState>>,
    request: Request,
) -> impl IntoResponse {
    use base64::Engine;

    let path = request.uri().path();

    // RFC 6960: GET {url}/{url-encoding of base64(DER(OCSPRequest))}
    // The base64 content is in the last path segment (after the last '/').
    let b64_segment = path.rsplit('/').next().unwrap_or("");
    if b64_segment.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [("content-type", "text/plain")],
            Vec::from("empty OCSP request path"),
        );
    }

    // URL-decode the segment (percent-decoding)
    let url_decoded = percent_decode(b64_segment);

    // Try standard base64 first, then URL-safe
    let der = base64::engine::general_purpose::STANDARD
        .decode(&url_decoded)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(&url_decoded));

    match der {
        Ok(der_bytes) => {
            state.request_count.fetch_add(1, Ordering::Relaxed);
            let response_der = build_response(&der_bytes, &state);
            (
                StatusCode::OK,
                [("content-type", "application/ocsp-response")],
                response_der,
            )
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            [("content-type", "text/plain")],
            Vec::from("invalid base64 in OCSP request"),
        ),
    }
}

/// Simple percent-decoding for URL paths.
fn percent_decode(input: &str) -> String {
    let mut result = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            result.push(byte);
            i += 3;
            continue;
        }
        result.push(bytes[i]);
        i += 1;
    }
    // Return as a string; base64 chars are ASCII so this is safe
    String::from_utf8(result).unwrap_or_else(|e| String::from_utf8_lossy(&e.into_bytes()).into())
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
    let year = u16::try_from(dt.year()).expect("year fits in u16");
    let month: u8 = dt.month().into();
    let der_dt = der::DateTime::new(year, month, dt.day(), dt.hour(), dt.minute(), dt.second())
        .expect("valid datetime components");
    OcspGeneralizedTime::from(der::asn1::GeneralizedTime::from_date_time(der_dt))
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
