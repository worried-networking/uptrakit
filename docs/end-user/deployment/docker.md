# Docker Deployment

This guide covers running Uptrakit with Docker Compose. Pre-built images are published to
`ghcr.io/worried-networking/uptrakit-*` on every push to `main` and for tagged releases.

## Prerequisites

- Docker Engine 24+ with Compose V2
- A 64-character hex master encryption key (generate with `openssl rand -hex 32`)

## Quick Start

```bash
# Clone the repository (or download docker-compose.yml + .env.example)
git clone https://github.com/worried-networking/uptrakit.git
cd uptrakit

# Create environment file
cp .env.example .env

# Generate and set the master key
echo "UPTRAKIT_MASTER_KEY=$(openssl rand -hex 32)" >> .env

# Start the controller (SQLite, embedded scheduler)
docker compose up -d
```

The controller is available at `https://localhost:8443`. On first start it prints a one-time registration token
to the logs:

```bash
docker compose logs controller | grep "registration token"
```

## Profiles

Docker Compose profiles control which services run alongside the controller.

| Profile | Services started | Description |
| --- | --- | --- |
| *(default)* | controller | Controller with SQLite and embedded scheduler |
| `postgres` | controller, postgres | Controller with PostgreSQL |
| `mqtt` | controller, mqtt | Controller + MQTT/HA bridge |
| `ssh` | controller, agent-ssh | Controller + SSH agent |
| `scheduler` | controller, scheduler, nats | Controller + external scheduler + NATS |
| `full` | all services | Everything |

```bash
# Controller with PostgreSQL
docker compose --profile postgres up -d

# All services
docker compose --profile full up -d
```

## PostgreSQL

To use PostgreSQL instead of SQLite:

```bash
# In .env
DB_URL=postgresql://uptrakit:changeme@postgres:5432/uptrakit
POSTGRES_PASSWORD=changeme
```

```bash
docker compose --profile postgres up -d
```

## Auto-enrollment

Bootstrap enrollment tokens allow services to auto-enroll without manual token creation.
Set shared secrets in `.env`; the controller hashes them at startup and creates short-lived tokens.

```bash
# In .env — generate random secrets
ENROLLMENT_TOKEN=$(openssl rand -hex 16)
SYSTEM_ENROLLMENT_TOKEN=$(openssl rand -hex 16)

# For --profile full (mqtt + scheduler = 2 system services)
SYSTEM_ENROLLMENT_TOKEN_MAX_USES=2
```

The controller creates bootstrap tokens with 1 use and 5-minute TTL by default. Each service presents the same
plaintext secret to enroll automatically. Adjust `*_MAX_USES` and `*_TTL` as needed.

### Token flow

1. Controller starts, reads `UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN` and `UPTRAKIT_BOOTSTRAP_SYSTEM_ENROLLMENT_TOKEN`
2. Hashes each with Argon2id and inserts a token named "bootstrap" (if none exists)
3. Services start, present the plaintext via `UPTRAKIT_ENROLLMENT_TOKEN` during WebSocket enrollment
4. Controller verifies the hash, enrolls the service, increments the use counter
5. After max uses are reached or TTL expires, the bootstrap token becomes inactive

## NATS (external scheduler)

Required when using `--profile scheduler` or `--profile full`:

```bash
# In .env
NATS_URL=nats://nats:4222
```

NATS JetStream is used for cross-controller messaging and external scheduler communication.

## OIDC

To bootstrap an OIDC provider at startup:

```bash
# In .env
OIDC_ISSUER_URL=https://auth.example.com
OIDC_CLIENT_ID=uptrakit
OIDC_CLIENT_SECRET=your-secret
```

The controller creates the OIDC provider on first start and skips creation on subsequent restarts.

## Volumes

Each service uses named volumes for persistent data:

| Volume | Service | Contents |
| --- | --- | --- |
| `controller-config` | controller | CA certificates, TLS certificates |
| `controller-state` | controller | SQLite database, JWT signing key |
| `postgres-data` | postgres | PostgreSQL data |
| `nats-data` | nats | JetStream storage |
| `scheduler-state` | scheduler | Enrollment state |
| `mqtt-state` | mqtt | Enrollment state |
| `agent-ssh-state` | agent-ssh | Enrollment state, SSH keys |

## Available Images

| Image | Description |
| --- | --- |
| `uptrakit-controller` | Controller with embedded frontend and scheduler |
| `uptrakit-controller-swagger` | Controller with Swagger UI enabled |
| `uptrakit-scheduler` | External scheduler |
| `uptrakit-mqtt` | MQTT/Home Assistant bridge |
| `uptrakit-agent-ssh` | SSH agent |
| `uptrakit-cli` | CLI tool |

All images are published for `linux/amd64` and `linux/arm64`.

## Building Locally

```bash
# Build controller image
docker compose build controller

# Build all images
docker compose --profile full build
```

To build individual images directly:

```bash
docker build -f docker/Dockerfile \
  --build-arg PACKAGE=uptrakit-controller \
  --build-arg BINARY=uptrakit-controller \
  --build-arg FEATURES=embed-frontend,db-all,oidc,embedded-scheduler,nats,notifications-all \
  -t uptrakit-controller .

# Single-tenant with embedded agent (manages the controller host):
docker build -f docker/Dockerfile \
  --build-arg PACKAGE=uptrakit-controller \
  --build-arg BINARY=uptrakit-controller \
  --build-arg FEATURES=embed-frontend,db-all,oidc,embedded-scheduler,embedded-agent,notifications-all \
  -t uptrakit-controller-with-agent .
```

## Upgrading

```bash
docker compose pull
docker compose up -d
```

## Related Documentation

- [Reverse proxy deployment](reverse-proxy.md) — running behind Nginx, Traefik, etc.
- [External scheduler deployment](external-scheduler.md) — standalone scheduler binary
- [NATS deployment](nats.md) — NATS JetStream for multi-controller HA
- [Docker development guide](../../development/docker.md) — building and testing images locally
- [Secrets and encryption](../../security/secrets-and-encryption.md) — master key management
