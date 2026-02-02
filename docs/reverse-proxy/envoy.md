# Envoy Reverse Proxy

## L4 TLS Passthrough

Forward raw TCP traffic to the controller without terminating TLS. The controller handles mTLS directly with agents.

```yaml
static_resources:
  listeners:
    - name: tcp_passthrough
      address:
        socket_address:
          address: 0.0.0.0
          port_value: 443
      filter_chains:
        - filters:
            - name: envoy.filters.network.tcp_proxy
              typed_config:
                "@type": type.googleapis.com/envoy.extensions.filters.network.tcp_proxy.v3.TcpProxy
                stat_prefix: uptrakit
                cluster: uptrakit_cluster

  clusters:
    - name: uptrakit_cluster
      connect_timeout: 5s
      type: STRICT_DNS
      load_assignment:
        cluster_name: uptrakit_cluster
        endpoints:
          - lb_endpoints:
              - endpoint:
                  address:
                    socket_address:
                      address: uptrakit
                      port_value: 8443
```

No controller flags needed for passthrough mode.

## L7 TLS Termination

Envoy terminates TLS and forwards client certificate details via the XFCC (X-Forwarded-Client-Cert) header.

```yaml
static_resources:
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
                      domains: ["uptrakit.example.com"]
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
                      address: uptrakit
                      port_value: 8443
```

### Controller Configuration

```bash
uptrakit-controller \
  --trusted-proxy=<envoy-ip> \
  --forwarded-client-cert-info-header=X-Forwarded-Client-Cert
```

### Notes

- **`use_remote_address: true` is recommended.** It causes Envoy to set `X-Forwarded-For` so the controller can resolve the real client IP for logging and rate limiting. It also affects how `SANITIZE_SET` processes the XFCC header.
- **`Cert=` field (preferred):** Envoy includes the URL-encoded PEM certificate in the `Cert` field of the XFCC header. The controller parses this to extract the full identity including serial number and verifies the issuer CN.
- **`Subject=` field (fallback):** When `Cert` is absent, the controller falls back to the `Subject` field. However, Envoy's XFCC does not include a `SerialNumber` field, so this path uses agent-id-only lookup.
- `forward_client_cert_details: SANITIZE_SET` ensures the XFCC header is set by Envoy and not spoofed by clients.
- `require_client_certificate: false` allows browsers to connect without client certificates.
- The `upgrade_configs` section enables WebSocket support.
- The upstream cluster uses TLS with the controller's CA for backend trust. The `alpn_protocols` setting enables HTTP/2 negotiation with the backend.

### CRL Revocation Checking

Envoy supports CRL checking for client certificates. CRL files are static — use an external sidecar or init container to periodically fetch a fresh CRL from the controller. The controller rebuilds CRLs hourly and immediately on every revocation event. Recommended refresh: every 30-60 minutes.

Add `crl` to the `validation_context` in the downstream TLS context:

```yaml
validation_context:
  trusted_ca:
    filename: /etc/envoy/ssl/ca.crt
  only_verify_leaf_cert_crl: true
  crl:
    filename: /etc/envoy/ssl/ca.crl
```

`only_verify_leaf_cert_crl: true` ensures only the leaf (agent) certificate is checked against the CRL, not the CA certificate itself.

Periodic refresh example (sidecar script or cron):

```bash
*/30 * * * * curl -sk https://controller:8443/api/v1/pki/ca.crl -o /etc/envoy/ssl/ca.crl
```

Note: Envoy requires a restart or SDS update to pick up CRL file changes — it does not watch the filesystem.

Envoy does not support OCSP checking for client certificates.

### Obtaining the CA Certificate

```bash
curl -k https://uptrakit:8443/api/v1/pki/ca.crt -o /etc/envoy/ssl/ca.crt
```
