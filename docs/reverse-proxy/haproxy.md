# HAProxy Reverse Proxy

## L4 TCP Passthrough

```
frontend tcp_front
    bind *:443
    mode tcp
    default_backend uptrakit_tcp

backend uptrakit_tcp
    mode tcp
    server uptrakit uptrakit:8443
```

## L7 TLS Termination

HAProxy terminates TLS, extracts client certificate details, and forwards them to the controller.

```
frontend https_front
    bind *:443 ssl crt /etc/haproxy/ssl/server.pem ca-file /etc/haproxy/ssl/ca.crt verify optional
    mode http

    # Forward client certificate info as a single structured header
    http-request set-header X-Forwarded-Client-Cert-Info Subject="%{+Q}[ssl_c_s_dn]";SerialNumber="%{+Q}[ssl_c_serial]";Issuer="%{+Q}[ssl_c_i_dn]" if { ssl_c_used }
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
- `ssl_c_s_dn` returns the client certificate subject DN.
- `ssl_c_serial` returns the certificate serial number as a colon-separated hex string.
- `ssl_c_i_dn` returns the client certificate issuer DN.
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
