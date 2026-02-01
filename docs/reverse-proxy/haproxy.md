# HAProxy Reverse Proxy

## L4 TLS Passthrough

Forward raw TCP traffic to the controller without terminating TLS. The controller handles mTLS directly with agents.

```
frontend tcp_front
    bind *:443
    mode tcp
    default_backend uptrakit_tcp

backend uptrakit_tcp
    mode tcp
    server uptrakit uptrakit:8443
```

No controller flags needed for passthrough mode.

## L7 TLS Termination

HAProxy terminates TLS, extracts client certificate details, and forwards them to the controller.

```
frontend https_front
    bind *:443 ssl crt /etc/haproxy/ssl/server.pem ca-file /etc/haproxy/ssl/ca.crt verify optional
    mode http
    option forwardfor

    # Forward client certificate info as a single structured header.
    # The value MUST be quoted to prevent HAProxy 3.x from misinterpreting
    # semicolons as ACL separators.
    http-request set-header X-Forwarded-Client-Cert-Info "Subject=%[ssl_c_s_dn];SerialNumber=%[ssl_c_serial,hex];Issuer=%[ssl_c_i_dn]" if { ssl_c_used }
    # Strip the header if no client cert was presented
    http-request del-header X-Forwarded-Client-Cert-Info unless { ssl_c_used }

    default_backend uptrakit_https

backend uptrakit_https
    mode http
    server uptrakit uptrakit:8443 ssl ca-file /etc/haproxy/ssl/ca.crt verify required
```

### Controller Configuration

```bash
uptrakit-controller \
  --trusted-proxy=<haproxy-ip> \
  --forwarded-client-cert-info-header=X-Forwarded-Client-Cert-Info
```

### Notes

- `verify optional` on the frontend bind makes client certificates optional (browsers work without one).
- **`option forwardfor` is recommended.** It adds the `X-Forwarded-For` header so the controller can resolve the real client IP for logging and rate limiting.
- `ssl_c_s_dn` returns the client certificate subject DN.
- `ssl_c_serial,hex` returns the certificate serial number as a hex string. The `,hex` converter is required to get a consistent hex format.
- `ssl_c_i_dn` returns the client certificate issuer DN.
- **The `set-header` value must be quoted** (enclosed in `"..."`) in HAProxy 3.x. Unquoted semicolons in the value are interpreted as ACL separators, causing the header to be silently truncated.
- The backend uses `ssl ca-file` to trust the controller's internal CA.
- HAProxy's `server.pem` file should contain both the public certificate and private key concatenated.

### WebSocket Support

HAProxy supports WebSocket natively in HTTP mode. For long-lived connections, increase timeouts:

```
defaults
    timeout client  86400s
    timeout server  86400s
    timeout tunnel  86400s
```

### Obtaining the CA Certificate

```bash
curl -k https://uptrakit:8443/api/v1/ca.crt -o /etc/haproxy/ssl/ca.crt
```
