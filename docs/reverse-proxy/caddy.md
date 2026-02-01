# Caddy Reverse Proxy

## L4 TLS Passthrough

Requires the [layer4](https://github.com/mholt/caddy-l4) plugin. The proxy forwards raw TCP traffic to the controller without terminating TLS. The controller handles mTLS directly with agents.

```
{
    layer4 {
        :443 {
            route {
                tls {
                    connection_policy {
                        match {
                            sni uptrakit.example.com
                        }
                    }
                }
                proxy {
                    upstream uptrakit:8443
                }
            }
        }
    }
}
```

No controller flags needed for passthrough mode — mTLS is handled end-to-end by the controller.

## L7 TLS Termination

Caddy terminates TLS, requests client certificates, and forwards the PEM-encoded cert to the controller.

### Caddyfile

```
uptrakit.example.com {
    tls {
        client_auth {
            mode request
            trust_pool file {
                pem_file /etc/caddy/ca.crt
            }
        }
    }

    reverse_proxy https://uptrakit:8443 {
        transport http {
            tls_trusted_ca_certs /etc/caddy/ca.crt
            tls_server_name uptrakit.example.com
        }

        header_up X-Forwarded-Tls-Client-Cert {http.request.tls.client.certificate_der_base64}
    }
}
```

### Controller Configuration

```bash
uptrakit-controller \
  --trusted-proxy=<caddy-ip> \
  --forwarded-client-cert-pem-header=X-Forwarded-Tls-Client-Cert
```

### Notes

- `mode request` makes client certificates optional (browsers work without one).
- `trust_pool file` is the modern syntax (Caddy 2.8+). Older versions used the deprecated `trusted_ca_cert_file` directive.
- `certificate_der_base64` sends the client certificate as base64-encoded DER, which is HTTP-header safe. The older `certificate_pem` placeholder contains raw PEM with newlines, which is **not valid** in HTTP headers.
- Caddy sets `X-Forwarded-For`, `X-Forwarded-Proto`, and `X-Forwarded-Host` automatically — no explicit `header_up` directives are needed for these.
- The `tls_trusted_ca_certs` directive ensures Caddy trusts the controller's internal CA.

### Obtaining the CA Certificate

```bash
curl -k https://uptrakit:8443/api/v1/ca.crt -o /etc/caddy/ca.crt
```
