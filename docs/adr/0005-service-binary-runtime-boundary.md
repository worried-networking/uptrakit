# 0005 — Service Binary/Runtime Boundary

**Date:** 2026-05-07
**Status:** Accepted

## Context

Service binary crates (`agent-ssh`, `mqtt`, `scheduler`) had poorly defined boundaries with
their `-runtime` counterparts. The root symptom: `agent-ssh` owned its DB entities and 12
migrations, so when `agent-ssh` ran embedded inside the Controller the same tables had to be
created a second way — `shared-db` carried `m20260331_000001_ssh_agent_tables`, a compressed
one-shot migration that duplicated the standalone schema. Any SSH agent schema change had to be
applied in two places or drift silently.

Additionally, `controller-runtime` depended on `uptrakit-agent-ssh` (a binary crate) to access
the embedded handler, violating the principle that the controller only depends on library crates.

The prerequisite for this ADR — `run_embedded_service`, `ShutdownCause::EmbeddedDrain`,
`dyn ServiceTransport` handler signatures, and controller-side `ServiceSettings` injection — was
shipped as ADR-0004.

## Decision

**Binary crates are thin launch shells; runtime crates own all business logic, DB schema, and
shared operations.**

The invariant for every Service binary crate:

| Allowed              | Examples                                                                                 |
| -------------------- | ---------------------------------------------------------------------------------------- |
| Entry point          | `main.rs` — process init, construct handler, call `run_lifecycle_and_handle_errors`      |
| CLI argument structs | `cli.rs` — clap derives only                                                             |
| Subcommand dispatch  | `host_cli.rs`, `commands/` (agent-ssh only) — argument parsers + thin calls into runtime |

Not allowed in binary crates: DB entities, migrations, business logic, protocol implementation,
transport logic, surface handlers, crypto helpers.

Each `-runtime` crate exports exactly one `ServiceHandler` implementation. The controller and
the standalone binary both construct that handler with different dependencies but the same type.
The controller never depends on a binary crate.

### service_migrations()

`ServiceHandler` gains a static method `service_migrations()` (behind the `service-migrations`
feature gate in `uptrakit-service-sdk`) returning the migrations this service owns. The default
returns `vec![]`. Runtime crates with a local DB override it.

The controller calls `AgentSshHandler::service_migrations()` at startup and passes the result
to `run_migrations_with_plugins`.

### Migration Strategy for Existing Controller Deployments (B+B1 over Tombstone)

Replacing `shared-db`'s monolithic `m20260331_000001_ssh_agent_tables` with a repair migration
(`m20260331_000002_agent_ssh_migration_history_repair`) uses a **B+B1 strategy** rather than
a tombstone:

- **B (repair migration):** Checks whether the old one-shot row is present in
  `seaql_migrations`. If so, inserts the 13 individual agent-ssh migration rows with
  `ON CONFLICT DO NOTHING`, then deletes the old row.
- **B1 (service migrations):** The 13 migrations from `agent-ssh-runtime` are contributed at
  startup. On existing deployments they are already recorded (B ran first); on fresh installs
  they run normally.

**Frozen-list constraint:** The 13 migration names in the repair migration's INSERT list are
frozen at the time the repair is written. No new `agent-ssh` migrations may be added between
writing the repair and shipping it to production. If a new agent-ssh migration lands in the
same release, it must be added to the INSERT list before the release cuts.

### Crate structure after refactor

| Before                                               | After                                                       |
| ---------------------------------------------------- | ----------------------------------------------------------- |
| `controller-runtime` dep on `uptrakit-agent-ssh`     | `controller-runtime` dep on `uptrakit-agent-ssh-runtime`    |
| `agent-ssh` owned 13 migrations + all business logic | `agent-ssh-runtime` owns all; `agent-ssh` is a thin shell   |
| `shared-db` carried duplicated SSH schema migration  | `shared-db` carries only repair migration; no SSH schema    |
| Hand-rolled embedded loops in `controller-runtime`   | Unified `run_embedded_service<H>` for all embedded services |

## Consequences

**Positive:**

- SSH schema changes happen in one place (agent-ssh-runtime migrations only).
- Controller never depends on binary crates.
- Adding a new embedded service requires only constructing its `ServiceHandler` and calling
  `run_embedded_service` — no hand-written event loops.
- `service_migrations()` provides a compile-time, type-safe mechanism for services to
  contribute schema changes.

**Negative:**

- Each embedded service's `ServiceHandler` must be constructible with controller-provided deps
  (DB connection, state dir, ECIES keypair). Handler constructors must not hardcode internal
  paths or open their own DB connections. This invariant extends to all tasks spawned by the
  handler at runtime — background tasks and surface-action handlers must thread the injected
  connection rather than calling `init_db` or equivalent.
- The frozen-list constraint on the repair migration requires coordination when new agent-ssh
  migrations land during the same release as the refactor.
