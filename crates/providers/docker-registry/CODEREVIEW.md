# Code Review: uptrakit-provider-docker-registry

## Summary

Docker/OCI Registry provider crate (~1200 lines across 7 source files) implementing the `Provider` trait for tracking container image versions. Supports semver tag tracking and digest-based tracking, full OCI Bearer authentication flow, smart registry inference (Docker Hub, GHCR, private), and both `docker pull` and `docker compose pull` update execution.

## Architecture

- **Module structure**: `lib.rs` re-exports from `provider.rs`, `config.rs`, `registry_client.rs`, `auth.rs`, `tag.rs`, `types.rs`, `error.rs`.
- **Public API surface**: `DockerRegistryProvider`, `DockerRegistryConfig`, `DockerRegistryError`.
- **Dependency choices**: `uptrakit-provider-core` (sole uptrakit dependency -- correct), `reqwest` (HTTP), `url`, `serde`/`serde_json`, `rootcause`/`thiserror`.
- **Layering**: Leaf provider crate. Depends only on `uptrakit-provider-core`.

## Security and Safety

- **OCI authentication**: Complete Bearer token flow with challenge parsing, token caching with expiry, and retry-on-401. Production-quality implementation.
- **Injection prevention**: `execute_update` uses `run_command_exec` (direct exec, no shell) for `docker pull`, preventing injection via crafted image names.
- **Secret handling**: Supports both Basic and Bearer auth with proper masking/restoration, including auth type change detection.
- **Credential redaction**: `DockerRegistryConfig` auth credentials are masked in API responses.
- No `unsafe` code.
- No `unwrap`/`panic` in non-test code.

## Code Quality

- **Error handling**: `DockerRegistryError` enum with multiple variants. Uses `rootcause::Report` wrapper.
- **Test coverage**: 40+ tests covering configuration, registry resolution, repository resolution, tag filtering/sorting, serialization, secret management.
- **Smart registry inference**: Correctly handles Docker Hub official images, Docker Hub user images, GHCR, private registries, localhost, and port-based registries.

## Coding Standards Compliance

- Typed error enum with `thiserror` + `rootcause::Report` -- compliant.
- `Result<T>` type alias defined.
- `impl_report_conversion!` used for cross-boundary errors.
- No `#[allow()]` directives.

## Findings

| ID | Severity | Category | Description | File:Line |
| --- | --- | --- | --- | --- |
| DOCK-01 | Minor | Code Quality | `execute_update` reads `restart_command` from raw `provider_config` JSON instead of from `self.config`. This field is not declared in `DockerRegistryConfig`, creating an undocumented configuration surface. Same issue as GH-01 in the GitHub provider. | `src/provider.rs:181` |
| DOCK-02 | Minor | Code Quality | `strip_tag_prefix` is duplicated identically in `crates/providers/github/src/tag.rs`. Should be extracted to `uptrakit-provider-core`. | `src/tag.rs` |
| DOCK-03 | Minor | Scalability | Tag list pagination is single-page only. `list_tags()` requests `?n={page_size}` but does not follow the `Link` header for next pages. Repositories with many tags (e.g., `nginx`) may not return all tags. | `src/registry_client.rs` |
| DOCK-04 | Info | Code Quality | `std::sync::Mutex` is used for `cached_token` in `RegistryAuth`. Since this is used in an async context, `tokio::sync::Mutex` would be conventional. However, the lock is never held across `.await` points, so `std::sync::Mutex` is correct and more efficient. | `src/auth.rs:28` |

## Verdict

**Pass.** Production-quality OCI registry integration with thorough authentication, injection prevention, and smart registry inference. The undocumented `restart_command` (DOCK-01) and duplicated `strip_tag_prefix` (DOCK-02) are the most actionable findings.
