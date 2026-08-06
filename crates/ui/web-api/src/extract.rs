#![expect(
    clippy::string_slice,
    reason = "slice index is at a validated ASCII boundary"
)]

use std::convert::Infallible;
use std::net::IpAddr;
use std::ops::Deref;

use axum::Form;
use axum::extract::{FromRef, FromRequest, FromRequestParts, OptionalFromRequest};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::Response;
use serde::de::DeserializeOwned;
use uptrakit_web_api_auth::auth::api_token::ApiTokenService;
use uptrakit_web_api_auth::auth::session::SessionService;
use uptrakit_web_api_types::validation::{Validate, ValidationError};

use crate::app_state::DbState;

use crate::error_response::error_response;

/// The resolved client IP address.
/// Set by the `resolve_ip` middleware.
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);

/// The trusted proxy's IP address.
/// Only present when the peer is a known trusted proxy.
/// Set by the `resolve_ip` middleware.
#[derive(Debug, Clone, Copy)]
pub struct ProxyIp(pub IpAddr);

/// The verified service identity from a client TLS certificate.
/// Injected by the mTLS acceptor when the peer presents a valid cert
/// signed by the internal CA, with a parseable UUID as CN.
#[derive(Debug, Clone)]
pub struct ServiceIdentity {
    pub service_id: uuid::Uuid,
    pub cert_serial: String,
}

/// The external base URL resolved from proxy headers or the Host header.
/// Used for constructing redirect URLs (OIDC callbacks, device auth, etc.)
/// when the controller is behind a reverse proxy.
#[derive(Debug, Clone)]
pub struct ExternalBaseUrl(pub String);

/// Parse [`ServiceIdentity`] from DER-encoded X.509 certificate bytes.
pub fn service_identity_from_der(der: &[u8]) -> Option<ServiceIdentity> {
    use const_oid::db::rfc4519::COMMON_NAME;
    use der::{
        Decode,
        asn1::{PrintableStringRef, Utf8StringRef},
    };
    use x509_cert::Certificate;
    let cert = Certificate::from_der(der).ok()?;
    let tbs = &cert.tbs_certificate;
    let cn = tbs
        .subject
        .0
        .iter()
        .flat_map(|rdn| rdn.0.iter())
        .filter(|atv| atv.oid == COMMON_NAME)
        .find_map(|atv| {
            atv.value
                .decode_as::<Utf8StringRef<'_>>()
                .map(|s| s.as_str().to_owned())
                .ok()
                .or_else(|| {
                    atv.value
                        .decode_as::<PrintableStringRef<'_>>()
                        .map(|s| s.as_str().to_owned())
                        .ok()
                })
        })?;
    let service_id = uuid::Uuid::parse_str(&cn).ok()?;
    let cert_serial = tbs.serial_number.to_string();
    Some(ServiceIdentity {
        service_id,
        cert_serial,
    })
}

/// Parse [`ServiceIdentity`] from structured info header fields.
///
/// Returns `ServiceIdentity` with `cert_serial = ""` if `serial` is absent,
/// signalling service-id-only lookup.
pub fn service_identity_from_info(
    subject_dn: &str,
    serial: Option<&str>,
) -> Option<ServiceIdentity> {
    let cn = extract_cn_from_dn(subject_dn)?;
    let service_id = uuid::Uuid::parse_str(cn).ok()?;
    let cert_serial = match serial {
        Some(s) => normalize_serial(s),
        None => String::new(),
    };
    Some(ServiceIdentity {
        service_id,
        cert_serial,
    })
}

