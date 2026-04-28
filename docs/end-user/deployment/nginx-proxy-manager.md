---
title: Nginx Proxy Manager
weight: 30
description: Configure Nginx Proxy Manager via its web GUI for basic L7 reverse proxying and advanced client certificate forwarding to the controller.
---

# Nginx Proxy Manager

[Nginx Proxy Manager](https://nginxproxymanager.com/) (NPM) provides a web GUI for managing Nginx reverse proxies. Client certificate forwarding
requires custom Nginx configuration.

## Basic Reverse Proxy (GUI)

1. Add a new **Proxy Host** in the NPM dashboard.
1. Set **Domain Names** to your controller domain (e.g., `uptrakit.example.com`).
1. Set **Scheme** to `https`, **Forward Hostname/IP** to `uptrakit`, **Forward Port** to `8443`.
1. Enable **SSL** and configure your public certificate (Let's Encrypt or custom).
1. Enable **WebSocket Support**.

This provides basic L7 reverse proxying without client certificate forwarding. Agents must connect directly to the controller for mTLS, or use L4
passthrough via a custom Nginx stream config.

## Client Certificate Forwarding (Advanced)

NPM supports custom Nginx configuration snippets. To forward client certificate info:

1. In the Proxy Host settings, go to the **Advanced** tab.
1. Add the following custom Nginx configuration:

```nginx
# Client certificate verification
ssl_client_certificate /data/custom_ssl/ca.crt;
ssl_verify_client optional;

# Trust the controller's internal CA for backend connection
proxy_ssl_trusted_certificate /data/custom_ssl/ca.crt;
proxy_ssl_verify on;

# Forward client certificate information
proxy_set_header X-Forwarded-Client-Cert-Info "Subject=\"$ssl_client_s_dn\";SerialNumber=\"$ssl_client_serial\";Issuer=\"$ssl_client_i_dn\"";
```

1. Copy the controller's CA certificate into NPM's data directory:

```bash
curl -k https://uptrakit:8443/api/v1/pki/ca.crt -o /path/to/npm/data/custom_ssl/ca.crt
```

### Controller Configuration

The controller needs to know the proxy's IP and which header carries the client certificate info.

**Option A — CLI flags:**

```bash
uptrakit-controller \
  --trusted-proxy=<npm-ip> \
  --forwarded-client-cert-info-header=X-Forwarded-Client-Cert-Info
```

**Option B — Web UI:** Navigate to Settings > Network and set:

- **Trusted Proxies**: the NPM server's IP or CIDR
- **Forwarded Client Cert Info Header**: `X-Forwarded-Client-Cert-Info`

**Option C — API:**

```bash
curl -s -X PUT -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  https://controller:8443/api/v1/settings/network \
  -d '{
    "trusted_proxies": ["<npm-ip>"],
    "forwarded_client_cert_info_header": "X-Forwarded-Client-Cert-Info"
  }'
```

Changes via Web UI or API apply immediately without a restart.

### Notes

- The custom Nginx config is appended to the `location /` block in NPM's generated config.
- WebSocket support should be enabled in the GUI (adds `Upgrade` and `Connection` headers).
- NPM's Docker container stores data in `/data`; mount the CA cert file accordingly.
- After CA rotation, re-export the CA certificate and reload NPM.
