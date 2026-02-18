# Code Review: uptrakit-agent

Extensibility-focused review of the agent crate.

## Dependency Analysis

| Dependency | Purpose | Concern |
| --- | --- | --- |
| `uptrakit-provider-registry` | Instantiate providers from JSON config | Pulls in **all** provider implementation crates transitively |
| `uptrakit-service-sdk` | Service lifecycle, enrollment, TLS | Clean; appropriate for an agent binary |
| `uptrakit-internal-wire` | Wire protocol messages | Clean; required for controller communication |
| `uptrakit-command` | Local command execution | Clean; minimal dependency |
| `uptrakit-shared-types` | Shared enums and value types | Clean; leaf crate |

## Findings

### Critical: agent depends on provider-registry

**Location:** `Cargo.toml:28`

The agent binary depends on `uptrakit-provider-registry`, which transitively pulls in every
concrete provider crate (GitHub, Docker Registry, Proxmox Helper Scripts, Homebrew). The agent
needs the registry to instantiate providers from JSON config for local version detection and
update execution. This is **architecturally justified** -- the agent must be able to run any
provider locally -- but it creates a large transitive dependency footprint that couples the agent
binary to all provider implementations at compile time.

**Impact:** Adding a new provider crate increases the agent's compile time and binary size, even
if a given agent deployment never uses that provider.

### Recommendation: trait-based ProviderFactory

Introduce a `ProviderFactory` trait (in `provider-core` or a new `provider-factory` crate) that
the registry implements:

```rust
pub trait ProviderFactory: Send + Sync {
    fn create_provider(
        &self,
        provider_type: ProviderType,
        config: &serde_json::Value,
        executor: Arc<dyn CommandExecutor>,
    ) -> Result<Box<dyn Provider>>;
}
```

The agent would depend on the factory trait rather than the concrete registry. Alternatively,
compile provider selection behind **feature flags** so deployments can opt into only the
providers they need.

## Positive Observations

- Uses `service-sdk` for lifecycle management, enrollment, and TLS -- clean separation.
- Command execution is abstracted via `CommandExecutor` trait injection.
- No direct database dependency; the agent is stateless beyond its service identity.
