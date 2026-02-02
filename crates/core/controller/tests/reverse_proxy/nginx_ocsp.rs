use tempfile::TempDir;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{AccessMode, Host, IntoContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

use super::ocsp_responder::OcspResponder;
use super::pki::{TestPki, extract_serial_hex};
use super::server::TestServer;

/// Nginx OCSP integration test.
///
/// Validates that Nginx queries the OCSP responder and rejects revoked
/// client certificates while accepting valid ones.
///
/// Requires Docker and `nginx:latest`.
#[tokio::test]
#[ignore = "Docker integration test (nginx:latest). Run: cargo test -p uptrakit-controller reverse_proxy::nginx_ocsp -- --ignored"]
async fn nginx_ocsp_rejects_revoked_cert() {
    let pki = TestPki::generate();
    let (revoked_cert_pem, revoked_key_pem, _revoked_id) = pki.generate_extra_agent_cert();
    let revoked_serial = extract_serial_hex(&revoked_cert_pem);

    // Start test OCSP responder
    let ocsp = OcspResponder::start(&pki.ca_cert_pem, &pki.ca_key_pem, vec![revoked_serial]).await;

    // Start backend test server
    let server = TestServer::start(&pki, Some("X-Forwarded-Client-Cert-Info"), None).await;

    // Write Nginx config with OCSP stapling
    let tmp = TempDir::new().expect("tempdir");
    write_nginx_ocsp_config(&tmp, &pki, server.port, ocsp.port());

    let container = GenericImage::new("nginx", "latest")
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
        .expect("start nginx container");

    let proxy_port = container
        .get_host_port_ipv4(443u16.tcp())
        .await
        .expect("get nginx mapped port");

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
    assert_eq!(resp.status(), 200);

    // Valid cert → should be accepted
    let resp = client_valid_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await
        .expect("valid cert request");
    assert_eq!(resp.status(), 200);

    // Revoked cert → should be rejected by Nginx OCSP check.
    // Nginx returns 400 "The SSL certificate error" when OCSP says revoked.
    let result = client_revoked_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await;

    match result {
        Ok(resp) => {
            assert!(
                resp.status() == 400 || resp.status() == 403,
                "revoked cert should be rejected, got status {}",
                resp.status()
            );
        }
        Err(e) => {
            // Connection reset or TLS error is also acceptable
            assert!(
                e.is_connect() || e.is_request(),
                "expected connection error for revoked cert, got: {e}"
            );
        }
    }

    // OCSP responder should have received at least one request
    assert!(
        ocsp.request_count() > 0,
        "OCSP responder should have received at least one request"
    );

    server.shutdown();
    ocsp.shutdown();
}

fn write_nginx_ocsp_config(tmp: &TempDir, pki: &TestPki, backend_port: u16, ocsp_port: u16) {
    std::fs::write(tmp.path().join("ca.crt"), &pki.ca_cert_pem).expect("write ca.crt");
    std::fs::write(tmp.path().join("server.crt"), &pki.server_cert_pem).expect("write server.crt");
    std::fs::write(tmp.path().join("server.key"), &pki.server_key_pem).expect("write server.key");

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
    ssl_ocsp_responder http://host.docker.internal:{ocsp_port}/;

    # Resolver needed for ssl_ocsp
    resolver 127.0.0.11 ipv6=off;

    location / {{
        proxy_pass https://host.docker.internal:{backend_port};
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

fn build_client(
    cert_pem: Option<&str>,
    key_pem: Option<&str>,
    ca_pki: &TestPki,
) -> reqwest::Client {
    let ca_cert = reqwest::Certificate::from_pem(ca_pki.ca_cert_pem.as_bytes()).expect("CA cert");

    let mut builder = reqwest::Client::builder()
        .add_root_certificate(ca_cert)
        .danger_accept_invalid_certs(false);

    if let (Some(cert), Some(key)) = (cert_pem, key_pem) {
        let mut id_pem = cert.as_bytes().to_vec();
        id_pem.extend_from_slice(key.as_bytes());
        let identity = reqwest::Identity::from_pem(&id_pem).expect("client identity");
        builder = builder.identity(identity);
    }

    builder.build().expect("reqwest client")
}
