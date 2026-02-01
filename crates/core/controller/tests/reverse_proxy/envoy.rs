use tempfile::TempDir;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{AccessMode, Host, IntoContainerPort, Mount, WaitFor};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers::GenericImage;

use super::pki::TestPki;
use super::server::{IdentityResponse, TestServer};

/// Envoy L7 TLS termination integration test.
///
/// Spins up a real envoyproxy/envoy:v1.31-latest container that terminates
/// TLS, requests client certificates, and forwards the XFCC
/// (X-Forwarded-Client-Cert) header with `Subject` and `Cert` fields.
/// The controller parses the `Cert` field (DER base64) to extract full
/// identity including serial number.
#[tokio::test]
#[ignore = "Docker integration test (envoyproxy/envoy:v1.31-latest). Run: cargo test -p uptrakit-controller reverse_proxy::envoy -- --ignored"]
async fn envoy_l7_forwards_client_cert() {
    let pki = TestPki::generate();
    let server =
        TestServer::start(&pki, Some("X-Forwarded-Client-Cert"), None).await;

    let tmp = TempDir::new().expect("tempdir");
    write_envoy_config(&tmp, &pki, server.port);

    let container = GenericImage::new("envoyproxy/envoy", "v1.31-latest")
        .with_exposed_port(443u16.tcp())
        .with_wait_for(WaitFor::Log(LogWaitStrategy::stderr(
            "all clusters initialized",
        )))
        .with_mount(
            Mount::bind_mount(
                tmp.path().join("envoy.yaml").to_str().expect("envoy cfg path"),
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

fn write_envoy_config(tmp: &TempDir, pki: &TestPki, backend_port: u16) {
    let ssl_dir = tmp.path().join("ssl");
    std::fs::create_dir_all(&ssl_dir).expect("create ssl dir");

    std::fs::write(ssl_dir.join("ca.crt"), &pki.ca_cert_pem).expect("write ca.crt");
    std::fs::write(ssl_dir.join("server.crt"), &pki.server_cert_pem).expect("write server.crt");
    std::fs::write(ssl_dir.join("server.key"), &pki.server_key_pem).expect("write server.key");

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

fn build_client(agent_pki: Option<&TestPki>, ca_pki: &TestPki) -> reqwest::Client {
    let ca_cert =
        reqwest::Certificate::from_pem(ca_pki.ca_cert_pem.as_bytes()).expect("CA cert");

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
