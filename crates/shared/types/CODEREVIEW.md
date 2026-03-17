# Code Review: `uptrakit-shared-types`

- Review date: 2026-03-17
- Scope: current-state review (full 14-dimension)

## Summary

The crate is stable, well-tested, and follows the workspace coding standards for wire-safe enums,
`#[non_exhaustive]`, and SSRF protection. The primary concern remains structural: the crate
aggregates too many unrelated domains behind a single boundary, which inflates rebuild scope
and review surface.

## Strengths

- Wire-safe enum patterns (`PluginType`, `AttestationStatus`, `OutputStreamType`, etc.) are
  correctly implemented with `Other(String)`, infallible `Deserialize`, `From<String>`, and
  `as_str()`.
- `SsrfSafeResolver` provides defence-in-depth against DNS rebinding with a clean
  restrictive/permissive mode switch. Test coverage includes public hostname resolution,
  localhost blocking, and permissive mode.
- `is_private_ip` covers IPv4 private (RFC 1918), loopback, link-local, CGNAT, and unspecified
  ranges plus IPv6 loopback, unspecified, ULA, and link-local.
- `is_private_host` covers DNS names (`localhost`, `*.local`, `*.internal`, `*.localhost`) in
  addition to IP literals.
- `webpki_client_config()` uses a per-call `CryptoProvider` instance, avoiding dependence on
  a global default.
- `danger_accept_any_cert_client_config()` still verifies TLS handshake signatures, preventing
  trivial MITM even when certificate chain verification is disabled.
- `SecretString` wraps sensitive values with redacted `Debug`/`Display`.
- Serialization roundtrip and forward-compatibility tests are thorough for `PluginType`,
  `AttestationStatus`, `ReleaseInfo`, and `ReleaseAsset`.

## Active Findings

### [MEDIUM] The crate still mixes too many unrelated concerns behind a high-fanout boundary

- **Dimension**: maintainability, extensibility, crate structure
- **Scope**: `crates/shared/types/src/lib.rs`
- **Description**: Plugin types, discovery types, MQTT connection types, update-state enums,
  auth-adjacent values, SSRF helpers, and network utilities still live together in one crate
  that almost the whole workspace imports.
- **Why it matters**: a change needed by one subsystem triggers widespread rebuilds, broad
  review surfaces, and unclear ownership because the crate boundary is too coarse.
- **Failure scenario**: adding a new plugin-specific type requires touching a crate that 30+
  downstream crates depend on, triggering a full workspace rebuild.

### [LOW] `PluginType::From<PluginType> for String` reimplements the `as_str()` match table

- **Dimension**: idiomatic Rust, maintainability
- **Scope**: `crates/shared/types/src/plugin_types.rs:230-258`
- **Description**: The `From<PluginType> for String` match arm duplicates the string values
  already present in `as_str()` (lines 63-87), creating two sources of truth for the same
  mapping. A future rename of a plugin type string requires updating both locations.
- **Why it matters**: with 20+ variants, divergence between `as_str()` and
  `From<PluginType> for String` is easy to introduce and hard to detect without explicit tests
  for every variant.
- **Failure scenario**: a new variant is added to `as_str()` but the corresponding
  `From<PluginType> for String` arm is forgotten, causing DB writes to use the wrong string.

## Split/Merge Notes

- Best split candidate: move plugin/discovery-specific types closer to the plugin infrastructure
  crates.
- No merge is recommended; the problem is over-aggregation, not excessive fragmentation.
