# Docker Plugin

The `releases_docker` plugin tracks container image updates in Docker/OCI-compatible registries.
It monitors the SHA-256 manifest digest of a specific tag (e.g. `latest`) and reports an update
when the remote digest changes — regardless of whether the tag name itself changes. It can also
discover running and stopped containers on the agent host and auto-populate your software catalog.

## Package Identifier Format

A Docker plugin's `package_identifier` is an **image reference** in standard Docker format:

```text
[registry/][repository/]image[:tag]
```

| Example | Registry | Repository | Tag |
| --- | --- | --- | --- |
| `nginx` | Docker Hub | `library/nginx` | `latest` (implied) |
| `myuser/myapp:v2` | Docker Hub | `myuser/myapp` | `v2` |
| `ghcr.io/owner/app:main` | `ghcr.io` | `owner/app` | `main` |
| `myhost:5000/app:latest` | `myhost:5000` | `app` | `latest` |

The tag embedded in `package_identifier` is used as the default tag to track. It can be
overridden with the `tracked_tag` config field.

## How Version Tracking Works

The plugin fetches the `Docker-Content-Digest` manifest header from the OCI registry using an
authenticated `HEAD` request. This digest (`sha256:…`) is used as the "version" — so an update
is available whenever the remote digest differs from the locally installed image digest, even if
the tag name (e.g. `latest`) has not changed.

- **Installed version** — read from the local Docker daemon via `inspect_image` → `RepoDigests`.
  If the image is not present locally, no installed version is reported.
- **Available version** — the manifest digest fetched from the registry.

## After Pulling: Container Recreation

After successfully pulling a new image, the plugin automatically recreates all containers that use
that image, preserving their full configuration (environment variables, volumes, port bindings,
labels, network settings, resource limits, etc.).

- **Running containers** — stopped, removed, recreated from the saved configuration, and started
  again.
- **Stopped containers** — removed and recreated from the saved configuration, but **not**
  started (they remain stopped after the update).
- **Auto-remove containers** (`AutoRemove = true`) — skipped; they manage their own lifecycle.

When `compose_restart` or `post_pull_command` is configured, the automatic container recreation
is **skipped** in favour of those mechanisms.

## Configuration Fields