/// Extract the CN value from a Distinguished Name string.
///
/// Handles formats:
/// - RFC 2253: `CN=uuid,O=Org`
/// - Single: `CN=uuid`
/// - OpenSSL: `/CN=uuid/O=Org`
/// - URL-encoded: `CN%3Duuid`
pub fn extract_cn_from_dn(dn: &str) -> Option<&str> {
    // Try URL-decoded matching first by checking for %3D (case-insensitive)
    if dn.contains("%3D") || dn.contains("%3d") {
        // URL-encoded: find CN%3D or cn%3d
        let lower = dn.to_ascii_lowercase();
        if let Some(start) = lower.find("cn%3d") {
            let value_start = start + 5; // len("cn%3d")
            let value = &dn[value_start..];
            // Value ends at comma, slash, or end
            let end = value.find([',', '/', '+']).unwrap_or(value.len());
            let cn = &value[..end];
            if !cn.is_empty() {
                return Some(cn);
            }
        }
        return None;
    }

    // OpenSSL format: /CN=uuid/O=Org
    if dn.starts_with('/') {
        for part in dn.split('/') {
            if let Some(value) = part.strip_prefix("CN=")
                && !value.is_empty()
            {
                return Some(value);
            }
        }
        return None;
    }

    // RFC 2253 format: CN=uuid,O=Org or CN=uuid
    for part in dn.split(',') {
        let trimmed = part.trim();
        if let Some(value) = trimmed.strip_prefix("CN=")
            && !value.is_empty()
        {
            return Some(value);
        }
    }

    None
}

/// Normalize a hex serial number to colon-separated uppercase format
/// matching x509-cert's `SerialNumber::Display` output.
///
/// Examples:
/// - `"01abcdef"` → `"01:AB:CD:EF"`
/// - `"01:ab:cd:ef"` → `"01:AB:CD:EF"`
/// - `"01:AB:CD:EF"` → `"01:AB:CD:EF"` (passthrough)
pub fn normalize_serial(hex: &str) -> String {
    // Strip colons and uppercase
    let clean: String = hex
        .chars()
        .filter(|c| *c != ':')
        .flat_map(|c| c.to_uppercase())
        .collect();

    // Insert colons every 2 chars
    let mut result = String::with_capacity(clean.len() + clean.len() / 2);
    for (i, ch) in clean.chars().enumerate() {
        if i > 0 && i % 2 == 0 {
            result.push(':');
        }
        result.push(ch);
    }
    result
}

/// Extract service UUID from SPIFFE URI: `spiffe://<trust_domain>/service/<uuid>`.
///
/// Returns `None` if the scheme is not `"spiffe"`, if the host does not match
/// `trust_domain`, or if the path is not exactly `/service/<uuid>` where
/// `<uuid>` is a valid [`uuid::Uuid`].
fn try_parse_spiffe_service_id(uri: &str, trust_domain: &str) -> Option<uuid::Uuid> {
    let url = url::Url::parse(uri).ok()?;
    if url.scheme() != "spiffe" {
        return None;
    }
    if url.host_str() != Some(trust_domain) {
        return None;
    }
    let mut segments = url.path_segments()?;
    let kind = segments.next()?;
    let id_str = segments.next()?;
    if kind != "service" || segments.next().is_some() {
        return None;
    }
    id_str.parse::<uuid::Uuid>().ok()
}

/// Extract service UUID from certificate CN (migration fallback).
fn extract_cn_service_id(cert: &x509_cert::Certificate) -> Option<uuid::Uuid> {
    use const_oid::db::rfc4519::COMMON_NAME;
    use der::asn1::{PrintableStringRef, Utf8StringRef};
    cert.tbs_certificate
        .subject
        .0
        .iter()
        .flat_map(|rdn| rdn.0.iter())
        .filter(|atv| atv.oid == COMMON_NAME)
        .find_map(|atv| {
            atv.value
                .decode_as::<Utf8StringRef<'_>>()
                .map(|s| s.as_str().to_owned())
                .ok()
                .or_else(|| {
                    atv.value
                        .decode_as::<PrintableStringRef<'_>>()
                        .map(|s| s.as_str().to_owned())
                        .ok()
                })
        })
        .and_then(|cn| cn.parse::<uuid::Uuid>().ok())
}

