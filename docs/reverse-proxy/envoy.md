# Envoy Reverse Proxy

## L4 TCP Passthrough

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
                forward_client_cert_details: SANITIZE_SET
                set_current_client_cert_details:
                  subject: true
                  cert: true
                  uri: true
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

- **XFCC lacks SerialNumber:** Envoy's XFCC header does not include the certificate serial number. The controller falls back to agent-id-only lookup when serial is absent.
- **`Cert=` field:** Envoy includes the full DER-encoded certificate (base64) in the `Cert=` field of the XFCC header. The controller can parse this to extract the full identity including serial number.
- `forward_client_cert_details: SANITIZE_SET` ensures the XFCC header is set by Envoy and not spoofed by clients.
- `require_client_certificate: false` allows browsers to connect without client certificates.
- The `upgrade_configs` section enables WebSocket support.
- The upstream cluster uses TLS with the controller's CA for backend trust.

### Obtaining the CA Certificate

```bash
curl -k https://uptrakit:8443/api/v1/ca.crt -o /etc/envoy/ssl/ca.crt
```
