# Nginx Reverse Proxy

## L4 TLS Passthrough

Use the `stream` module to forward raw TCP traffic. The controller handles mTLS directly with agents.

```nginx
stream {
    upstream uptrakit_backend {
        server uptrakit:8443;
    }

    server {
        listen 443;
        proxy_pass uptrakit_backend;
    }
}
```

No controller flags needed for passthrough mode.

**Note:** L4 passthrough does not support SNI-based routing with multiple backends unless you use the `ngx_stream_ssl_preread_module`:

```nginx
stream {
    map $ssl_preread_server_name $backend {
        uptrakit.example.com  uptrakit_backend;
        default               default_backend;
    }

    upstream uptrakit_backend {
        server uptrakit:8443;
    }

    server {
        listen 443;
        ssl_preread on;
        proxy_pass $backend;
    }
}
```

## L7 TLS Termination

Nginx terminates TLS, optionally verifies client certificates, and forwards cert details via headers.

```nginx
http {
    server {
        listen 443 ssl;
        server_name uptrakit.example.com;

        # Public TLS certificate (e.g., Let's Encrypt)
        ssl_certificate     /etc/nginx/ssl/server.crt;
        ssl_certificate_key /etc/nginx/ssl/server.key;

        # Client certificate verification (optional)
        ssl_client_certificate /etc/nginx/ssl/ca.crt;
        ssl_verify_client optional;  # "optional" allows browsers without client certs

        location / {
            proxy_pass https://uptrakit:8443;

            # Trust the controller's internal CA for backend connection
            proxy_ssl_trusted_certificate /etc/nginx/ssl/ca.crt;
            proxy_ssl_verify on;
            proxy_ssl_server_name on;

            # Forward client certificate information
            proxy_set_header X-Forwarded-Client-Cert-Info "Subject=\"$ssl_client_s_dn\";SerialNumber=\"$ssl_client_serial\";Issuer=\"$ssl_client_i_dn\"";

            # Standard proxy headers
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
            proxy_set_header X-Forwarded-Proto $scheme;
            proxy_set_header X-Forwarded-Host $host;
            proxy_set_header Host $host;

            # WebSocket support
            proxy_http_version 1.1;
            proxy_set_header Upgrade $http_upgrade;
            proxy_set_header Connection "upgrade";
            proxy_read_timeout 86400;
        }
    }
}
```

### Controller Configuration

```bash
uptrakit-controller \
  --trusted-proxy=<nginx-ip> \
  --forwarded-client-cert-info-header=X-Forwarded-Client-Cert-Info
```

### Notes

- Nginx's `$ssl_client_s_dn` uses the OpenSSL format (`/CN=uuid/O=Org`). The controller handles this format.
- `$ssl_client_serial` is a colon-separated hex string, which the controller normalizes automatically.
- `ssl_verify_client optional` ensures browsers without client certificates can still access the web UI.
- WebSocket support requires `proxy_http_version 1.1` and the Upgrade/Connection headers.
- `proxy_read_timeout 86400` (24h) prevents Nginx from closing long-lived WebSocket connections.

### OCSP Revocation Checking (Recommended)

Nginx 1.19.0+ supports OCSP checking for client certificates. This is the recommended approach — it validates revocation in real-time with no staleness window.

```nginx
# Add to the server block, alongside ssl_client_certificate:
ssl_ocsp leaf;
ssl_ocsp_responder https://controller:8443/api/v1/pki/ocsp;
```

`ssl_ocsp leaf` validates only the leaf (agent) certificate, not the full chain. The `ssl_ocsp_responder` directive overrides the OCSP URL from the certificate's AIA extension if needed. When `--pki-addr` is configured, agent certificates include the OCSP URL in their AIA extension, so `ssl_ocsp on` (without an explicit responder) also works if the controller is reachable at the embedded URL.

**Note:** Nginx caches OCSP responses. After revoking an agent certificate, there may be a brief delay before Nginx rejects connections from that agent.

### CRL Revocation Checking (Alternative)

CRL files are static snapshots. You must periodically download a fresh CRL and reload Nginx. The controller rebuilds CRLs hourly and immediately on every revocation event. Recommended refresh: cron job every 30-60 minutes.

```nginx
# Add to the server block, alongside ssl_client_certificate:
ssl_crl /etc/nginx/ssl/ca.crl;
```

Periodic refresh example:

```bash
# crontab entry: refresh CRL every 30 minutes
*/30 * * * * curl -sk https://controller:8443/api/v1/pki/ca.crl -o /etc/nginx/ssl/ca.crl && nginx -s reload
```

### Obtaining the CA Certificate

```bash
curl -k https://uptrakit:8443/api/v1/pki/ca.crt -o /etc/nginx/ssl/ca.crt
```
