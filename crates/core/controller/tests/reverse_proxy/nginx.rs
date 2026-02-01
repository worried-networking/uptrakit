use tempfile::TempDir;
use testcontainers::GenericImage;
use testcontainers::ImageExt;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{AccessMode, Host, IntoContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;

use super::pki::TestPki;
use super::server::{IdentityResponse, TestServer};

/// Nginx L7 TLS termination integration test.
///
/// Spins up a real nginx:latest container that terminates TLS, optionally
/// verifies client certificates, and forwards cert details via
/// `X-Forwarded-Client-Cert-Info`.
#[tokio::test]
#[ignore = "Docker integration test (nginx:latest). Run: cargo test -p uptrakit-controller reverse_proxy::nginx -- --ignored"]
async fn nginx_l7_forwards_client_cert() {
    let pki = TestPki::generate();
    let server = TestServer::start(&pki, Some("X-Forwarded-Client-Cert-Info"), None).await;

    let tmp = TempDir::new().expect("tempdir");
    write_nginx_config(&tmp, &pki, server.port);

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

fn write_nginx_config(tmp: &TempDir, pki: &TestPki, backend_port: u16) {
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

fn build_client(agent_pki: Option<&TestPki>, ca_pki: &TestPki) -> reqwest::Client {
    let ca_cert = reqwest::Certificate::from_pem(ca_pki.ca_cert_pem.as_bytes()).expect("CA cert");

    let mut builder = reqwest::Client::builder()
        .add_root_certificate(ca_cert)
        .danger_accept_invalid_certs(false);

    if let Some(pki) = agent_pki {
        let mut id_pem = pki.agent_cert_pem.as_bytes().to_vec();
        id_pem.extend_from_slice(pki.agent_key_pem.as_bytes());
        let identity = reqwest::Identity::from_pem(&id_pem).expect("client identity");
        builder = builder.identity(identity);
    }

    builder.build().expect("reqwest client")
}
