# Nginx Proxy Manager

[Nginx Proxy Manager](https://nginxproxymanager.com/) (NPM) provides a web GUI for managing Nginx reverse proxies. Client certificate forwarding requires custom Nginx configuration.

## Basic Reverse Proxy (GUI)

1. Add a new **Proxy Host** in the NPM dashboard.
2. Set **Domain Names** to your controller domain (e.g., `uptrakit.example.com`).
3. Set **Scheme** to `https`, **Forward Hostname/IP** to `uptrakit`, **Forward Port** to `8443`.
4. Enable **SSL** and configure your public certificate (Let's Encrypt or custom).
5. Enable **WebSocket Support**.

This provides basic L7 reverse proxying without client certificate forwarding. Agents must connect directly to the controller for mTLS, or use L4 passthrough via a custom Nginx stream config.

## Client Certificate Forwarding (Advanced)

NPM supports custom Nginx configuration snippets. To forward client certificate info:

1. In the Proxy Host settings, go to the **Advanced** tab.
2. Add the following custom Nginx configuration:

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

3. Copy the controller's CA certificate into NPM's data directory:

```bash
curl -k https://uptrakit:8443/api/v1/ca.crt -o /path/to/npm/data/custom_ssl/ca.crt
```

### Controller Configuration

```bash
uptrakit-controller \
  --trusted-proxy=<npm-ip> \
  --forwarded-client-cert-info-header=X-Forwarded-Client-Cert-Info
```

### Notes

- The custom Nginx config is appended to the `location /` block in NPM's generated config.
- WebSocket support should be enabled in the GUI (adds `Upgrade` and `Connection` headers).
- NPM's Docker container stores data in `/data`; mount the CA cert file accordingly.
- After CA rotation, re-export the CA certificate and reload NPM.
