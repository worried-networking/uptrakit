# Code Review: uptrakit-plugin-releases-docker

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-plugin-releases-docker` (~2,603 LoC across 10 source files) provides Docker registry
integration for version checking and tag tracking. It supports Docker Hub, GitHub Container
Registry, and private registries with both basic and bearer authentication. The `ImageRef` parser
handles all six reference formats correctly, and `DockerConfig` exposes `page_size` as a
user-configurable field.

The SSRF vulnerability in the Docker registry authentication flow has been fixed: the auth realm
URL is now validated against the configured registry host before following it.

## Architecture

### Strengths

- `src/image_ref.rs` -- `ImageRef::from_str` centralizes image reference parsing for all six
  formats (official, user, GHCR, private, localhost, port).
- `src/config.rs` -- `DockerConfig` exposes `page_size` as a user-configurable field (default
  1000), unlike GitHub's hardcoded `per_page=100`.
- `src/auth.rs` -- `DockerAuth` uses `#[serde(tag = "type")]` tagged union with fixed
  discriminant set.

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- `src/config.rs` -- `DockerAuth::Basic.password` and `DockerAuth::Bearer.token` use
  `SecretString`. `expose_secret()` confined to header construction point.
- No `unsafe` blocks.

### Issues

No security issues found.

## Code Quality

### Strengths

- `src/config.rs` and `src/image_ref.rs` -- 30+ test cases covering validation, image reference
  parsing for all six formats, `ImageRef::web_url` and `server_address`, serialization
  round-trips, `DockerAuth` both variants including masking and secret restore, and
  `TrackingMode` permutations.

### Issues

No code quality issues found.

## High Availability

### Strengths

- Plugin construction is infallible after `validate()`.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- `#[serde(rename_all = "snake_case")]` consistently applied. `DockerAuth` uses
  `#[serde(tag = "type")]` correctly.
- `skip_serializing_if = "Option::is_none"` on all optional config fields.
- Zero `#[allow(clippy::...)]` suppressions.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- `TrackingMode` enum allows different tag tracking strategies (latest, semver, specific).
- `DockerAuth` tagged union is open for new auth types.

### Issues

No extensibility issues found.

## Tests

### Strengths

- `src/config.rs:227-316` -- 15+ tests cover `DockerConfig` validation (valid, page size
  bounds), `ImageRef::from_str` for all six reference formats (official, user, GHCR,
  private, localhost, port), `ImageRef::web_url` and `server_address`, serialisation
  round-trips, `DockerAuth` masking and secret restore for both Basic and Bearer variants,
  and `TrackingMode` permutations.
- `src/error.rs:59-140` -- 11 tests cover all `DockerError` variants for `Display`
  correctness, HTTP error mapping, and the `from_registry_error` conversion path.
- `src/image_ref.rs` -- Round-trip parsing tests cover every image reference format the
  plugin supports, including edge cases like `localhost` addresses and port-qualified registries.
- `src/registry.rs:151` -- One test exercises tag pagination result merging logic.

### Issues

**[MEDIUM]** `src/docker_client.rs` and `src/plugin.rs` -- The `fetch_tags` and HTTP
authentication exchange paths have no mock-server tests. The `DockerClient::fetch_tags`
method (tag listing, pagination, auth token exchange) and the `plugin.rs` `fetch_releases`
method are the primary operations of the plugin, but neither has an HTTP interaction test.
The rest of the release plugins (GitHub, npm) use `httpmock` or in-process fixtures for
this; Docker should follow the same pattern, exercising at least: successful tag list,
authentication challenge-and-response, 401 unauthenticated, and empty tag list.
