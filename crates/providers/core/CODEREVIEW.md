# Code Review: uptrakit-provider-core

Extensibility-focused review of the provider core trait crate.

## Role in the Architecture

This crate defines the `Provider` trait and supporting types that all provider implementations
depend on. It is the primary interface external developers interact with when creating new
providers.

## Findings

### Minor: re-exports tokio::sync::mpsc

**Location:** `src/lib.rs:30`

```rust
pub use tokio::sync::mpsc;
```

This is convenient for provider developers (they avoid a direct `tokio` dependency), but it
couples the provider interface to tokio's specific channel implementation. If the output channel
abstraction ever needs to change (e.g., to support a different async runtime), this becomes a
breaking change for all providers.

**Recommendation:** Consider abstracting the output channel behind a trait or type alias that
could be swapped without breaking the `Provider` trait signature. This is low priority given
tokio's dominance, but worth noting for long-term flexibility.

### Minor: ProviderCapability enum is centralized and closed

**Location:** `src/types.rs:9-17`

`ProviderCapability` has 2 variants (`DiscoverLocalSoftware`, `RefreshPackageIndex`) and is
`#[non_exhaustive]`. While `#[non_exhaustive]` allows adding variants without a semver break,
external providers cannot declare custom capabilities without modifying this crate.

**Recommendation:** Consider a trait-based capability system where providers declare capabilities
via strings or a marker trait, rather than a closed enum. For example:

```rust
pub trait ProviderCapabilities {
    fn supports(&self, capability: &str) -> bool;
    fn capabilities(&self) -> Vec<&'static str>;
}
```

### Minor: re-exports command types

**Location:** `src/lib.rs:18-24`

Re-exporting `CommandExecutor`, `CommandSpec`, `CommandMode`, `CommandOutput`,
`LocalCommandExecutor`, `HookShell`, `OutputStreamType`, and `UpdateOutputLine` is good for
provider developer convenience -- they only need to depend on `provider-core`.

## Positive Observations

- **`Provider` trait is well-designed** -- default implementations with `unimplemented` errors
  enable progressive disclosure. Providers override only the methods they support.
- **`SecretMasking` trait is clean and elegant** -- the default no-op implementation and the
  JSON round-trip pattern in the registry make it trivial for providers with no secrets.
- **`Version` type handles semver and non-semver gracefully** -- uses semver for comparison when
  both sides parse, falls back to string ordering otherwise.
- **`DiscoveredSoftware` with optional `extra` metadata** allows provider-specific data without
  changing the core type.
- **`UpstreamRelease` captures comprehensive release metadata** -- version, tag, prerelease flag,
  release notes, published date, and downloadable assets.
- **Error types are well-structured** with `Report<ProviderError>` for context chains.
- Comprehensive test coverage for all types.
