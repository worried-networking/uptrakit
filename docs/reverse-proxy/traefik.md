# Traefik Reverse Proxy

## L4 TCP Passthrough

Forward raw TCP to the controller without TLS termination.

### Docker Compose

```yaml
services:
  traefik:
    image: traefik:v3
    command:
      - --entrypoints.websecure.address=:443
      - --providers.docker=true
    ports:
      - "443:443"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro

  uptrakit:
    image: uptrakit-controller
    labels:
      - traefik.enable=true
      - traefik.tcp.routers.uptrakit.rule=HostSNI(`uptrakit.example.com`)
      - traefik.tcp.routers.uptrakit.entrypoints=websecure
      - traefik.tcp.routers.uptrakit.tls.passthrough=true
      - traefik.tcp.services.uptrakit.loadbalancer.server.port=8443
```

No controller flags needed beyond the defaults for passthrough mode.

## L7 TLS Termination

Traefik terminates TLS, verifies client certs against the controller CA, and forwards cert info via headers.

### Docker Compose

```yaml
services:
  traefik:
    image: traefik:v3
    command:
      - --entrypoints.websecure.address=:443
      - --providers.docker=true
      - --providers.file.filename=/etc/traefik/dynamic.yaml
    ports:
      - "443:443"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - ./traefik-dynamic.yaml:/etc/traefik/dynamic.yaml:ro
      - ./ca.crt:/etc/traefik/ca.crt:ro

  uptrakit:
    image: uptrakit-controller
    command:
      - --trusted-proxy=172.16.0.0/12
      - --forwarded-client-cert-info-header=X-Forwarded-Tls-Client-Cert-Info
    labels:
      - traefik.enable=true
      - traefik.http.routers.uptrakit.rule=Host(`uptrakit.example.com`)
      - traefik.http.routers.uptrakit.entrypoints=websecure
      - traefik.http.routers.uptrakit.tls=true
      - traefik.http.routers.uptrakit.tls.options=mtls@file
      - traefik.http.routers.uptrakit.middlewares=clientcert@file
      - traefik.http.services.uptrakit.loadbalancer.server.port=8443
      - traefik.http.services.uptrakit.loadbalancer.server.scheme=https
      - traefik.http.services.uptrakit.loadbalancer.serversTransport=uptrakit@file
```

### Dynamic Configuration (`traefik-dynamic.yaml`)

```yaml
tls:
  options:
    mtls:
      clientAuth:
        caFiles:
          - /etc/traefik/ca.crt
        clientAuthType: RequestClientCert

http:
  middlewares:
    clientcert:
      passTLSClientCert:
        info:
          subject:
            commonName: true
          issuer:
            commonName: true
          serialNumber: true

  serversTransports:
    uptrakit:
      rootCAs:
        - /etc/traefik/ca.crt
      serverName: "uptrakit.example.com"
```

### Notes

- Use `RequestClientCert` (not `RequireAndVerifyClientCert`) so browsers can access the web UI without a client certificate.
- `passTLSClientCert` sends the `X-Forwarded-Tls-Client-Cert-Info` header with structured Subject, Issuer, and SerialNumber fields.
- The `serversTransport` section ensures Traefik trusts the controller's internal CA when connecting to the backend.
- Replace `172.16.0.0/12` with your Docker network CIDR.

### Obtaining the CA Certificate

```bash
curl -k https://uptrakit:8443/api/v1/ca.crt -o ca.crt
```
