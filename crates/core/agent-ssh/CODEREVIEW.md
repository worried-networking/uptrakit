# Code Review: uptrakit-agent-ssh

Extensibility-focused review of the SSH agent crate.

## Dependency Analysis

| Dependency | Purpose | Concern |
| --- | --- | --- |
| `uptrakit-shared-db` | Local SQLite for encrypted SSH credentials | Pulls in **all 34 entity definitions** |
| `uptrakit-service-sdk` | Service lifecycle, enrollment, TLS | Clean |
| `uptrakit-internal-wire` | Wire protocol messages | Clean |
| `uptrakit-command` | Command execution abstraction | Clean |
| `sea-orm` + `sea-orm-migration` | SQLite ORM and migrations | Appropriate for local DB |

## Findings

### Significant: agent-ssh depends on uptrakit-shared-db

**Location:** `Cargo.toml:32`

The SSH agent depends on `uptrakit-shared-db`, which contains all 34 entity definitions for the
entire system (controller entities, OIDC entities, MQTT entities, etc.). The SSH agent only uses
its own local SQLite tables for encrypted SSH credential storage. It does not need controller-side
entities like `oidc_provider`, `mqtt_lease`, `scheduled_task`, or `api_rate_limit`.

**Impact:** The SSH agent compiles and links all 34 entity models plus the `crypto` module even
though it only uses a small subset. This increases compile time and conceptually couples the agent
to the controller's schema.

### Recommendation: scope entity visibility

Two approaches to reduce this coupling:

1. **Feature-gate entity groups** in `shared-db` -- e.g., `controller-entities`, `agent-entities`,
   `crypto`. The SSH agent would enable only `crypto` and `agent-entities`.
2. **Extract agent-ssh's local DB schema** into its own minimal crate (e.g.,
   `uptrakit-agent-ssh-db`) that depends only on `sea-orm` and the `crypto` module from
   `shared-db`.

## Positive Observations

- Uses `EncryptedString` from `shared-db::crypto` for transparent at-rest encryption of SSH
  credentials -- well-designed security pattern.
- Clean use of `service-sdk` for lifecycle management.
- SSH credential management is self-contained; no leakage into other crates.
- Demonstrates `SshCommandExecutor` as a custom `CommandExecutor` implementation, validating the
  trait's extensibility.
