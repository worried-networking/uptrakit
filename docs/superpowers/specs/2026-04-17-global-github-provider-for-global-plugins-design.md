# Global GitHub Provider For Global Plugins Design

Date: 2026-04-17

## Summary

Introduce a host-owned global GitHub provider client for global or singleton
plugins, starting with `dashboard-icons`.

The provider is backed by one global owner-managed GitHub credentials record
(`auth_token`, `api_base_url`) and exposed only through application-layer
injection. Plugins do not read shared settings directly. The provider client is
implemented with `octocrab` behind an Uptrakit-owned abstraction and enforces
shared outbound policy for authentication, retry, and rate-limit handling across
all global GitHub consumers.

In V1, regular tenant-scoped plugins such as `releases_github` do not consume
this provider. `dashboard-icons` may still operate unauthenticated when no
global GitHub credentials are configured.

## Goals

- Allow global or singleton plugins to use one shared global GitHub credential.
- Keep shared credential resolution outside plugin code.
- Centralize GitHub outbound policy, especially rate-limit and retry handling.
- Support unauthenticated fallback for `dashboard-icons`.
- Preserve a clean upgrade path to future global GitHub consumers.

## Non-Goals

- Tenant overrides.
- Multiple GitHub provider instances.
- Making tenant-scoped plugins consume the global provider.
- Generic cross-provider or cross-plugin provider selection.
- Reworking `releases_github` in V1.

## Current Context

Today:

- Global GitHub credentials are stored in the host layer as global settings.
- The current shared-provider fallback implementation applies to regular
  GitHub-backed config materialization paths.
- `dashboard-icons` is a singleton enhancement plugin built from
  `CatalogConfig`, not a `plugin_config` instance.
- `dashboard-icons` fetches GitHub-backed data without a shared provider
  abstraction.
- GitHub request logic and rate-limit behavior are still largely consumer-local.

This creates two problems:

- global plugins have no clean shared-credential injection seam
- GitHub API consumers do not coordinate rate limits centrally

## Decision

V1 narrows the shared provider concept:

- there is one global owner-managed GitHub credentials record
- it is usable only by global or singleton plugins
- the host application builds one shared GitHub provider client from it
- global plugins declare that they consume the global GitHub provider
- the host injects that provider into eligible global plugins at construction
  time
- `dashboard-icons` is the first consumer
- `dashboard-icons` still works without credentials, using unauthenticated
  requests

Regular plugins, including tenant-scoped ones like `releases_github`, do not
consume the global provider in V1.

## Architecture

### 1. Global Provider Storage

The persisted data remains simple and GitHub-specific in V1:

- `auth_token`
- `api_base_url`

Responsibilities:

- load/store the global GitHub credentials record
- encrypt the token at rest
- expose typed load/store helpers

Responsibilities explicitly excluded:

- deciding which plugins may use the provider
- building HTTP clients
- handling rate limits or retry policy

Storage remains dumb. Consumer applicability is moved out of storage code and
into host-layer runtime construction.

### 2. Host-Owned GitHub Provider Runtime

Add an application-layer GitHub provider runtime that:

- loads the global GitHub credentials record
- builds one shared GitHub provider client
- caches it for process-wide reuse
- exposes a GitHub-specific trait or handle owned by Uptrakit

This runtime owns:

- auth header injection
- base URL selection
- `octocrab` construction
- retry policy
- rate-limit coordination
- metrics and tracing

`octocrab` is an implementation detail behind the Uptrakit abstraction, not the
plugin-facing contract.

### 3. Global Plugin Consumer Declaration

Global or singleton plugins need a declaration that they consume the global
GitHub provider.

This declaration must be:

- plugin metadata
- provider-aware
- limited to global-plugin construction paths in V1
- independent from storage code

The generic plugin core should only carry provider-agnostic metadata such as an
opaque provider identifier or consumer declaration. It must not grow
GitHub-specific types.

### 4. Injection Into Global Plugin Construction

The provider handle is injected during singleton/global plugin construction.

For `dashboard-icons`:

- the host sees that the plugin consumes the global GitHub provider
- it resolves the shared GitHub provider handle
- it injects the handle through singleton construction context

The plugin never:

- reads shared settings
- reads the DB
- builds its own shared rate-limit policy

