---
title: Caddy Reverse Proxy
weight: 50
description: Configuration examples for Caddy as a reverse proxy using the layer4 plugin for TCP passthrough or HTTP termination with PEM header handling.
---

# Caddy Reverse Proxy

## L4 TLS Passthrough

Requires the [layer4](https://github.com/mholt/caddy-l4) plugin. The proxy forwards raw TCP traffic to the controller without terminating TLS. The
controller handles mTLS directly with agents.

```caddyfile
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

No controller configuration changes needed for passthrough mode — mTLS is handled end-to-end by the controller.

## L7 TLS Termination

Caddy terminates TLS, requests client certificates, and forwards the PEM-encoded cert to the controller.

### Caddyfile

```caddyfile
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

The controller needs to know the proxy's IP and which header carries the PEM-encoded client certificate.

**Option A — CLI flags:**

```bash
uptrakit-controller \
  --trusted-proxy=<caddy-ip> \
  --forwarded-client-cert-pem-header=X-Forwarded-Tls-Client-Cert
```

**Option B — Web UI:** Navigate to Settings > Network and set:

- **Trusted Proxies**: the Caddy server's IP or CIDR
- **Forwarded Client Cert PEM Header**: `X-Forwarded-Tls-Client-Cert`

**Option C — API:**

```bash
curl -s -X PUT -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  https://controller:8443/api/v1/settings/network \
  -d '{
    "trusted_proxies": ["<caddy-ip>"],
    "forwarded_client_cert_pem_header": "X-Forwarded-Tls-Client-Cert"
  }'
```

Changes via Web UI or API apply immediately without a restart.

### Notes

- `mode request` makes client certificates optional (browsers work without one).
- `trust_pool file` is the modern syntax (Caddy 2.8+). Older versions used the deprecated `trusted_ca_cert_file` directive.
- `certificate_der_base64` sends the client certificate as base64-encoded DER, which is HTTP-header safe. The older `certificate_pem` placeholder
  contains raw PEM with newlines, which is **not valid** in HTTP headers.
- Caddy sets `X-Forwarded-For`, `X-Forwarded-Proto`, and `X-Forwarded-Host` automatically — no explicit `header_up` directives are needed for these.
- The `tls_trusted_ca_certs` directive ensures Caddy trusts the controller's internal CA.

### Revocation Checking

Caddy does not natively support CRL or OCSP checking for client certificates. Revocation is handled at the application layer — the controller's mTLS
verifier checks CRLs for direct (non-proxied) connections, and the proxy header middleware verifies forwarded certificate identity regardless.

### Obtaining the CA Certificate

```bash
curl -k https://uptrakit:8443/api/v1/pki/ca.crt -o /etc/caddy/ca.crt
```
