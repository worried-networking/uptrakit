# Docker Plugin

The `docker` plugin tracks container images in Docker/OCI-compatible registries. It supports two
tracking modes: semver-based tag tracking and SHA digest-based tracking. It can also discover
running and stopped containers on the agent host and auto-populate your software catalog.

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

The tag in the `package_identifier` is used as the base reference for digest tracking; in semver
mode the tag portion is typically omitted (or set to a common value like `latest`).

## Tracking Modes

### SemverTags (default)

In `semver_tags` mode the plugin lists all tags from the registry, filters them by
`tag_patterns`, strips `tag_strip_prefix`, parses the remaining string as a semver version, and
returns the sorted list of releases.

- Pre-release versions are excluded by default (`include_prereleases: false`).
- Tags that cannot be parsed as semver are silently skipped.
- Installed version detection is not performed in this mode (`detect_installed_version` always
  returns `None` — the local image version is not tracked).

### DigestTracking

In `digest_tracking` mode the plugin fetches the manifest digest for a specific tag
(`tracked_tag`, default `"latest"`) and treats the SHA digest as the "version". This is useful for
mutable tags like `latest` that do not carry a stable version identifier.

- The plugin checks whether the local Docker daemon has the image by calling `inspect_image`.
- If the image is present locally, its `RepoDigests` field is compared against the registry
  manifest digest to determine whether an update is available.
- The "version" displayed in Uptrakit is the `sha256:…` digest.

## Configuration Fields

```json
{
  "tracking_mode": "semver_tags",
  "tag_patterns": ["^v?[0-9]+\\.[0-9]+\\.[0-9]+$"],
  "tag_strip_prefix": "v",
  "include_prereleases": false,
  "tracked_tag": "latest",
  "page_size": 1000,
  "auth": null,
  "docker_host": null,
  "compose_restart": null,
  "post_pull_command": null
}
```

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `tracking_mode` | `"semver_tags"` \| `"digest_tracking"` | No | `"semver_tags"` | How to resolve upstream versions |
| `tag_patterns` | `Vec<String>` | No | `[]` | Regex patterns to filter tags (OR logic; empty = all) |
| `tag_strip_prefix` | String | No | `"v"` | Prefix to strip from tag names before semver parsing |
| `include_prereleases` | bool | No | `false` | Include pre-release versions in semver mode |
| `tracked_tag` | String | No | `"latest"` | Tag to track in digest mode |
| `page_size` | u32 | No | `1000` | Maximum number of tags to fetch per registry request |
| `auth` | `DockerAuth` \| `null` | No | `null` | Registry authentication credentials |
| `docker_host` | String \| `null` | No | `null` | Docker daemon endpoint override (see [Remote Docker via SSH](#remote-docker-via-ssh)) |
| `compose_restart` | `ComposeRestartConfig` \| `null` | No | `null` | Run `docker compose up -d` after pulling |
| `post_pull_command` | String \| `null` | No | `null` | Custom shell command to run after pulling |

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

Runs `docker compose up -d` (with optional service name and file path) after a successful image
pull.

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
| `{tag}` | Tag pulled (e.g. `v1.2.3`) |
| `{digest}` | SHA digest of the locally installed image (e.g. `sha256:abc…`) |

`compose_restart` and `post_pull_command` can be set simultaneously; `compose_restart` runs first.

## Autodiscovery

The Docker plugin supports **local software discovery**. When discovery runs, the agent queries
the local Docker daemon for all containers (running and stopped) via `list_containers`. For each
container image that is not a bare SHA digest:

1. The image reference is normalised (missing tag defaults to `latest`).
2. `inspect_image` is called to retrieve the local SHA digest from `RepoDigests`.
3. Images with no registry provenance (locally built images with no `RepoDigests`) are skipped.
4. Container names are stored as extra metadata (`{"containers": ["my-container"]}`).

Name derivation:

- **Single container** for an image → container name (leading `/` stripped)
- **Multiple containers** for the same image → `"image:tag"` format

Auto-created plugin config name: **`"Docker"`** (one config per tenant, shared across all hosts).

Discovered items use `digest_tracking` semantics by default; `package_identifier` is set to the
full image reference including tag.

## Remote Docker via SSH

> **Note:** Remote Docker via SSH is only available when using the **SSH agent** binary
> (`uptrakit-agent-ssh`). It is not available in the standard `uptrakit-agent`.

When `agent-ssh` connects to a remote host, it automatically configures the Docker plugin to
reach the remote host's Docker daemon over the same SSH connection. You do not need to set
`docker_host` manually for SSH agent deployments; the connection context is injected automatically.

To manually override the Docker daemon endpoint, set `docker_host` in the plugin config:

```json
{ "docker_host": "ssh://user@host:22" }
```

SSH connections require the `ssh` Cargo feature to be enabled on the `uptrakit-plugin-docker`
crate (automatically enabled in `uptrakit-agent-ssh`). The SSH client uses the default SSH key
resolution (SSH agent or `~/.ssh/id_*`).

Supported endpoint formats:

| Format | Description |
| --- | --- |
| (omitted) | Platform default (`/var/run/docker.sock` on Linux, or `DOCKER_HOST` env var) |
| `unix:///path/to/docker.sock` | Unix socket at a custom path |
| `http://host:2375` | Unencrypted HTTP |
| `ssh://user@host:port` | SSH tunnel (SSH agent only) |

## Example Configurations

### Track nginx with semver tags

```json
{
  "tracking_mode": "semver_tags",
  "tag_patterns": ["^[0-9]+\\.[0-9]+\\.[0-9]+-alpine$"],
  "tag_strip_prefix": "",
  "page_size": 100
}
```

Package identifier: `nginx`

### Track a private GHCR image by digest

```json
{
  "tracking_mode": "digest_tracking",
  "tracked_tag": "stable",
  "auth": { "type": "bearer", "token": "ghp_…" },
  "post_pull_command": "systemctl restart myapp"
}
```

Package identifier: `ghcr.io/myorg/myapp`

### Track an image and restart via Docker Compose

```json
{
  "tracking_mode": "digest_tracking",
  "tracked_tag": "latest",
  "compose_restart": {
    "working_dir": "/opt/myapp",
    "service": "web"
  }
}
```

Package identifier: `ghcr.io/myorg/myapp`

## Related Documentation

- [Plugin Configurations](../plugin-configs.md) — creating and managing plugin configs
- [Autodiscovery](../autodiscovery.md) — discovery workflow and ignore rules
- [Plugin Guidelines](../../development/plugin-guidelines.md) — implementation details
- [SSH Agent](../../architecture/ssh-agent.md) — SSH agent setup for remote host management
