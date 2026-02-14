# Code Review: uptrakit-provider-registry

## Summary

Provider dispatch and validation crate (~350 lines across 3 source files) acting as the factory layer between `ProviderType` and concrete provider implementations. Provides `ProviderRegistry` with static methods for provider creation, config validation, and secret masking/restoration.

## Architecture

- **Module structure**: `lib.rs` re-exports from `registry.rs` and `error.rs`.
- **Public API surface**: `ProviderRegistry` struct with `create_provider()`, `validate_config()`, `validate_config_str()`, `mask_config_secrets()`, `mask_config_secrets_str()`, `restore_config_secrets()`, `restore_config_secrets_str()`, `parse_provider_type()`.
- **Dependency choices**: Direct dependencies on all four provider crates (`github`, `docker-registry`, `proxmox-helper-scripts`, `homebrew`), plus `uptrakit-provider-core` and `uptrakit-shared-types`.
- **Layering**: Consumed by `web-api` and `agent`. Acts as the single entry point for all provider operations.

## Security and Safety

- **Secret masking**: `mask_config_secrets()` correctly replaces sensitive fields with `"***MASKED***"` sentinel for API responses. `restore_config_secrets()` restores original values when the sentinel is detected during updates.
- No `unsafe` code.
- No `unwrap`/`panic` in non-test code.

## Code Quality

- **Error handling**: `RegistryError` with 3 variants (`UnknownProvider`, `Validation`, `Config`). Uses `rootcause::Report` wrapper.
- **Test coverage**: Tests cover creation, validation, secret masking, and secret restoration for all four provider types.
- **Documentation**: Module-level doc comment with usage example.

## Coding Standards Compliance

- Typed error enum with `thiserror` + `rootcause::Report` -- compliant.
- `Result<T>` type alias defined.
- `impl_report_conversion!` used for cross-boundary errors.
- No `#[allow()]` directives.

## Extensibility Assessment

**The registry is the primary extensibility bottleneck in the entire provider system.**

## Findings

| ID | Severity | Category | Description | File:Line |
| --- | --- | --- | --- | --- |
| PREG-01 | Critical | Extensibility | The registry is completely closed to external providers. `create_provider()`, `validate_config()`, `mask_config_secrets()`, and `restore_config_secrets()` all use `match` on the closed `ProviderType` enum. An external developer cannot register a new provider without forking this crate and modifying `uptrakit-shared-types::ProviderType`. | `src/registry.rs` |
| PREG-02 | Major | Extensibility | Hard compile-time dependency on all provider crates. Every provider is unconditionally compiled into the registry. There are no Cargo feature gates to exclude unneeded providers, and no way to compile a minimal binary. Adding a provider with heavy dependencies (e.g., a cloud SDK) inflates all binaries. | `Cargo.toml:13-16` |
| PREG-03 | Major | Extensibility | `ProviderRegistry` is a unit struct with all static methods -- there is no instance-based registry. Providers cannot be registered at runtime. To support external providers, the registry would need a `HashMap<String, Box<dyn ProviderFactory>>` and a `register()` method. | `src/registry.rs:18` |
| PREG-04 | Minor | Code Quality | `parse_provider_type()` uses `serde_json::from_value(serde_json::Value::String(...))` as a parser. A `FromStr` implementation on `ProviderType` would be more idiomatic and avoid the allocation. | `src/registry.rs:230` |
| PREG-05 | Minor | Code Quality | Secret masking/restoration logic is duplicated across the `match` arms rather than being delegated to a trait method on provider configs. A `ProviderConfig` trait with `fn mask_secrets(&mut self)` and `fn restore_secrets(&mut self, existing: &Self)` would allow polymorphic dispatch. | `src/registry.rs:95-180` |

## Recommendations

1. **Immediate**: Add Cargo feature gates for each provider (`features = ["github", "docker", "homebrew", "proxmox"]`, default = all). This enables minimal builds.

2. **Medium-term**: Introduce a `ProviderFactory` trait for runtime registration:

   ```rust
   pub trait ProviderFactory: Send + Sync {
       fn provider_type_name(&self) -> &str;
       fn create(&self, config: &serde_json::Value) -> Result<Box<dyn Provider>>;
       fn validate_config(&self, config: &serde_json::Value) -> Result<()>;
       fn mask_secrets(&self, config: &mut serde_json::Value);
       fn restore_secrets(&self, incoming: &mut serde_json::Value, existing: &serde_json::Value);
   }
   ```

3. **Long-term**: Change `ProviderType` from a closed enum to a string-based newtype (or add `#[non_exhaustive]` + `Other(String)`) so external providers can use arbitrary type names.

## Verdict

**Conditional pass.** The registry works correctly for built-in providers but is the primary blocker for external extensibility (PREG-01). The closed enum + static match pattern means adding any provider requires modifying two workspace crates. Feature gates (PREG-02) and a factory trait (PREG-03) are the recommended path forward.
