use tempfile::TempDir;
use testcontainers::GenericImage;
use testcontainers::ImageExt;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{AccessMode, Host, IntoContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;

use super::pki::TestPki;
use super::server::{IdentityResponse, TestServer};

/// Caddy L7 TLS termination integration test.
///
/// Spins up a real caddy:latest container that terminates TLS, optionally
/// verifies client certificates, and forwards the PEM-encoded client cert
/// via `X-Forwarded-Tls-Client-Cert`. Caddy URL-encodes the PEM; the
/// controller handles decoding.
#[tokio::test]
#[ignore = "Docker integration test (caddy:latest). Run: cargo test -p uptrakit-integration-tests --test reverse_proxy caddy -- --ignored"]
async fn caddy_l7_forwards_client_cert() {
    let pki = TestPki::generate();
    let server = TestServer::start(&pki, None, Some("X-Forwarded-Tls-Client-Cert")).await;

    let tmp = TempDir::new().expect("tempdir");
    write_caddy_config(&tmp, &pki, server.port);

    let container = GenericImage::new("caddy", "latest")
        .with_exposed_port(443u16.tcp())
        .with_wait_for(WaitFor::Log(LogWaitStrategy::stderr("serving initial")))
        .with_mount(
            Mount::bind_mount(
                tmp.path()
                    .join("Caddyfile")
                    .to_str()
                    .expect("caddyfile path"),
                "/etc/caddy/Caddyfile",
            )
            .with_access_mode(AccessMode::ReadOnly),
        )
        .with_mount(
            Mount::bind_mount(
                tmp.path().join("certs").to_str().expect("certs path"),
                "/etc/caddy/certs",
            )
            .with_access_mode(AccessMode::ReadOnly),
        )
        .with_host("host.docker.internal", Host::HostGateway)
        .start()
        .await
        .expect("start caddy container");

    let proxy_port = container
        .get_host_port_ipv4(443u16.tcp())
        .await
        .expect("get caddy mapped port");

    let client_no_cert = build_client(None, &pki);
    let client_with_cert = build_client(Some(&pki), &pki);

    // Health check: GET /healthz without client cert
    let resp = client_no_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await
        .expect("healthz request");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.expect("body"), "ok");

    // With client cert: GET /test/identity
    let resp = client_with_cert
        .get(format!("https://localhost:{proxy_port}/test/identity"))
        .send()
        .await
        .expect("identity request with cert");
    assert_eq!(resp.status(), 200);
    let identity: IdentityResponse = resp.json().await.expect("parse identity");
    assert_eq!(
        identity.agent_id.as_deref(),
        Some(pki.agent_id.to_string().as_str()),
        "agent_id should match"
    );
    assert!(
        identity.cert_serial.is_some(),
        "cert_serial should be present"
    );

    // Without client cert: GET /test/identity
    let resp = client_no_cert
        .get(format!("https://localhost:{proxy_port}/test/identity"))
        .send()
        .await
        .expect("identity request without cert");
    assert_eq!(resp.status(), 200);
    let identity: IdentityResponse = resp.json().await.expect("parse identity");
    assert!(identity.agent_id.is_none(), "agent_id should be null");

    server.shutdown();
}

fn write_caddy_config(tmp: &TempDir, pki: &TestPki, backend_port: u16) {
    // Write certs to a subdirectory
    let certs_dir = tmp.path().join("certs");
    std::fs::create_dir_all(&certs_dir).expect("create certs dir");
    std::fs::write(certs_dir.join("ca.crt"), &pki.ca_cert_pem).expect("write ca.crt");
    std::fs::write(certs_dir.join("server.crt"), &pki.server_cert_pem).expect("write server.crt");
    std::fs::write(certs_dir.join("server.key"), &pki.server_key_pem).expect("write server.key");

    // Caddyfile: Caddy uses {http.request.tls.client.certificate_pem} which is
    // URL-encoded PEM. The controller decodes this in try_pem_header.
    let caddyfile = format!(
        r#"{{
    auto_https off
}}

:443 {{
    tls /etc/caddy/certs/server.crt /etc/caddy/certs/server.key {{
        client_auth {{
            mode request
            trust_pool file {{
                pem_file /etc/caddy/certs/ca.crt
            }}
        }}
    }}

    reverse_proxy https://host.docker.internal:{backend_port} {{
        transport http {{
            tls_insecure_skip_verify
        }}

        header_up X-Forwarded-Tls-Client-Cert "{{http.request.tls.client.certificate_der_base64}}"
    }}
}}
"#
    );
    std::fs::write(tmp.path().join("Caddyfile"), caddyfile).expect("write Caddyfile");
}

fn build_client(agent_pki: Option<&TestPki>, ca_pki: &TestPki) -> reqwest::Client {
    let ca_cert = reqwest::Certificate::from_pem(ca_pki.ca_cert_pem.as_bytes()).expect("CA cert");

    let mut builder = reqwest::Client::builder().tls_certs_merge([ca_cert]);

    if let Some(pki) = agent_pki {
        let mut id_pem = pki.agent_cert_pem.as_bytes().to_vec();
        id_pem.extend_from_slice(pki.agent_key_pem.as_bytes());
        let identity = reqwest::Identity::from_pem(&id_pem).expect("client identity");
        builder = builder.identity(identity);
    }

    builder.build().expect("reqwest client")
}
