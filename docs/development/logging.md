# Logging

This document covers the logging infrastructure, verbosity flags, `RUST_LOG` interaction, and best practices for all Uptrakit binaries and library crates.

## Infrastructure

All crates use the [`tracing`](https://docs.rs/tracing) crate for instrumentation. Log output is produced by
[`tracing-subscriber`](https://docs.rs/tracing-subscriber) with the `env-filter` feature.

There is no use of `log` or `env_logger` anywhere in this codebase. Macros to use are:

```rust
tracing::error!("...");
tracing::warn!("...");
tracing::info!("...");
tracing::debug!("...");
tracing::trace!("...");
```

The `tracing` ecosystem attaches structured fields directly to events, which enables log aggregation tooling (Loki,
structured journald, etc.) to filter and search by field values.

### Tracing initialisation

The canonical implementation lives in `crates/shared/tracing-init/src/lib.rs`
(`uptrakit-tracing-init`). `uptrakit-service-sdk` re-exports everything from it, so most callers
use the `uptrakit_service_sdk::` path. The controller depends on `uptrakit-tracing-init` directly
to avoid pulling in the full service SDK.

| Context | Call |
| --- | --- |
| Service daemons | `uptrakit_service_sdk::TracingBuilder::new().verbosity(…).init()` |
| Controller | `uptrakit_tracing_init::TracingBuilder::new().verbosity(…).init()` |
| CLI | `uptrakit_service_sdk::init_cli_tracing(cli.verbose)` |
| Tests | `uptrakit_service_sdk::init_test_tracing()` (feature `test-support`) |

Do not add per-binary `init_tracing()` helpers or call `tracing_subscriber` directly in binaries.

## Log Level Guidelines

| Level | Usage | Example events |
| --- | --- | --- |
| `error` | Unrecoverable failures that terminate an operation or require operator attention. Always capture the error value as `error = %e`. | TLS setup failure, DEK unwrap failure, database connection lost |
| `warn` | Unexpected conditions that are handled but may indicate misconfiguration or degraded operation. | TOFU without a pinned fingerprint, rate limits encountered, update frozen |
| `info` | High-level lifecycle events: service started, enrollment completed, connection established, CA fetched, update completed. One or a few lines per major operation. | `"enrollment approved"`, `"WebSocket connected"`, `"update completed"` |
| `debug` | Per-step instrumentation useful for diagnosing failures during development or on-site debugging. | Detected version, sending message, executing plugin, spawning command |
| `trace` | Highly verbose, per-message or per-item events. Never enable in production unless targeting a specific subsystem. | Individual output lines from commands, WebSocket frames, token cache hits |

### Rules

- `error!` **must always** capture the error as a structured field: `error!(error = %e, "operation failed")`.
  Never embed the error in the message string: `error!("operation failed: {e}")` is wrong.
- `info!` must not be used with the `"security_audit"` target. Security audit events use `warn!` at that target.

## `security_audit` Target

Operations that modify security-sensitive state emit events at `warn` level with
`target: "security_audit"`. This allows log aggregation tools and alerting systems to filter and
alert on audit events without text parsing.

```rust
// ✓ Correct — tracing target is first, structured fields follow
tracing::warn!(
    target: "security_audit",
    user_id = %user.user_id,
    tenant_id = %tenant_db.tenant_id,
    plugin_config_id = %config_id,
    "plugin config deleted"
);

// ✗ Wrong — message prefix is not machine-readable
tracing::warn!("security_audit: plugin config deleted");
```

### Filtering by audit target

```bash
# Show only security audit events
RUST_LOG=security_audit=warn uptrakit-controller

# Suppress audit noise while debugging another subsystem
RUST_LOG=uptrakit=debug,security_audit=off uptrakit-controller
```

### Operations that require the `security_audit` target

- Create, update, or delete plugin configs with command-bearing fields
- Toggle update freeze on a service or host
- Reject updates due to rate limiting or freeze state
- Upsert or delete plugin type settings
- Machine ID mismatch detection on incoming controller messages

### Relation to `uptrakit-audit-log`

The `security_audit` tracing target is separate from the `uptrakit-audit-log` subsystem, which
records structured audit records to the database. The tracing events are advisory/operational
(operator-visible in logs); the database records are the authoritative audit trail for compliance.
See [Security — Audit Logging](../security/audit-logging.md) for the full audit architecture.

## Verbosity Flags

All binaries accept a `-v` / `--verbose` flag that can be repeated. Each additional `-v` steps up one log level.

### Service daemons (`uptrakit-agent`, `uptrakit-agent-ssh`, `uptrakit-mqtt`, `uptrakit-scheduler`)

The baseline level is `info` for all uptrakit crates; third-party dependencies like
`tokio`, `h2`, `rustls`, and `reqwest` are silent unless `RUST_LOG` enables them explicitly.

| Flags | Directives | Effect |
| --- | --- | --- |
| (none) | `uptrakit=info` | All uptrakit crates at info |
| `-v` | `uptrakit=debug` | All uptrakit crates at debug |
| `-vv` | `uptrakit=trace` | All uptrakit crates at trace |
| `-vvv`+ | Same as `-vv`; a warning is printed | |

### Controller (`uptrakit-controller`)

The controller uses a finer-grained scheme at lower verbosity levels to reduce noise from
web-API request logs at the default level.

| Flags | Directives | Effect |
| --- | --- | --- |
| (none) | `uptrakit_controller=info,uptrakit_web_api=info` | Controller and API at info |
| `-v` | `uptrakit_controller=debug,uptrakit_web_api=debug` | Controller and API at debug |
| `-vv` | `uptrakit=debug` | All uptrakit crates at debug |
| `-vvv` | `uptrakit=trace` | All uptrakit crates at trace |
| `-vvvv`+ | Same as `-vvv`; a warning is printed | |

### CLI (`uptrakit`)

The CLI produces no log output by default (tracing is not initialised at all). When `-v` is given, output goes to
**stderr** so that command output on stdout is not contaminated.

| Flags | Directive | Effect |
| --- | --- | --- |
| (none) | *(no subscriber)* | No log output |
| `-v` | `uptrakit_cli=warn` | CLI crate warnings only |
| `-vv` | `uptrakit_cli=debug` | CLI crate at debug |
| `-vvv` | `uptrakit=debug` | All uptrakit crates at debug |
| `-vvvv` | `uptrakit=trace` | All uptrakit crates at trace |
| `-vvvvv`+ | Same as `-vvvv`; a warning is printed | |

### Excessive-verbosity warning

When more `-v` flags are passed than necessary to reach `trace`, a message is printed to stderr **before** the
subscriber is initialised (so it always reaches the user regardless of log level):

- Services: `warning: -vvv or more has no additional effect; maximum verbosity is -vv (trace)`
- Controller: `warning: -vvvv or more has no additional effect; maximum verbosity is -vvv (trace)`
- CLI: `warning: -vvvvv or more has no additional effect; maximum verbosity is -vvvv (trace)`

## `RUST_LOG` Reference

All binaries respect the standard `RUST_LOG` environment variable. Since the migration to
`TracingBuilder`, **`RUST_LOG` directives take precedence over programmatic (verbosity-derived)
directives for the same target**. Programmatic directives are added first; `RUST_LOG` entries are
appended after. Because `EnvFilter` resolves same-target conflicts in favour of the later directive,
`RUST_LOG` always wins.

> **Precedence reversal (post-migration):** Before the `TracingBuilder` migration, programmatic
> directives were added *after* `RUST_LOG`, meaning `-v` flags could silently override `RUST_LOG`
> settings for the same target. The new behaviour (RUST_LOG wins) is more predictable and matches
> operator expectations.

More-specific directives (target-qualified) always beat less-specific ones (bare level) regardless
of order.

### Syntax

```text
RUST_LOG=crate=level
RUST_LOG=crate::module=level
RUST_LOG=level                  # global fallback
RUST_LOG=crate1=level1,crate2=level2,global_level
```

### How `-v` and `RUST_LOG` interact

`-v` adds a scoped directive (`uptrakit=debug` etc.) to the filter. `RUST_LOG` directives are
always added after, so:

- `RUST_LOG=uptrakit_agent=error uptrakit-agent -vv` → agent crate is `error` (RUST_LOG wins),
  rest of uptrakit is `trace` (from `-vv`).
- `RUST_LOG=tokio=info uptrakit-agent -v` → uptrakit at `debug`, tokio at `info`.
- `RUST_LOG=uptrakit=info uptrakit-agent -vv` → all uptrakit at `info` (RUST_LOG=info overrides -vv=trace).

Use `-v` to get debug output across all uptrakit crates without touching third-party noise.
Use `RUST_LOG` for surgical per-crate overrides.

### Common examples

```bash
# Own crate at debug only
uptrakit-agent -v

# All uptrakit crates at debug
uptrakit-agent -vv

# All uptrakit crates at debug, plus tokio at info
RUST_LOG=tokio=info uptrakit-agent -vv

# Trace only the agent crate (no flag needed)
RUST_LOG=uptrakit_agent=trace uptrakit-agent

# All uptrakit crates at trace
uptrakit-agent -vvv

# Debug the controller without touching RUST_LOG
uptrakit-controller -vv

# CLI warnings only
uptrakit -v hosts list

# All uptrakit crates at debug via CLI
uptrakit -vvv hosts list

# Security audit events only
RUST_LOG=security_audit=warn uptrakit-controller
```

## Security Rule

**Never log secrets, tokens, passwords, or private key material.**

All secret fields in HTTP API types must use `SecretString` instead of `String`. The `tracing` macros must never be
called with values that expose sensitive data, even at `trace` level. The secret must be masked before any logging
call. See also the `sensitive_params` module in `uptrakit-service-sdk` for helpers.

### Startup-time credentials: use `eprintln!`, not `tracing`

One-time registration tokens and similarly short-lived credentials that must be shown to the operator at startup must
be written to **stderr via `eprintln!()`**, never emitted through the `tracing` pipeline.

```rust
// ✓ Correct — goes to stderr only; never captured by log aggregators
if let Some(token) = reg_token {
    eprintln!("==========================================================");
    eprintln!("  No users found. Use this one-time registration token:");
    eprintln!("  {token}");
    eprintln!("==========================================================");
}

// ✗ Wrong — any tracing subscriber (Loki, journald, structured JSON)
// will capture and persist this credential
tracing::info!("Registration token: {}", token);
```

**Why:** Structured tracing subscribers (Loki, Datadog, structured journald) persist all
`tracing::info!` output indefinitely. A one-time token in a log aggregator becomes a persistent
credential that attackers can extract. Stderr is the conventional channel for startup-time
operator messages and is not forwarded by log aggregators.

When a `SecretString` wrapper holds a credential that must be displayed, call `.expose_secret()`
explicitly at the `eprintln!` call site to make the exposure visible in code review:

```rust
eprintln!("  Token: {}", secret.expose_secret());
```

See [Secrets Handling and Encryption](../security/secrets-and-encryption.md) for the full secrets policy, including
`SecretString` usage, master key handling, and at-rest encryption.

## Best Practices

### Use structured fields

Always attach relevant context as structured fields rather than embedding values in the message string:

```rust
// Correct
tracing::debug!(version = %detected, package = %id, "version detected");

// Avoid
tracing::debug!("version {} detected for package {}", detected, id);
```

Structured fields let downstream tooling (Loki, structured journald) filter and aggregate by specific values without
text parsing.

### Always use `error = %e` for error fields

```rust
// Correct — error is a structured field
tracing::error!(error = %e, "operation failed");

// Avoid — error embedded in message string is not filterable
tracing::error!("operation failed: {e}");
```

Use `%e` (Display) for most errors. Use `?e` (Debug) only when the Display representation is insufficient.

### Avoid `format!()` in log calls

`format!()` allocates even when the log level is not enabled. Use fields directly:

```rust
// Correct — allocation skipped when debug is disabled
tracing::debug!(count = items.len(), "items processed");

// Avoid — always allocates
tracing::debug!("processed {} items", items.len());
```

### Avoid logging in hot paths

Only instrument hot paths at `trace` level (individual output lines, tight loops). `debug` and above should be for
per-operation or per-request events, not per-iteration.

### Follow the level table

When adding instrumentation, consult the level guidelines above. Lifecycle events belong at `info`, per-step
diagnostics at `debug`, and per-message events at `trace`. Avoid demoting `info` events to `debug` or promoting
`debug` events to `info` without a clear reason.

### Security audit events

Operations that modify security-sensitive state must emit a `warn!` event with `target: "security_audit"`.
See the [Security audit events](#security_audit-target) section above and
[Coding Standards — Security Audit Logging](coding-standards.md#security-audit-logging) for the
full convention, required fields, and examples.

## Cross-references

- [Tracing Conventions](tracing.md) — spans, `#[instrument]`, request IDs, distributed tracing
- [Security — Secrets Handling and Encryption](../security/secrets-and-encryption.md)
- [Security — Secure Development](../security/secure-development.md)
- [Coding Standards](coding-standards.md)
