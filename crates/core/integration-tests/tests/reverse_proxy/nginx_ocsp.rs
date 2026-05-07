#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "integration test infrastructure: panics are acceptable in reverse-proxy test helpers"
)]

use std::net::TcpListener;
use std::sync::OnceLock;

use tempfile::TempDir;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{AccessMode, Host, IntoContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

use super::ocsp_responder::OcspResponder;
use super::pki::{TestPki, extract_serial_hex};
use super::server::TestServer;

// ---------------------------------------------------------------------------
// Test 1: Explicit ssl_ocsp_responder with HTTP URL (existing behaviour)
// ---------------------------------------------------------------------------

/// Nginx OCSP integration test — explicit HTTP responder.
///
/// Validates that Nginx queries an explicit `ssl_ocsp_responder http://…`
/// endpoint and rejects revoked client certificates while accepting valid ones.
///
/// Requires Docker and `nginx:latest`.
#[tokio::test]
#[ignore = "Docker integration test (nginx:latest). Run: cargo test -p uptrakit-integration-tests --test reverse_proxy nginx_ocsp_rejects_revoked_cert -- --ignored"]
async fn nginx_ocsp_rejects_revoked_cert() {
    let pki = TestPki::generate();
    let (revoked_cert_pem, revoked_key_pem, _revoked_id) = pki.generate_extra_agent_cert();
    let revoked_serial = extract_serial_hex(&revoked_cert_pem);
    let host_gateway_ip = resolve_docker_host_gateway_ip();

    // Start test OCSP responder (HTTP)
    let ocsp = OcspResponder::start(&pki.ca_cert_pem, &pki.ca_key_pem, vec![revoked_serial]).await;

    // Start backend test server
    let server = TestServer::start(&pki, Some("X-Forwarded-Client-Cert-Info"), None).await;

    // Write Nginx config with explicit HTTP OCSP responder
    let tmp = TempDir::new().expect("tempdir");
    write_common_nginx_tls_files(&tmp, &pki);
    write_nginx_ocsp_config(&tmp, host_gateway_ip, server.port, ocsp.port());

    let container = start_nginx_ocsp_container(&tmp).await;
    let proxy_port = get_nginx_port(&container).await;

    // Give Nginx a moment to initialize OCSP
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let client_no_cert = build_client(None, None, &pki);
    let client_valid_cert = build_client(Some(&pki.agent_cert_pem), Some(&pki.agent_key_pem), &pki);
    let client_revoked_cert = build_client(Some(&revoked_cert_pem), Some(&revoked_key_pem), &pki);

    // No cert → healthz should succeed
    let resp = client_no_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await
        .expect("healthz request");
    assert_status(
        &container,
        resp.status(),
        reqwest::StatusCode::OK,
        "no-cert healthz",
    )
    .await;

    // Valid cert → should be accepted
    let resp = client_valid_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await
        .expect("valid cert request");
    assert_status(
        &container,
        resp.status(),
        reqwest::StatusCode::OK,
        "valid cert",
    )
    .await;

    // Revoked cert → should be rejected by Nginx OCSP check.
    //
    // Nginx's ssl_ocsp performs the OCSP fetch asynchronously on first encounter
    // per cert serial: before the fetch completes and the result is cached, nginx
    // may fail-open and return 200. Retry to give nginx time to complete the
    // fetch, cache "revoked", and enforce it on the next request. We additionally
    // require ocsp.request_count() > 0 before treating a rejection as success —
    // a 400 without any OCSP query means nginx rejected the cert for a structural
    // reason rather than revocation, which would be a test infrastructure bug.
    const MAX_RETRIES: usize = 5;
    let mut revoked_rejected = false;
    for attempt in 0..MAX_RETRIES {
        let result = client_revoked_cert
            .get(format!("https://localhost:{proxy_port}/healthz"))
            .send()
            .await;

        match result {
            Ok(resp)
                if (resp.status() == reqwest::StatusCode::BAD_REQUEST
                    || resp.status() == reqwest::StatusCode::FORBIDDEN)
                    && ocsp.request_count() > 0 =>
            {
                revoked_rejected = true;
                break;
            }
            Err(e) if (e.is_connect() || e.is_request()) && ocsp.request_count() > 0 => {
                revoked_rejected = true;
                break;
            }
            Ok(resp) if attempt + 1 < MAX_RETRIES => {
                tracing::debug!(
                    attempt,
                    status = %resp.status(),
                    ocsp_requests = ocsp.request_count(),
                    "revoked cert not yet rejected via OCSP; retrying after 1s"
                );
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            Ok(resp) => {
                let logs = get_nginx_logs(&container);
                panic!(
                    "revoked cert should be rejected, got status {} after {MAX_RETRIES} attempts \
                     (OCSP requests: {})\nNginx logs:\n{logs}",
                    resp.status(),
                    ocsp.request_count()
                );
            }
            Err(e) => panic!("unexpected error for revoked cert: {e}"),
        }
    }
    assert!(
        revoked_rejected,
        "revoked cert not rejected after {MAX_RETRIES} retries"
    );

    server.shutdown();
    ocsp.shutdown();
}

// ---------------------------------------------------------------------------
// Test 2: AIA-embedded HTTP OCSP URL (no explicit responder)
// ---------------------------------------------------------------------------

/// Nginx OCSP integration test — AIA HTTP responder.
///
/// Validates that Nginx reads the OCSP responder URL from the client
/// certificate's AIA extension (HTTP) and rejects revoked certificates
/// without needing an explicit `ssl_ocsp_responder` directive.
///
/// Requires Docker and `nginx:latest`.
#[tokio::test]
#[ignore = "Docker integration test (nginx:latest). Run: cargo test -p uptrakit-integration-tests --test reverse_proxy nginx_ocsp_aia_http_rejects_revoked_cert -- --ignored"]
async fn nginx_ocsp_aia_http_rejects_revoked_cert() {
    let pki = TestPki::generate();
    let host_gateway_ip = resolve_docker_host_gateway_ip();

    // Bind the listener before generating certs so the port is embedded in the
    // AIA extension while the OS socket is still held. This avoids the TOCTOU
    // window where another process could claim the port between dropping the
    // listener and the responder rebinding to the same port.
    let ocsp_listener = TcpListener::bind("0.0.0.0:0").expect("bind OCSP listener");
    let ocsp_port = ocsp_listener.local_addr().expect("local addr").port();

    let aia_url = format!("http://{host_gateway_ip}:{ocsp_port}/api/v1/pki/ocsp");

    // Generate agent certs with AIA extension pointing to HTTP OCSP.
    let (valid_cert_pem, valid_key_pem, _valid_id) =
        pki.generate_extra_agent_cert_with_aia(&aia_url);
    let (revoked_cert_pem, revoked_key_pem, _revoked_id) =
        pki.generate_extra_agent_cert_with_aia(&aia_url);
    let revoked_serial = extract_serial_hex(&revoked_cert_pem);

    let ocsp = OcspResponder::start_http_with_listener(
        ocsp_listener,
        &pki.ca_cert_pem,
        &pki.ca_key_pem,
        vec![revoked_serial],
    )
    .await;

    let server = TestServer::start(&pki, Some("X-Forwarded-Client-Cert-Info"), None).await;

    let tmp = TempDir::new().expect("tempdir");
    write_common_nginx_tls_files(&tmp, &pki);
    write_nginx_ocsp_aia_config(&tmp, host_gateway_ip, server.port);

    let container = start_nginx_ocsp_container(&tmp).await;
    let proxy_port = get_nginx_port(&container).await;

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let client_no_cert = build_client(None, None, &pki);
    let client_valid_cert = build_client(Some(&valid_cert_pem), Some(&valid_key_pem), &pki);
    let client_revoked_cert = build_client(Some(&revoked_cert_pem), Some(&revoked_key_pem), &pki);

    // No cert → healthz should succeed
    let resp = client_no_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await
        .expect("healthz request");
    assert_status(
        &container,
        resp.status(),
        reqwest::StatusCode::OK,
        "no-cert healthz",
    )
    .await;

    // Valid cert → should be accepted
    let resp = client_valid_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await
        .expect("valid cert request");
    assert_status(
        &container,
        resp.status(),
        reqwest::StatusCode::OK,
        "valid cert",
    )
    .await;

    // Revoked cert → should be rejected (Nginx reads AIA, queries HTTP OCSP).
    //
    // Nginx's ssl_ocsp performs the OCSP fetch asynchronously on first encounter
    // per cert serial: before the fetch completes and the result is cached, nginx
    // may fail-open and return 200. Retry to give nginx time to complete the
    // fetch, cache "revoked", and enforce it on the next request. We additionally
    // require ocsp.request_count() > 0 before treating a rejection as success —
    // a 400 without any OCSP query means nginx rejected the cert for a structural
    // reason rather than revocation, which would be a test infrastructure bug.
    const MAX_RETRIES: usize = 5;
    let mut revoked_rejected = false;
    for attempt in 0..MAX_RETRIES {
        let result = client_revoked_cert
            .get(format!("https://localhost:{proxy_port}/healthz"))
            .send()
            .await;

        match result {
            Ok(resp)
                if (resp.status() == reqwest::StatusCode::BAD_REQUEST
                    || resp.status() == reqwest::StatusCode::FORBIDDEN)
                    && ocsp.request_count() > 0 =>
            {
                revoked_rejected = true;
                break;
            }
            Err(e) if (e.is_connect() || e.is_request()) && ocsp.request_count() > 0 => {
                revoked_rejected = true;
                break;
            }
            Ok(resp) if attempt + 1 < MAX_RETRIES => {
                tracing::debug!(
                    attempt,
                    status = %resp.status(),
                    ocsp_requests = ocsp.request_count(),
                    "revoked cert not yet rejected via AIA HTTP OCSP; retrying after 1s"
                );
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            Ok(resp) => {
                let logs = get_nginx_logs(&container);
                panic!(
                    "revoked cert should be rejected via AIA HTTP OCSP, got status {} after \
                     {MAX_RETRIES} attempts (OCSP requests: {})\nNginx logs:\n{logs}",
                    resp.status(),
                    ocsp.request_count()
                );
            }
            Err(e) => panic!("unexpected error for revoked cert: {e}"),
        }
    }
    assert!(
        revoked_rejected,
        "revoked cert not rejected after {MAX_RETRIES} retries"
    );

    server.shutdown();
    ocsp.shutdown();
}

// ---------------------------------------------------------------------------
// Test 3: AIA-embedded HTTPS OCSP URL — documents Nginx HTTP-only limitation
// ---------------------------------------------------------------------------

/// Nginx OCSP integration test — AIA HTTPS responder (documents failure).
///
/// Proves that Nginx rejects client certificates when the AIA extension
/// contains an `https://` OCSP responder URL. Nginx only supports `http://`
/// for OCSP — it reports "invalid URL prefix in OCSP responder" during the
/// SSL handshake and returns 400 for any client cert with an HTTPS AIA URL.
///
/// Requires Docker and `nginx:latest`.
#[tokio::test]
#[ignore = "Docker integration test (nginx:latest). Run: cargo test -p uptrakit-integration-tests --test reverse_proxy nginx_ocsp_aia_https_cannot_verify -- --ignored"]
async fn nginx_ocsp_aia_https_cannot_verify() {
    let pki = TestPki::generate();
    let host_gateway_ip = resolve_docker_host_gateway_ip();

    // Bind the listener before generating certs — same TOCTOU fix as the HTTP
    // AIA test: the port is embedded in the cert while the socket is still held.
    let ocsp_listener = TcpListener::bind("0.0.0.0:0").expect("bind HTTPS OCSP listener");
    let ocsp_port = ocsp_listener.local_addr().expect("local addr").port();

    let aia_url = format!("https://{host_gateway_ip}:{ocsp_port}/api/v1/pki/ocsp");

    let (valid_cert_pem, valid_key_pem, _valid_id) =
        pki.generate_extra_agent_cert_with_aia(&aia_url);
    let (revoked_cert_pem, revoked_key_pem, _revoked_id) =
        pki.generate_extra_agent_cert_with_aia(&aia_url);
    let revoked_serial = extract_serial_hex(&revoked_cert_pem);

    let ocsp = OcspResponder::start_https_with_listener(
        ocsp_listener,
        &pki.ca_cert_pem,
        &pki.ca_key_pem,
        &pki.server_cert_pem,
        &pki.server_key_pem,
        vec![revoked_serial],
    )
    .await;

    // Verify HTTPS responder is reachable from test host.
    let healthz_client = reqwest::Client::builder()
        .tls_certs_merge([
            reqwest::Certificate::from_pem(pki.ca_cert_pem.as_bytes()).expect("CA cert")
        ])
        .build()
        .expect("healthz client");
    let healthz_resp = healthz_client
        .get(format!("https://localhost:{ocsp_port}/healthz"))
        .send()
        .await
        .expect("HTTPS OCSP healthz request");
    assert_eq!(
        healthz_resp.status(),
        reqwest::StatusCode::OK,
        "HTTPS OCSP responder should be reachable"
    );

    let server = TestServer::start(&pki, Some("X-Forwarded-Client-Cert-Info"), None).await;

    let tmp = TempDir::new().expect("tempdir");
    write_common_nginx_tls_files(&tmp, &pki);
    write_nginx_ocsp_aia_config(&tmp, host_gateway_ip, server.port);

    let container = start_nginx_ocsp_container(&tmp).await;
    let proxy_port = get_nginx_port(&container).await;

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let client_no_cert = build_client(None, None, &pki);
    let client_valid_cert = build_client(Some(&valid_cert_pem), Some(&valid_key_pem), &pki);
    let client_revoked_cert = build_client(Some(&revoked_cert_pem), Some(&revoked_key_pem), &pki);

    // No cert → healthz should still succeed (OCSP only affects client certs)
    let resp = client_no_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await
        .expect("healthz request");
    assert_status(
        &container,
        resp.status(),
        reqwest::StatusCode::OK,
        "no-cert healthz",
    )
    .await;

    // Valid cert → rejected because Nginx cannot use https:// AIA OCSP URLs.
    // Nginx reports "invalid URL prefix in OCSP responder" during SSL handshake.
    let resp = client_valid_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await
        .expect("valid cert request (should be rejected — Nginx rejects HTTPS AIA URL)");
    assert!(
        resp.status() == reqwest::StatusCode::BAD_REQUEST
            || resp.status() == reqwest::StatusCode::FORBIDDEN,
        "valid cert should be REJECTED because Nginx rejects https:// AIA OCSP URLs, got {}",
        resp.status()
    );

    // Revoked cert → also rejected for the same reason (invalid HTTPS AIA URL).
    let result = client_revoked_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await;
    match result {
        Ok(resp) => {
            assert!(
                resp.status() == reqwest::StatusCode::BAD_REQUEST
                    || resp.status() == reqwest::StatusCode::FORBIDDEN,
                "revoked cert should be REJECTED (HTTPS AIA URL), got {}",
                resp.status()
            );
        }
        Err(e) => {
            assert!(
                e.is_connect() || e.is_request(),
                "expected connection error for revoked cert, got: {e}"
            );
        }
    }

    // OCSP responder should NOT have received any requests.
    assert_eq!(
        ocsp.request_count(),
        0,
        "OCSP responder should NOT have received requests — Nginx rejects https:// AIA URLs"
    );

    server.shutdown();
    ocsp.shutdown();
}

// ---------------------------------------------------------------------------
// Test 4: Explicit ssl_ocsp_responder with HTTPS URL — Nginx rejects config
// ---------------------------------------------------------------------------

/// Nginx OCSP integration test — explicit HTTPS responder rejected at config
/// parse time.
///
/// Proves that Nginx rejects `https://` URLs in the `ssl_ocsp_responder`
/// directive with an "invalid URL prefix" error during configuration
/// validation. Unlike AIA HTTPS URLs (which are silently ignored), explicit
/// HTTPS responder URLs cause a hard failure at startup.
///
/// Requires Docker and `nginx:latest`.
#[tokio::test]
#[ignore = "Docker integration test (nginx:latest). Run: cargo test -p uptrakit-integration-tests --test reverse_proxy nginx_ocsp_rejects_https_ssl_ocsp_responder -- --ignored"]
async fn nginx_ocsp_rejects_https_ssl_ocsp_responder() {
    let pki = TestPki::generate();
    let host_gateway_ip = resolve_docker_host_gateway_ip();

    let tmp = TempDir::new().expect("tempdir");
    write_common_nginx_tls_files(&tmp, &pki);
    // Port values don't matter — Nginx rejects the config before connecting.
    write_nginx_ocsp_explicit_https_config(&tmp, host_gateway_ip, 8443, 9443);

    // Run `nginx -t` to test the config — should fail because Nginx rejects
    // https:// in ssl_ocsp_responder at parse time.
    let output = tokio::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/etc/nginx/conf.d:ro", tmp.path().display()),
            "nginx:latest",
            "nginx",
            "-t",
        ])
        .output()
        .await
        .expect("docker run nginx -t");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "nginx -t should fail with https:// ssl_ocsp_responder, but succeeded.\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("invalid URL prefix"),
        "Nginx should report \"invalid URL prefix\" for https:// ssl_ocsp_responder.\nstderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Write CA cert, server cert, and server key files to the Nginx config dir.
fn write_common_nginx_tls_files(tmp: &TempDir, pki: &TestPki) {
    std::fs::write(tmp.path().join("ca.crt"), &pki.ca_cert_pem).expect("write ca.crt");
    std::fs::write(tmp.path().join("server.crt"), &pki.server_cert_pem).expect("write server.crt");
    std::fs::write(tmp.path().join("server.key"), &pki.server_key_pem).expect("write server.key");
}

/// Write Nginx config with explicit HTTP OCSP responder.
fn write_nginx_ocsp_config(
    tmp: &TempDir,
    host_gateway_ip: &str,
    backend_port: u16,
    ocsp_port: u16,
) {
    let config = format!(
        r#"
server {{
    listen 443 ssl;
    server_name localhost;

    ssl_certificate     /etc/nginx/conf.d/server.crt;
    ssl_certificate_key /etc/nginx/conf.d/server.key;

    ssl_client_certificate /etc/nginx/conf.d/ca.crt;
    ssl_verify_client optional;

    # OCSP checking for client certificates
    ssl_ocsp leaf;
    ssl_ocsp_cache shared:OCSP:10m;
    ssl_ocsp_responder http://{host_gateway_ip}:{ocsp_port}/;

    location / {{
        proxy_pass https://{host_gateway_ip}:{backend_port};
        proxy_ssl_verify off;

        proxy_set_header X-Forwarded-Client-Cert-Info "Subject=\"$ssl_client_s_dn\";SerialNumber=\"$ssl_client_serial\";Issuer=\"$ssl_client_i_dn\"";
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header X-Forwarded-Host $host;
        proxy_set_header Host $host;

        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }}
}}
"#
    );
    std::fs::write(tmp.path().join("default.conf"), config).expect("write nginx config");
}

/// Write Nginx config with `ssl_ocsp leaf` only (no explicit responder).
///
/// Nginx reads the OCSP responder URL from the client certificate's AIA extension.
fn write_nginx_ocsp_aia_config(tmp: &TempDir, host_gateway_ip: &str, backend_port: u16) {
    let config = format!(
        r#"
server {{
    listen 443 ssl;
    server_name localhost;

    ssl_certificate     /etc/nginx/conf.d/server.crt;
    ssl_certificate_key /etc/nginx/conf.d/server.key;

    ssl_client_certificate /etc/nginx/conf.d/ca.crt;
    ssl_verify_client optional;

    # OCSP checking — no explicit responder; Nginx reads AIA from client cert
    ssl_ocsp leaf;
    ssl_ocsp_cache shared:OCSP:10m;

    location / {{
        proxy_pass https://{host_gateway_ip}:{backend_port};
        proxy_ssl_verify off;

        proxy_set_header X-Forwarded-Client-Cert-Info "Subject=\"$ssl_client_s_dn\";SerialNumber=\"$ssl_client_serial\";Issuer=\"$ssl_client_i_dn\"";
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header X-Forwarded-Host $host;
        proxy_set_header Host $host;

        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }}
}}
"#
    );
    std::fs::write(tmp.path().join("default.conf"), config).expect("write nginx AIA config");
}

/// Write Nginx config with explicit HTTPS OCSP responder.
///
/// Nginx rejects `https://` in `ssl_ocsp_responder` at config parse time.
/// This config is used by test 4 to verify Nginx's config validation.
fn write_nginx_ocsp_explicit_https_config(
    tmp: &TempDir,
    host_gateway_ip: &str,
    backend_port: u16,
    ocsp_port: u16,
) {
    let config = format!(
        r#"
server {{
    listen 443 ssl;
    server_name localhost;

    ssl_certificate     /etc/nginx/conf.d/server.crt;
    ssl_certificate_key /etc/nginx/conf.d/server.key;

    ssl_client_certificate /etc/nginx/conf.d/ca.crt;
    ssl_verify_client optional;

    # OCSP checking with https:// responder — Nginx rejects this at parse time.
    ssl_ocsp leaf;
    ssl_ocsp_responder https://{host_gateway_ip}:{ocsp_port}/;

    # Resolver needed for ssl_ocsp
    resolver 127.0.0.11 ipv6=off;

    location / {{
        proxy_pass https://{host_gateway_ip}:{backend_port};
        proxy_ssl_verify off;

        proxy_set_header X-Forwarded-Client-Cert-Info "Subject=\"$ssl_client_s_dn\";SerialNumber=\"$ssl_client_serial\";Issuer=\"$ssl_client_i_dn\"";
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header X-Forwarded-Host $host;
        proxy_set_header Host $host;

        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }}
}}
"#
    );
    std::fs::write(tmp.path().join("default.conf"), config).expect("write nginx HTTPS OCSP config");
}

fn build_client(
    cert_pem: Option<&str>,
    key_pem: Option<&str>,
    ca_pki: &TestPki,
) -> reqwest::Client {
    let ca_cert = reqwest::Certificate::from_pem(ca_pki.ca_cert_pem.as_bytes()).expect("CA cert");

    let mut builder = reqwest::Client::builder().tls_certs_merge([ca_cert]);

    if let (Some(cert), Some(key)) = (cert_pem, key_pem) {
        let mut id_pem = cert.as_bytes().to_vec();
        id_pem.extend_from_slice(key.as_bytes());
        let identity = reqwest::Identity::from_pem(&id_pem).expect("client identity");
        builder = builder.identity(identity);
    }

    builder.build().expect("reqwest client")
}

/// Start an Nginx container for OCSP tests.
///
/// The container still maps `host.docker.internal` to Docker's host gateway
/// for consistency with other reverse-proxy tests, but OCSP configs in this
/// file use the resolved gateway IP directly to avoid DNS-dependent failures.
async fn start_nginx_ocsp_container(tmp: &TempDir) -> testcontainers::ContainerAsync<GenericImage> {
    GenericImage::new("nginx", "latest")
        .with_exposed_port(443u16.tcp())
        .with_wait_for(WaitFor::Log(LogWaitStrategy::stderr("start worker")))
        .with_mount(
            Mount::bind_mount(
                tmp.path().to_str().expect("tmpdir path"),
                "/etc/nginx/conf.d",
            )
            .with_access_mode(AccessMode::ReadOnly),
        )
        .with_host("host.docker.internal", Host::HostGateway)
        .start()
        .await
        .expect("start nginx OCSP container")
}

/// Resolve Docker's host-gateway IP as seen from containers.
///
/// Uses a short-lived container with `--add-host host.docker.internal:host-gateway`
/// and reads `/etc/hosts` so OCSP URLs can use an IP literal instead of DNS.
fn resolve_docker_host_gateway_ip() -> &'static str {
    static DOCKER_HOST_GATEWAY_IP: OnceLock<String> = OnceLock::new();

    DOCKER_HOST_GATEWAY_IP.get_or_init(|| {
        let output = std::process::Command::new("docker")
            .args([
                "run",
                "--rm",
                "--add-host",
                "host.docker.internal:host-gateway",
                "nginx:latest",
                "/bin/sh",
                "-c",
                "awk '/host\\.docker\\.internal/ {print $1; exit}' /etc/hosts",
            ])
            .output()
            .expect("resolve docker host gateway IP");

        assert!(
            output.status.success(),
            "failed to resolve docker host gateway IP.\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let ip = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert!(
            !ip.is_empty(),
            "docker host gateway IP resolution returned empty output"
        );
        ip
    })
}

async fn get_nginx_port(container: &testcontainers::ContainerAsync<GenericImage>) -> u16 {
    container
        .get_host_port_ipv4(443u16.tcp())
        .await
        .expect("get nginx mapped port")
}

/// Assert an HTTP status code, printing Nginx container logs on failure.
async fn assert_status(
    container: &testcontainers::ContainerAsync<GenericImage>,
    actual: reqwest::StatusCode,
    expected: reqwest::StatusCode,
    label: &str,
) {
    if actual != expected {
        let logs = get_nginx_logs(container);
        panic!("{label}: expected HTTP {expected}, got {actual}.\nNginx logs:\n{logs}",);
    }
}

/// Retrieve Nginx container logs (stdout + stderr) for diagnostics.
fn get_nginx_logs(container: &testcontainers::ContainerAsync<GenericImage>) -> String {
    let output = std::process::Command::new("docker")
        .args(["logs", container.id()])
        .output()
        .expect("docker logs");
    format!(
        "--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