/// Parse [`ServiceIdentity`] from DER-encoded X.509 certificate bytes,
/// preferring a SPIFFE URI SAN over the CN (migration fallback).
///
/// - If the cert contains a `spiffe://<trust_domain>/service/<uuid>` URI SAN
///   matching `trust_domain`, that UUID is used as the service identity.
/// - Otherwise falls back to CN (supports pre-SPIFFE certs during the ≤2-year
///   renewal tail).
pub fn service_identity_from_der_with_trust_domain(
    der: &[u8],
    trust_domain: &str,
) -> Option<ServiceIdentity> {
    use const_oid::db::rfc5280::ID_CE_SUBJECT_ALT_NAME;
    use der::Decode;
    use x509_cert::Certificate;
    use x509_cert::ext::pkix::SubjectAltName;
    use x509_cert::ext::pkix::name::GeneralName;

    let cert = Certificate::from_der(der).ok()?;
    let cert_serial = cert.tbs_certificate.serial_number.to_string();

    // 1. SPIFFE SAN path.
    if let Some(exts) = &cert.tbs_certificate.extensions {
        for ext in exts {
            if ext.extn_id != ID_CE_SUBJECT_ALT_NAME {
                continue;
            }
            let Ok(san) = SubjectAltName::from_der(ext.extn_value.as_bytes()) else {
                continue;
            };
            for gn in san.0 {
                if let GeneralName::UniformResourceIdentifier(uri) = gn
                    && let Some(service_id) =
                        try_parse_spiffe_service_id(uri.as_str(), trust_domain)
                {
                    return Some(ServiceIdentity {
                        service_id,
                        cert_serial,
                    });
                }
            }
        }
    }

    // 2. CN fallback.
    tracing::debug!(
        "no SPIFFE SAN matched trust domain {trust_domain:?}; falling back to CN (migration tail)"
    );
    extract_cn_service_id(&cert).map(|service_id| ServiceIdentity {
        service_id,
        cert_serial,
    })
}

/// Axum extractor that deserialises the request body as JSON and immediately
/// validates it with [`Validate::validate()`].
///
/// Replaces the repetitive 3-line pattern:
/// ```rust,ignore
/// if let Err(e) = req.validate() {
///     return error_response(StatusCode::BAD_REQUEST, e.to_string());
/// }
/// ```
///
/// Usage — change the handler signature from:
/// ```rust,ignore
/// Json(req): Json<CreateFooRequest>
/// ```
/// to:
/// ```rust,ignore
/// Validated(req): Validated<CreateFooRequest>
/// ```
/// and remove the manual `validate()` call.
///
/// Returns `400 Bad Request` on JSON deserialisation failure or validation failure.
pub struct Validated<T>(pub T);

impl<T, S> FromRequest<S> for Validated<T>
where
    T: DeserializeOwned + Validate + Send,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(
        req: axum::http::Request<axum::body::Body>,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let axum::Json(body) = <axum::Json<T> as FromRequest<S>>::from_request(req, state)
            .await
            .map_err(|e| error_response(StatusCode::BAD_REQUEST, e.to_string()))?;
        body.validate()
            .map_err(|e| error_response(StatusCode::BAD_REQUEST, e.to_string()))?;
        Ok(Validated(body))
    }
}

/// A deserialized-but-not-yet-validated request body. The inner value is
/// private; the only way to reach the fields is [`Unvalidated::require_valid`].
pub struct Unvalidated<T>(T);

impl<T: Validate> Unvalidated<T> {
    pub fn require_valid(self) -> Result<T, ValidationError> {
        self.0.validate()?;
        Ok(self.0)
    }
}

impl<T, S> FromRequest<S> for Unvalidated<T>
where
    T: DeserializeOwned + Validate + Send,
    S: Send + Sync,
{
    type Rejection = <axum::Json<T> as FromRequest<S>>::Rejection;

    async fn from_request(
        req: axum::http::Request<axum::body::Body>,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let axum::Json(body) = <axum::Json<T> as FromRequest<S>>::from_request(req, state).await?;
        Ok(Unvalidated(body))
    }
}

