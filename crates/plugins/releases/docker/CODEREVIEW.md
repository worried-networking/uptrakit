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

The main concern is a Critical SSRF vulnerability in the Docker registry authentication flow where
an attacker-controlled `realm` URL from a `WWW-Authenticate` header is followed without domain
validation.

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

**[CRITICAL]** `src/auth.rs:60-86` -- SSRF via attacker-controlled registry auth realm URL.
The Docker registry authentication flow reads the `realm` URL from the `WWW-Authenticate`
response header of an HTTP 401 response. The client then issues an HTTP GET to that realm URL
with the registry credentials as query parameters. There is no validation that the realm URL
belongs to the expected registry domain. A malicious registry (or DNS-hijacked registry) can
return a `realm` pointing to an internal metadata endpoint (e.g.,
`http://169.254.169.254/latest/meta-data/`), causing credential exfiltration. Fix: validate
that the `realm` URL shares the same host as the configured registry URL.

## Code Quality

### Strengths

- `src/config.rs` and `src/image_ref.rs` -- 30+ test cases covering validation, image reference
  parsing for all six formats, `ImageRef::web_url` and `server_address`, serialization
  round-trips, `DockerAuth` both variants including masking and secret restore, and
  `TrackingMode` permutations.

### Issues

**[LOW]** `src/auth.rs` -- `Mutex::lock().unwrap()` on `cached_token` uses
`std::sync::Mutex` in an async context. Risks blocking the Tokio runtime thread if contended.
`tokio::sync::Mutex` would be idiomatic.

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
