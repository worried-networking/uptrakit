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

## Log Level Guidelines

| Level | Usage |
| --- | --- |
| `error` | Unrecoverable failures that terminate an operation or require operator attention. |
| `warn` | Unexpected conditions that are handled but may indicate misconfiguration or degraded operation (e.g., TOFU without a pinned fingerprint, rate limits encountered). |
| `info` | High-level lifecycle events: service started, enrollment completed, connection established, CA fetched, update completed. One or a few lines per major operation. |
| `debug` | Per-step instrumentation useful for diagnosing failures during development or on-site debugging: detected version, sending message, executing plugin, spawning command. |
| `trace` | Highly verbose, per-message or per-item events: individual output lines from commands, WebSocket frames, token cache hits, slug validation skip. Never enable in production unless targeting a specific subsystem. |

**Examples from the codebase:**

- `error!` — unrecoverable update failures, TLS setup failures.
- `warn!` — TOFU CA acceptance without fingerprint, Docker registry rate limit, pre/post hook failures.
- `info!` — enrollment approved, WebSocket connected, service certificate saved, CA fetched.
- `debug!` — connecting to controller, detecting installed version, pulling Docker image, running brew upgrade.
- `trace!` — sending individual WebSocket messages, command stdout/stderr lines, PHS slug validation skip.

## Verbosity Flags

All binaries accept a `-v` / `--verbose` flag that can be repeated. Each additional `-v` steps up one log level.

### Service daemons (`uptrakit-controller`, `uptrakit-agent`, `uptrakit-agent-ssh`, `uptrakit-mqtt`)

The baseline level is `info` for all uptrakit crates; third-party dependencies like
`tokio`, `h2`, `rustls`, and `reqwest` are silent unless `RUST_LOG` enables them explicitly.

| Flags | Directives | Effect |
| --- | --- | --- |
| (none) | `uptrakit=info` | All uptrakit crates at info |
| `-v` | `uptrakit=debug` | All uptrakit crates at debug |
| `-vv` | `uptrakit=trace` | All uptrakit crates at trace |
| `-vvv`+ | Same as `-vv`; a warning is printed | |

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

- Services: `warning: -vvvv or more has no additional effect; maximum verbosity is -vvv (trace)`
- CLI: `warning: -vvvvv or more has no additional effect; maximum verbosity is -vvvv (trace)`

## `RUST_LOG` Reference

All binaries respect the standard `RUST_LOG` environment variable. Its directives take precedence over the
verbosity-derived level because [`tracing_subscriber::EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)
resolves by specificity: a more-specific directive always beats a less-specific one.

### Syntax

```text
RUST_LOG=crate=level
RUST_LOG=crate::module=level
RUST_LOG=level                  # global fallback
RUST_LOG=crate1=level1,crate2=level2,global_level
```

### How `-v` and `RUST_LOG` interact

`-v` adds a scoped directive (`{module}=debug` or `uptrakit=debug/trace`) to the filter. Third-party crates remain
silent unless `RUST_LOG` enables them explicitly. `RUST_LOG` entries are always additive and work alongside the
verbosity directive, because `EnvFilter` resolves by specificity: a more-specific directive always beats a
less-specific one.

This means:

- Use `-vv` to get debug output across all uptrakit crates without touching third-party noise.
- Use `RUST_LOG=tokio=info` to enable tokio logging independently of any `-v` flag.
- Use `RUST_LOG=uptrakit_agent=trace` to get trace output from one crate without enabling it elsewhere.

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
```

## Security Rule

**Never log secrets, tokens, passwords, or private key material.**

All secret fields in HTTP API types must use `SecretString` instead of `String`. The `tracing` macros must never be
called with values that expose sensitive data, even at `trace` level. The secret must be masked before any logging
call.

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

Operations that modify security-sensitive state use a `security_audit:` message
prefix at `warn` level. This prefix enables log aggregation filters and alerts
without text parsing. See
[Coding Standards — Security Audit Logging](coding-standards.md#security-audit-logging)
for the full convention, required fields, and examples.

### Cross-references

- [Tracing Conventions](tracing.md) — spans, `#[instrument]`, request IDs, distributed tracing
- [Security — Secrets Handling and Encryption](../security/secrets-and-encryption.md)
- [Security — Secure Development](../security/secure-development.md)
- [Coding Standards](coding-standards.md)