impl<T, S> OptionalFromRequest<S> for Unvalidated<T>
where
    T: DeserializeOwned + Validate + Send,
    S: Send + Sync,
{
    type Rejection = <axum::Json<T> as OptionalFromRequest<S>>::Rejection;

    async fn from_request(
        req: axum::http::Request<axum::body::Body>,
        state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(
            <axum::Json<T> as OptionalFromRequest<S>>::from_request(req, state)
                .await?
                .map(|axum::Json(body)| Unvalidated(body)),
        )
    }
}

/// Form-borne counterpart of [`Unvalidated`].
pub struct UnvalidatedForm<T>(T);

impl<T: Validate> UnvalidatedForm<T> {
    pub fn require_valid(self) -> Result<T, ValidationError> {
        self.0.validate()?;
        Ok(self.0)
    }
}

impl<T, S> FromRequest<S> for UnvalidatedForm<T>
where
    T: DeserializeOwned + Validate + Send,
    S: Send + Sync,
{
    type Rejection = <Form<T> as FromRequest<S>>::Rejection;

    async fn from_request(
        req: axum::http::Request<axum::body::Body>,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let Form(body) = <Form<T> as FromRequest<S>>::from_request(req, state).await?;
        Ok(UnvalidatedForm(body))
    }
}

#[cfg(test)]
impl<T> Unvalidated<T> {
    /// Test-only: existing handler tests call handlers directly (no HTTP layer)
    /// and must construct a body value. Production code cannot reach this.
    pub(crate) fn new_for_test(value: T) -> Self {
        Unvalidated(value)
    }
}

/// Axum extractor that constructs a [`SessionService`] from the request state.
///
/// Requires the router state to implement `FromRef<DbState>` (satisfied by
/// `Arc<AppState>` via the [`FromRef`] impl in `app_state`). Inner field is
/// private; access the service via [`Deref`].
pub struct SessionSvc(SessionService);

impl SessionSvc {
    pub fn new(svc: SessionService) -> Self {
        Self(svc)
    }
}

impl Deref for SessionSvc {
    type Target = SessionService;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S> FromRequestParts<S> for SessionSvc
where
    DbState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(_parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let db_state = DbState::from_ref(state);
        Ok(SessionSvc(SessionService::new(db_state.db().clone())))
    }
}

/// Axum extractor that constructs an [`ApiTokenService`] from the request state.
///
/// Same mechanics as [`SessionSvc`] — requires `FromRef<DbState>` on the state.
pub struct ApiTokenSvc(ApiTokenService);