```json
{
  "tracked_tag": "latest",
  "auth": null,
  "docker_host": null,
  "compose_restart": null,
  "post_pull_command": null
}
```

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `tracked_tag` | String \| `null` | No | tag from `package_identifier` | Override the tag to track (e.g. `"stable"`) |
| `auth` | `DockerAuth` \| `null` | No | `null` | Registry authentication credentials |
| `docker_host` | String \| `null` | No | `null` | Docker daemon endpoint override (see [Remote Docker via SSH](#remote-docker-via-ssh)) |
| `compose_restart` | `ComposeRestartConfig` \| `null` | No | `null` | Run `docker compose up` after pulling instead of auto-recreating containers |
| `post_pull_command` | String \| `null` | No | `null` | Custom shell command to run after pulling (disables auto-recreate) |

An empty config object `{}` is valid. No field is required.

### DockerAuth

```json
{ "type": "basic", "username": "myuser", "password": "mytoken" }
```

```json
{ "type": "bearer", "token": "myregistrytoken" }
```

Credentials are masked in API responses (`"***"` replaces the secret value).

### ComposeRestartConfig

Runs `docker compose up` (with optional service name and file path) after a successful image pull,
instead of the automatic container recreation.

- When any containers using the image were **running** before the pull: `docker compose up -d`
  (recreate and start).
- When all containers were **stopped** before the pull: `docker compose up --no-start`
  (recreate without starting).

```json
{
  "compose_restart": {
    "compose_file": "/opt/myapp/docker-compose.yml",
    "service": "myapp",
    "working_dir": "/opt/myapp"
  }
}
```

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `compose_file` | String \| `null` | No | Path to the Compose file (`-f` flag) |
| `service` | String \| `null` | No | Service name to restart (omit to restart all services) |
| `working_dir` | String \| `null` | No | Working directory for the `docker compose` command |

### post_pull_command

A custom shell command executed after a successful pull. Supports the following placeholders
(shell-escaped automatically):

| Placeholder | Value |
| --- | --- |
| `{image}` | Image name without tag (e.g. `ghcr.io/owner/app`) |
| `{tag}` | Tag pulled (e.g. `latest`) |
| `{digest}` | SHA digest of the locally installed image (e.g. `sha256:abc…`) |

`compose_restart` and `post_pull_command` can be set simultaneously; `compose_restart` runs
first. When either is set, automatic container recreation is disabled.

## Autodiscovery

The Docker plugin supports **local software discovery**. Before querying the Docker daemon the
agent runs a host-compatibility check by pinging the daemon directly (`GET /_ping` via bollard).
If the daemon is not reachable (not installed, not running, or SSH tunnel failure), the plugin
reports zero discoveries and no error — it is expected that some hosts will not have Docker running.
This prevents spurious connection errors on non-Docker hosts.

When discovery runs, the agent queries the local Docker daemon for all containers (running and
stopped) via `list_containers`. For each container image that is not a bare SHA digest:

1. The image reference is normalised (missing tag defaults to `latest`).
2. `inspect_image` is called to retrieve the local SHA digest from `RepoDigests`.
3. Images with no registry provenance (locally built images with no `RepoDigests`) are skipped.
4. Container names are stored as extra metadata (`{"containers": ["my-container"]}`).

Name derivation:

- **Single container** for an image → container name (leading `/` stripped)
- **Multiple containers** for the same image → `"image:tag"` format

Auto-created plugin config name: **`"Docker"`** (one config per tenant, shared across all hosts).

Discovered items use digest tracking semantics; `package_identifier` is set to the full image
reference including tag.

## Remote Docker via SSH

> **Note:** Remote Docker via SSH is only available when using the **SSH agent** binary
> (`uptrakit-agent-ssh`). It is not available in the standard `uptrakit-agent`.

When `agent-ssh` connects to a remote host, it automatically tunnels Docker API traffic over the
existing SSH session. The plugin starts a local Unix socket proxy that runs
`docker system dial-stdio` on the remote host, then connects bollard to that proxy socket.
You do not need to set `docker_host` manually; the tunnel is established automatically when
the executor supports stdio tunnels and no explicit `docker_host` is configured.

This approach avoids spawning a second SSH connection (which can fail on hosts without the system
`ssh` binary, such as Flatcar Container Linux) and reuses the authenticated russh session that is
already established.

To manually override the Docker daemon endpoint, set `docker_host` in the plugin config:

```json
{ "docker_host": "unix:///var/run/docker.sock" }
```

Setting `docker_host` disables the automatic tunnel — the plugin connects directly to the specified
endpoint instead.

Supported endpoint formats:

| Format | Description |
| --- | --- |
| (omitted) | Auto-tunnel via `docker system dial-stdio` when SSH, otherwise platform default |
| `unix:///path/to/docker.sock` | Unix socket at a custom path |
| `http://host:2375` | Unencrypted HTTP |

## Example Configurations

### Track a Docker Hub image by digest

```json
{
  "tracked_tag": "latest"
}
```

Package identifier: `nginx`

### Track a private GHCR image by digest

```json
{
  "tracked_tag": "stable",
  "auth": { "type": "bearer", "token": "ghp_…" }
}
```

Package identifier: `ghcr.io/myorg/myapp`

### Track an image and restart via Docker Compose

```json
{
  "tracked_tag": "latest",
  "compose_restart": {
    "working_dir": "/opt/myapp",
    "service": "web"
  }
}
```

Package identifier: `ghcr.io/myorg/myapp`

### Track an image and run a custom post-pull command

```json
{
  "tracked_tag": "latest",
  "post_pull_command": "systemctl restart myapp"
}
```

Package identifier: `ghcr.io/myorg/myapp`

## Backward Compatibility

Configs created before this change may contain `tracking_mode`, `tag_patterns`,
`tag_strip_prefix`, `include_prereleases`, or `page_size` fields from the old semver-based
tracking mode. These fields are silently ignored — existing configs will continue to work and
will automatically use digest-based tracking.

## Related Documentation

- [Plugin Configurations](../plugin-configs.md) — creating and managing plugin configs
- [Autodiscovery](../autodiscovery.md) — discovery workflow and ignore rules
- [Plugin Guidelines](../../development/plugin-guidelines.md) — implementation details
- [SSH Agent](../../architecture/ssh-agent.md) — SSH agent setup for remote host management
