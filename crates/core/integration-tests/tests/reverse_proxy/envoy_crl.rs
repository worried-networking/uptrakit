#![expect(
    clippy::expect_used,
    reason = "integration test infrastructure: panics are acceptable in reverse-proxy test helpers"
)]

use tempfile::TempDir;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{AccessMode, Host, IntoContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

use super::pki::{TestPki, extract_serial_hex};
use super::server::TestServer;

/// Envoy CRL revocation checking integration test.
///
/// Validates that Envoy rejects client certificates that appear in a CRL
/// while still accepting valid (non-revoked) certificates.
#[tokio::test]
#[ignore = "Docker integration test (envoyproxy/envoy:v1.31-latest). Run: cargo test -p uptrakit-integration-tests --test reverse_proxy envoy_crl -- --ignored"]
async fn envoy_crl_rejects_revoked_cert() {
    let pki = TestPki::generate();
    let (revoked_cert_pem, revoked_key_pem, _revoked_id) = pki.generate_extra_agent_cert();

    // Generate CRL containing the revoked cert's serial
    let revoked_serial = extract_serial_hex(&revoked_cert_pem);
    let crl_pem = pki.generate_crl_pem(&[&revoked_serial]);

    let server = TestServer::start(&pki, Some("X-Forwarded-Client-Cert"), None).await;

    let tmp = TempDir::new().expect("tempdir");
    write_envoy_crl_config(&tmp, &pki, &crl_pem, server.port);

    let container = GenericImage::new("envoyproxy/envoy", "v1.31-latest")
        .with_exposed_port(443u16.tcp())
        .with_wait_for(WaitFor::Log(LogWaitStrategy::stderr(
            "all clusters initialized",
        )))
        .with_mount(
            Mount::bind_mount(
                tmp.path()
                    .join("envoy.yaml")
                    .to_str()
                    .expect("envoy cfg path"),
                "/etc/envoy/envoy.yaml",
            )
            .with_access_mode(AccessMode::ReadOnly),
        )
        .with_mount(
            Mount::bind_mount(
                tmp.path().join("ssl").to_str().expect("ssl path"),
                "/etc/envoy/ssl",
            )
            .with_access_mode(AccessMode::ReadOnly),
        )
        .with_host("host.docker.internal", Host::HostGateway)
        .start()
        .await
        .expect("start envoy container");

    let proxy_port = container
        .get_host_port_ipv4(443u16.tcp())
        .await
        .expect("get envoy mapped port");

    let client_no_cert = build_client(None, None, &pki);
    let client_valid_cert = build_client(Some(&pki.agent_cert_pem), Some(&pki.agent_key_pem), &pki);
    let client_revoked_cert = build_client(Some(&revoked_cert_pem), Some(&revoked_key_pem), &pki);

    // Health check without cert should succeed
    let resp = client_no_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await
        .expect("healthz request");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Valid cert should be accepted
    let resp = client_valid_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await
        .expect("valid cert request");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Revoked cert should be rejected by Envoy at the TLS layer.
    let result = client_revoked_cert
        .get(format!("https://localhost:{proxy_port}/healthz"))
        .send()
        .await;

    match result {
        Ok(resp) => {
            // Envoy may return 503 or other error when CRL rejects the cert
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

fn write_envoy_crl_config(tmp: &TempDir, pki: &TestPki, crl_pem: &str, backend_port: u16) {
    let ssl_dir = tmp.path().join("ssl");
    std::fs::create_dir_all(&ssl_dir).expect("create ssl dir");

    std::fs::write(ssl_dir.join("ca.crt"), &pki.ca_cert_pem).expect("write ca.crt");
    std::fs::write(ssl_dir.join("server.crt"), &pki.server_cert_pem).expect("write server.crt");
    std::fs::write(ssl_dir.join("server.key"), &pki.server_key_pem).expect("write server.key");
    std::fs::write(ssl_dir.join("ca.crl"), crl_pem).expect("write ca.crl");

    let config = format!(
        r#"static_resources:
  listeners:
    - name: https_listener
      address:
        socket_address:
          address: 0.0.0.0
          port_value: 443
      filter_chains:
        - transport_socket:
            name: envoy.transport_sockets.tls
            typed_config:
              "@type": type.googleapis.com/envoy.extensions.transport_sockets.tls.v3.DownstreamTlsContext
              common_tls_context:
                tls_certificates:
                  - certificate_chain:
                      filename: /etc/envoy/ssl/server.crt
                    private_key:
                      filename: /etc/envoy/ssl/server.key
                validation_context:
                  trusted_ca:
                    filename: /etc/envoy/ssl/ca.crt
                  only_verify_leaf_cert_crl: true
                  crl:
                    filename: /etc/envoy/ssl/ca.crl
              require_client_certificate: false
          filters:
            - name: envoy.filters.network.http_connection_manager
              typed_config:
                "@type": type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
                stat_prefix: ingress_http
                use_remote_address: true
                forward_client_cert_details: SANITIZE_SET
                set_current_client_cert_details:
                  subject: true
                  cert: true
                upgrade_configs:
                  - upgrade_type: websocket
                route_config:
                  name: local_route
                  virtual_hosts:
                    - name: uptrakit
                      domains: ["*"]
                      routes:
                        - match:
                            prefix: "/"
                          route:
                            cluster: uptrakit_cluster
                http_filters:
                  - name: envoy.filters.http.router
                    typed_config:
                      "@type": type.googleapis.com/envoy.extensions.filters.http.router.v3.Router

  clusters:
    - name: uptrakit_cluster
      connect_timeout: 5s
      type: STRICT_DNS
      dns_lookup_family: V4_ONLY
      transport_socket:
        name: envoy.transport_sockets.tls
        typed_config:
          "@type": type.googleapis.com/envoy.extensions.transport_sockets.tls.v3.UpstreamTlsContext
          common_tls_context:
            validation_context:
              trusted_ca:
                filename: /etc/envoy/ssl/ca.crt
            alpn_protocols:
              - h2
              - http/1.1
      load_assignment:
        cluster_name: uptrakit_cluster
        endpoints:
          - lb_endpoints:
              - endpoint:
                  address:
                    socket_address:
                      address: host.docker.internal
                      port_value: {backend_port}
"#
    );
    std::fs::write(tmp.path().join("envoy.yaml"), config).expect("write envoy.yaml");
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
