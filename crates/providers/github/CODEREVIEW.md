# Code Review: uptrakit-provider-github

## Summary

GitHub Releases provider crate (~900 lines across 6 source files) implementing the `Provider` trait for tracking software releases via the GitHub API. Supports authenticated and unauthenticated access, asset filtering via regex, configurable tag prefix stripping, and SSRF protection.

## Architecture

- **Module structure**: `lib.rs` re-exports from `provider.rs`, `config.rs`, `tag.rs`, `types.rs`, `error.rs`.
- **Public API surface**: `GitHubProvider`, `GitHubConfig`, `GitHubError`.
- **Dependency choices**: `uptrakit-provider-core` (sole uptrakit dependency -- correct), `reqwest` (HTTP), `regex` (asset filtering), `url` (URL construction/validation), `serde`/`serde_json`, `rootcause`/`thiserror`.
- **Layering**: Leaf provider crate. Depends only on `uptrakit-provider-core`.

## Security and Safety

- **SSRF protection**: `is_private_host()` checks for private IPv4/IPv6 ranges, localhost, `.local`, `.internal`, `.localhost` TLDs. Applied to `api_base_url` during validation.
- **Input validation**: `GitHubConfig::validate()` rejects empty fields, path traversal (`..`), slash injection, and non-HTTPS `api_base_url`.
- **Shell injection prevention**: `execute_update` uses `shell_escape()` for user-supplied values in install commands.
- **Rate limit awareness**: Logs warnings when GitHub API rate limits are low; returns specific error on exhaustion.
- **Secret handling**: `SecretString` for `auth_token`, proper masking/restoration.
- No `unsafe` code.
- No `unwrap`/`panic` in non-test code.

## Code Quality

- **Error handling**: `GitHubError` enum with 4 variants. Uses `rootcause::Report` wrapper.
- **Test coverage**: 22+ tests covering conversion, filtering, URL construction, date parsing, configuration validation, secret masking/restoration.
- **Clean dependency chain**: Only depends on `uptrakit-provider-core`.

## Coding Standards Compliance

- Typed error enum with `thiserror` + `rootcause::Report` -- compliant.
- `Result<T>` type alias defined.
- `impl_report_conversion!` used for cross-boundary errors.
- No `#[allow()]` directives.

## Findings

| ID | Severity | Category | Description | File:Line |
| --- | --- | --- | --- | --- |
| ~~GH-01~~ | ~~Minor~~ | ~~Code Quality~~ | ~~`execute_update` reads `install_command` from raw JSON.~~ **FIXED.** `install_command: Option<String>` added to `GitHubConfig`. Provider uses `self.config.install_command`. Raw JSON parameter removed from trait. | `src/provider.rs` |
| GH-02 | Minor | Code Quality | `detect_installed_version` overrides the default to return `Ok(None)`. Since GitHub is a remote-only provider, the default error return would be more honest. Either remove the override or add a doc comment explaining the semantics. | `src/provider.rs:257` |
| GH-03 | Minor | Code Quality | HTTP errors conflated under `ProviderError::Configuration`. Both rate limit errors and API errors are wrapped as `ProviderError::Configuration` in the trait implementation, losing the specificity of `GitHubError::RateLimited` and `GitHubError::ApiError`. | `src/provider.rs` |
| GH-04 | Minor | Code Quality | `strip_tag_prefix` is duplicated identically in `crates/providers/docker-registry/src/tag.rs`. Should be extracted to `uptrakit-provider-core`. | `src/tag.rs` |
| GH-05 | Info | Scalability | GitHub releases API requests `per_page=100` but does not follow `Link` header pagination. Repositories with more than 100 releases will silently miss older releases. | `src/provider.rs` |

## Verdict

**Pass.** Well-secured provider with thorough input validation, SSRF protection, and clean dependency chain. The undocumented `install_command` field (GH-01) and duplicated `strip_tag_prefix` (GH-04) are the most actionable findings.
