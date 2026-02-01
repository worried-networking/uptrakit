# Caddy Reverse Proxy

## L4 TCP Passthrough

Requires the [layer4](https://github.com/mholt/caddy-l4) plugin.

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

## L7 TLS Termination

Caddy terminates TLS, requests client certificates, and forwards the PEM-encoded cert to the controller.

### Caddyfile

```
uptrakit.example.com {
    tls {
        client_auth {
            mode request
            trusted_ca_certs_pem_file /etc/caddy/ca.crt
        }
    }

    reverse_proxy https://uptrakit:8443 {
        transport http {
            tls_trusted_ca_certs /etc/caddy/ca.crt
            tls_server_name uptrakit.example.com
        }

        header_up X-Forwarded-Tls-Client-Cert {http.request.tls.client.certificate_pem}
        header_up X-Forwarded-Proto {scheme}
        header_up X-Forwarded-Host {host}
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
- Caddy URL-encodes the PEM certificate in the header value; the controller handles URL-decoding automatically.
- The `tls_trusted_ca_certs` directive ensures Caddy trusts the controller's internal CA.

### Obtaining the CA Certificate

```bash
curl -k https://uptrakit:8443/api/v1/ca.crt -o /etc/caddy/ca.crt
```