This keeps `dashboard-icons` testable: unit tests can inject a fake GitHub
provider handle.

## Provider Client Design

The host-owned abstraction should be GitHub-specific, but only outside generic
plugin infrastructure.

Expected behavior:

- provide a small GitHub-oriented interface suited to current global consumers
- expose typed operations or request helpers for GitHub REST access
- hide raw credential resolution from plugins
- hide `octocrab` from plugins

The provider client is implemented with `octocrab` because it is an actively
maintained GitHub API crate and supports injecting a custom service or layer
stack. That allows Uptrakit to keep ownership of middleware and policy while
still using a supported GitHub client internally.

## Rate Limiting And Retry Policy

All global GitHub consumers share one provider client policy surface.

The provider runtime must centralize:

- primary rate-limit handling
- `Retry-After` handling
- secondary-rate-limit backoff
- bounded retries with jitter
- cooldown coordination across consumers

The provider client is the only retry owner. Plugins must not add their own
GitHub-specific retry loops on top.

Shared state should not be keyed as one undifferentiated "GitHub bucket". Even
in V1, it should be keyed at least by:

- `api_base_url`
- credential fingerprint

This keeps the model compatible with future multi-instance or tenant-specific
provider resolution.

The shared policy should:

- avoid a naive global mutex around all GitHub traffic
- gate only when cooldown or budget state requires it
- surface transient provider-throttled errors consistently

## `dashboard-icons` Behavior

`dashboard-icons` becomes the first global consumer of the GitHub provider.

Behavior rules:

- if global GitHub credentials exist, use the injected authenticated GitHub
  provider client
- if credentials do not exist, continue to function with unauthenticated
  requests
- if rate-limited or GitHub is temporarily unavailable, preserve existing cache
  behavior and fail gracefully

The plugin should not need to know whether it is running authenticated or
unauthenticated. That distinction is owned by the provider runtime.

## Observability

Add provider-level metrics and tracing for:

- request count
- retry count
- throttled count
- cooldown duration
- authentication failures
- consumer identity, such as `dashboard-icons`

This is necessary because one shared client now mediates multiple consumers.

## Testing

### Unit Tests

- provider storage load/store helpers
- provider runtime creation with and without credentials
- deterministic retry and cooldown behavior with injected clock/transport
- `dashboard-icons` using a fake provider handle

### Integration Tests

- global provider credentials configured -> `dashboard-icons` uses provider path
- no global provider credentials -> `dashboard-icons` still works
- two global consumers share one cooldown window
- invalid credentials produce durable auth failures without aggressive retry

### Regression Requirements

- no plugin code path may read shared settings directly
- no storage helper may decide plugin applicability

## Migration

### V1

- keep the global GitHub settings persistence shape
- narrow its semantics to "global GitHub provider for global plugins"
- move consumer applicability out of storage helpers
- add provider runtime and injection path
- migrate `dashboard-icons` to use the injected provider
- leave `releases_github` unchanged

### V2-Compatible Seams

V1 should leave room for:

- multiple GitHub provider instances
- tenant-scoped selection
- provider instance resolution keyed by more than one credentials record

The critical V2 preparation is that plugin-facing contracts do not assume a
single hardcoded global GitHub settings source.

## Rejected Alternatives

### Make `dashboard-icons` a `plugin_config` Plugin

Rejected because it changes the plugin's semantics from one global enhancement
to one or more assignable config instances without a V1 need.

### Let Plugins Read Shared Settings Directly

Rejected because it violates the boundary, harms testability, and makes future
provider evolution expensive.

### Make Regular Plugins Consume The Global Provider In V1

Rejected to keep scope small and avoid locking tenant-scoped plugins into the
wrong provider-selection model before V2.

### Use Raw `reqwest` Instead Of A GitHub Crate

Possible, but not preferred. `octocrab` gives a supported GitHub API surface
while still allowing Uptrakit to own the service stack and policy through an
abstraction layer.

## Concrete V1 Outcome

After V1:

- one global GitHub credentials record exists
- one host-owned GitHub provider client exists per process
- `dashboard-icons` can use it
- future global plugins can opt in the same way
- GitHub rate limiting for global consumers is coordinated centrally
- tenant-scoped plugins remain unchanged
