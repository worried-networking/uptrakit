#![expect(
    clippy::expect_used,
    reason = "integration test infrastructure: panics are acceptable in reverse-proxy test helpers"
)]

use tempfile::TempDir;
use testcontainers::GenericImage;
use testcontainers::ImageExt;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{AccessMode, Host, IntoContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;

use super::pki::TestPki;
use super::server::{IdentityResponse, TestServer};

/// Traefik L7 TLS termination integration test.
///
/// Spins up a real traefik:v3 container with a file provider that terminates
/// TLS, optionally verifies client certificates, and forwards cert details via
/// `X-Forwarded-Tls-Client-Cert-Info` using the `passTLSClientCert` middleware.
#[tokio::test]
#[ignore = "Docker integration test (traefik:v3). Run: cargo test -p uptrakit-integration-tests --test reverse_proxy traefik -- --ignored"]
async fn traefik_l7_forwards_client_cert() {
    let pki = TestPki::generate();
    let server = TestServer::start(&pki, Some("X-Forwarded-Tls-Client-Cert-Info"), None).await;

    let tmp = TempDir::new().expect("tempdir");
    write_traefik_config(&tmp, &pki, server.port);

    let container = GenericImage::new("traefik", "v3")
        .with_exposed_port(443u16.tcp())
        .with_wait_for(WaitFor::Log(LogWaitStrategy::stdout("Creating middleware")))
        .with_mount(
            Mount::bind_mount(tmp.path().to_str().expect("tmpdir path"), "/etc/traefik")
                .with_access_mode(AccessMode::ReadOnly),
        )
        .with_host("host.docker.internal", Host::HostGateway)
        .with_cmd([
            "--log.level=DEBUG",
            "--entrypoints.websecure.address=:443",
            "--providers.file.filename=/etc/traefik/dynamic.yaml",
        ])
        .start()
        .await
        .expect("start traefik container");

    let proxy_port = container
        .get_host_port_ipv4(443u16.tcp())
        .await
        .expect("get traefik mapped port");

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

fn write_traefik_config(tmp: &TempDir, pki: &TestPki, backend_port: u16) {
    // Write CA cert
    std::fs::write(tmp.path().join("ca.crt"), &pki.ca_cert_pem).expect("write ca.crt");

    // Write server cert + key
    std::fs::write(tmp.path().join("server.crt"), &pki.server_cert_pem).expect("write server.crt");
    std::fs::write(tmp.path().join("server.key"), &pki.server_key_pem).expect("write server.key");

    // Write Traefik dynamic configuration
    let dynamic = format!(
        r#"tls:
  stores:
    default:
      defaultCertificate:
        certFile: /etc/traefik/server.crt
        keyFile: /etc/traefik/server.key
  options:
    default:
      clientAuth:
        caFiles:
          - /etc/traefik/ca.crt
        clientAuthType: RequestClientCert

http:
  routers:
    uptrakit:
      rule: "PathPrefix(`/`)"
      entryPoints:
        - websecure
      tls: {{}}
      middlewares:
        - clientcert
      service: uptrakit

  middlewares:
    clientcert:
      passTLSClientCert:
        info:
          subject:
            commonName: true
            organization: true
          issuer:
            commonName: true
          serialNumber: true

  services:
    uptrakit:
      loadBalancer:
        servers:
          - url: "https://host.docker.internal:{backend_port}"
        serversTransport: insecure@file

  serversTransports:
    insecure:
      insecureSkipVerify: true
"#
    );
    std::fs::write(tmp.path().join("dynamic.yaml"), dynamic).expect("write dynamic.yaml");
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