impl Deref for ApiTokenSvc {
    type Target = ApiTokenService;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S> FromRequestParts<S> for ApiTokenSvc
where
    DbState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(_parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let db_state = DbState::from_ref(state);
        Ok(ApiTokenSvc(ApiTokenService::new(db_state.db().clone())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_cn_rfc2253_with_org() {
        assert_eq!(
            extract_cn_from_dn("CN=550e8400-e29b-41d4-a716-446655440000,O=Uptrakit"),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn extract_cn_rfc2253_only() {
        assert_eq!(
            extract_cn_from_dn("CN=550e8400-e29b-41d4-a716-446655440000"),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn extract_cn_openssl_format() {
        assert_eq!(
            extract_cn_from_dn("/CN=550e8400-e29b-41d4-a716-446655440000/O=Uptrakit"),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn extract_cn_url_encoded() {
        assert_eq!(
            extract_cn_from_dn("CN%3D550e8400-e29b-41d4-a716-446655440000"),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn extract_cn_empty_returns_none() {
        assert_eq!(extract_cn_from_dn(""), None);
        assert_eq!(extract_cn_from_dn("O=Org"), None);
        assert_eq!(extract_cn_from_dn("/O=Org"), None);
    }

    #[test]
    fn normalize_serial_compact_hex() {
        assert_eq!(normalize_serial("01abcdef"), "01:AB:CD:EF");
    }

    #[test]
    fn normalize_serial_already_colon_separated_upper() {
        assert_eq!(normalize_serial("01:AB:CD:EF"), "01:AB:CD:EF");
    }

    #[test]
    fn normalize_serial_colon_separated_lower_uppercased() {
        assert_eq!(normalize_serial("01:ab:cd:ef"), "01:AB:CD:EF");
    }

    #[test]
    fn normalize_serial_single_byte() {
        assert_eq!(normalize_serial("0a"), "0A");
    }

    #[test]
    fn identity_from_info_with_serial() {
        let id = service_identity_from_info(
            "CN=550e8400-e29b-41d4-a716-446655440000,O=Uptrakit",
            Some("01abcdef"),
        )
        .expect("should parse");
        assert_eq!(
            id.service_id,
            uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
        );
        assert_eq!(id.cert_serial, "01:AB:CD:EF");
    }

    #[test]
    fn identity_from_info_without_serial() {
        let id = service_identity_from_info("CN=550e8400-e29b-41d4-a716-446655440000", None)
            .expect("should parse");
        assert_eq!(
            id.service_id,
            uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
        );
        assert_eq!(id.cert_serial, "");
    }

    #[test]
    fn identity_from_info_non_uuid_cn() {
        assert!(service_identity_from_info("CN=not-a-uuid,O=Org", Some("01")).is_none());
    }

    #[test]
    fn identity_from_der_valid_cert() {
        // Generate a test certificate with a UUID CN
        let service_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("keygen");
        let mut params = rcgen::CertificateParams::new(vec![]).expect("cert params");
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, service_id.to_string());
        let cert = params.self_signed(&key_pair).expect("self-sign");

        let identity = service_identity_from_der(cert.der()).expect("should parse DER cert");
        assert_eq!(identity.service_id, service_id);
        assert!(!identity.cert_serial.is_empty());
    }

    #[test]
    fn identity_from_der_non_uuid_cn() {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("keygen");
        let mut params = rcgen::CertificateParams::new(vec![]).expect("cert params");
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "not-a-uuid");
        let cert = params.self_signed(&key_pair).expect("self-sign");

        assert!(service_identity_from_der(cert.der()).is_none());
    }
}

#[cfg(test)]
mod spiffe_tests {
    use super::service_identity_from_der_with_trust_domain;

    fn make_cert_with_spiffe_san(service_id: uuid::Uuid, trust_domain: &str) -> Vec<u8> {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).unwrap();
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, service_id.to_string());
        params.subject_alt_names.push(rcgen::SanType::URI(
            format!("spiffe://{trust_domain}/service/{service_id}")
                .as_str()
                .try_into()
                .unwrap(),
        ));
        params.self_signed(&key).unwrap().der().to_vec()
    }

    fn make_cert_cn_only(service_id: uuid::Uuid) -> Vec<u8> {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).unwrap();
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, service_id.to_string());
        params.self_signed(&key).unwrap().der().to_vec()
    }

    fn make_cert_non_spiffe_uri_san_no_uuid_cn() -> Vec<u8> {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).unwrap();
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "not-a-uuid");
        params.subject_alt_names.push(rcgen::SanType::URI(
            "https://example.com".try_into().unwrap(),
        ));
        params.self_signed(&key).unwrap().der().to_vec()
    }

    fn make_cert_non_spiffe_uri_san_with_uuid_cn(service_id: uuid::Uuid) -> Vec<u8> {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).unwrap();
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, service_id.to_string());
        params.subject_alt_names.push(rcgen::SanType::URI(
            "https://example.com".try_into().unwrap(),
        ));
        params.self_signed(&key).unwrap().der().to_vec()
    }

    const TRUST_DOMAIN: &str = "example.org";
    const SERVICE_ID: uuid::Uuid = uuid::uuid!("550e8400-e29b-41d4-a716-446655440000");

    #[test]
    fn spiffe_san_preferred_over_cn() {
        let der = make_cert_with_spiffe_san(SERVICE_ID, TRUST_DOMAIN);
        let identity = service_identity_from_der_with_trust_domain(&der, TRUST_DOMAIN)
            .expect("should parse identity from SPIFFE SAN");
        assert_eq!(identity.service_id, SERVICE_ID);
        assert!(!identity.cert_serial.is_empty());
    }

    #[test]
    fn cn_fallback_when_no_spiffe_san() {
        let der = make_cert_cn_only(SERVICE_ID);
        let identity = service_identity_from_der_with_trust_domain(&der, TRUST_DOMAIN)
            .expect("should parse identity from CN");
        assert_eq!(identity.service_id, SERVICE_ID);
    }

    #[test]
    fn wrong_trust_domain_falls_back_to_cn() {
        let der = make_cert_with_spiffe_san(SERVICE_ID, "wrong.domain");
        let identity = service_identity_from_der_with_trust_domain(&der, TRUST_DOMAIN)
            .expect("should fall back to CN when trust domain does not match");
        assert_eq!(identity.service_id, SERVICE_ID);
    }

    #[test]
    fn non_spiffe_uri_san_no_uuid_cn_returns_none() {
        let der = make_cert_non_spiffe_uri_san_no_uuid_cn();
        let result = service_identity_from_der_with_trust_domain(&der, TRUST_DOMAIN);
        assert!(result.is_none());
    }

    #[test]
    fn non_spiffe_uri_san_falls_back_to_cn() {
        let der = make_cert_non_spiffe_uri_san_with_uuid_cn(SERVICE_ID);
        let identity = service_identity_from_der_with_trust_domain(&der, TRUST_DOMAIN)
            .expect("should fall back to CN when URI SAN is not SPIFFE");
        assert_eq!(identity.service_id, SERVICE_ID);
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod service_extractor_tests {
    use super::*;
    use axum::extract::FromRequestParts;
    use axum::http::Request;

    #[tokio::test]
    async fn session_svc_extracts_from_app_state() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let mut parts = Request::builder().body(()).unwrap().into_parts().0;
        let svc = SessionSvc::from_request_parts(&mut parts, &state)
            .await
            .unwrap();
        let _: &SessionService = &svc;
    }

    #[tokio::test]
    async fn api_token_svc_extracts_from_app_state() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let mut parts = Request::builder().body(()).unwrap().into_parts().0;
        let svc = ApiTokenSvc::from_request_parts(&mut parts, &state)
            .await
            .unwrap();
        let _: &ApiTokenService = &svc;
    }
}

#[cfg(test)]
mod unvalidated_tests {
    use super::{Unvalidated, UnvalidatedForm, Validate, ValidationError};

    struct Probe {
        ok: bool,
    }

    impl Validate for Probe {
        fn validate(&self) -> Result<(), ValidationError> {
            if self.ok {
                Ok(())
            } else {
                Err(ValidationError {
                    field: "ok",
                    message: "must be true".to_string(),
                })
            }
        }
    }

    #[test]
    fn require_valid_returns_inner_on_valid_body() {
        let body = Unvalidated::new_for_test(Probe { ok: true });
        body.require_valid().unwrap();
    }

    #[test]
    fn require_valid_errors_on_invalid_body() {
        let body = Unvalidated::new_for_test(Probe { ok: false });
        assert_eq!(body.require_valid().err().map(|e| e.field), Some("ok"));
    }

    #[test]
    fn form_require_valid_errors_on_invalid_body() {
        let body = UnvalidatedForm(Probe { ok: false });
        assert_eq!(body.require_valid().err().map(|e| e.field), Some("ok"));
    }
}
