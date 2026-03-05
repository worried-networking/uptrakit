use tempfile::TempDir;
use testcontainers::GenericImage;
use testcontainers::ImageExt;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{AccessMode, Host, IntoContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;

use super::pki::TestPki;
use super::server::{IdentityResponse, TestServer};

/// HAProxy L7 TLS termination integration test.
///
/// Spins up a real haproxy:latest container that terminates TLS, optionally
/// verifies client certificates, and forwards cert details via
/// `X-Forwarded-Client-Cert-Info`. HAProxy requires cert+key concatenated
/// in a single `server.pem` file.
#[tokio::test]
#[ignore = "Docker integration test (haproxy:latest). Run: cargo test -p uptrakit-integration-tests --test reverse_proxy haproxy_l7 -- --ignored"]
async fn haproxy_l7_forwards_client_cert() {
    let pki = TestPki::generate();
    let server = TestServer::start(&pki, Some("X-Forwarded-Client-Cert-Info"), None).await;

    let tmp = TempDir::new().expect("tempdir");
    write_haproxy_config(&tmp, &pki, server.port);

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

fn write_haproxy_config(tmp: &TempDir, pki: &TestPki, backend_port: u16) {
    let ssl_dir = tmp.path().join("ssl");
    std::fs::create_dir_all(&ssl_dir).expect("create ssl dir");

    // HAProxy needs cert+key concatenated in a single PEM file
    let mut server_pem = pki.server_cert_pem.clone();
    server_pem.push_str(&pki.server_key_pem);
    std::fs::write(ssl_dir.join("server.pem"), &server_pem).expect("write server.pem");

    // CA cert for client verification
    std::fs::write(ssl_dir.join("ca.crt"), &pki.ca_cert_pem).expect("write ca.crt");

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
    bind *:443 ssl crt /etc/haproxy/ssl/server.pem ca-file /etc/haproxy/ssl/ca.crt verify optional
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
