use tempfile::TempDir;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{AccessMode, Host, IntoContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

use super::pki::{TestPki, extract_serial_hex};
use super::server::TestServer;

/// HAProxy CRL revocation checking integration test.
///
/// Validates that HAProxy rejects client certificates that appear in a CRL
/// while still accepting valid (non-revoked) certificates.
#[tokio::test]
#[ignore = "Docker integration test (haproxy:latest). Run: cargo test -p uptrakit-integration-tests --test reverse_proxy haproxy_crl -- --ignored"]
async fn haproxy_crl_rejects_revoked_cert() {
    let pki = TestPki::generate();
    let (revoked_cert_pem, revoked_key_pem, _revoked_id) = pki.generate_extra_agent_cert();

    // Generate CRL containing the revoked cert's serial
    let revoked_serial = extract_serial_hex(&revoked_cert_pem);
    let crl_pem = pki.generate_crl_pem(&[&revoked_serial]);

    let server = TestServer::start(&pki, Some("X-Forwarded-Client-Cert-Info"), None).await;

    let tmp = TempDir::new().expect("tempdir");
    write_haproxy_crl_config(&tmp, &pki, &crl_pem, server.port);

    let container = GenericImage::new("haproxy", "latest")
        .with_exposed_port(443u16.tcp())
        .with_wait_for(WaitFor::Log(LogWaitStrategy::stderr("Loading success")))
        .with_mount(
            Mount::bind_mount(
                tmp.path().join("haproxy.cfg").to_str().expect("cfg path"),
                "/usr/local/etc/haproxy/haproxy.cfg",
            )
            .with_access_mode(AccessMode::ReadOnly),
        )
        .with_mount(
            Mount::bind_mount(
                tmp.path().join("ssl").to_str().expect("ssl path"),
                "/etc/haproxy/ssl",
            )
            .with_access_mode(AccessMode::ReadOnly),
        )
        .with_host("host.docker.internal", Host::HostGateway)
        .start()
        .await
        .expect("start haproxy container");

    let proxy_port = container
        .get_host_port_ipv4(443u16.tcp())
        .await
        .expect("get haproxy mapped port");

    let client_no_cert = build_client(None, None, &pki);
    let client_valid_cert = build_client(Some(&pki.agent_cert_pem), Some(&pki.agent_key_pem), &pki);
    let client_revoked_cert = build_client(Some(&revoked_cert_pem), Some(&revoked_key_pem), &pki);

    // Health check without cert should succeed
    let resp = client_no_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await
        .expect("healthz request");
    assert_eq!(resp.status(), 200);

    // Valid cert should be accepted
    let resp = client_valid_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await
        .expect("valid cert request");
    assert_eq!(resp.status(), 200);

    // Revoked cert should be rejected by HAProxy at the TLS layer.
    let result = client_revoked_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await;

    match result {
        Ok(resp) => {
            // HAProxy may return 503 or other error when CRL rejects the cert
            assert!(
                resp.status().is_client_error() || resp.status().is_server_error(),
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

    server.shutdown();
}

fn write_haproxy_crl_config(tmp: &TempDir, pki: &TestPki, crl_pem: &str, backend_port: u16) {
    let ssl_dir = tmp.path().join("ssl");
    std::fs::create_dir_all(&ssl_dir).expect("create ssl dir");

    // HAProxy needs cert+key concatenated in a single PEM file
    let mut server_pem = pki.server_cert_pem.clone();
    server_pem.push_str(&pki.server_key_pem);
    std::fs::write(ssl_dir.join("server.pem"), &server_pem).expect("write server.pem");

    // CA cert for client verification
    std::fs::write(ssl_dir.join("ca.crt"), &pki.ca_cert_pem).expect("write ca.crt");

    // CRL file
    std::fs::write(ssl_dir.join("ca.crl"), crl_pem).expect("write ca.crl");

    let config = format!(
        r#"global
    log stdout format raw local0

defaults
    mode http
    log global
    option httplog
    timeout client 30s
    timeout server 30s
    timeout connect 5s

frontend https_front
    bind *:443 ssl crt /etc/haproxy/ssl/server.pem ca-file /etc/haproxy/ssl/ca.crt crl-file /etc/haproxy/ssl/ca.crl verify optional
    option forwardfor
    http-request set-header X-Forwarded-Client-Cert-Info "Subject=%[ssl_c_s_dn];SerialNumber=%[ssl_c_serial,hex];Issuer=%[ssl_c_i_dn]" if {{ ssl_c_used }}
    http-request del-header X-Forwarded-Client-Cert-Info unless {{ ssl_c_used }}
    default_backend uptrakit_https

backend uptrakit_https
    server uptrakit host.docker.internal:{backend_port} ssl verify none
"#
    );
    std::fs::write(tmp.path().join("haproxy.cfg"), config).expect("write haproxy.cfg");
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
