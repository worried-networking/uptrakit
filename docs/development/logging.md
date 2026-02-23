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
| `debug` | Per-step instrumentation useful for diagnosing failures during development or on-site debugging: detected version, sending message, executing provider, spawning command. |
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

The baseline level is `info` for the service's own crate; all other crates are silent unless `RUST_LOG` enables them.

| Flags | Effective level |
| --- | --- |
| (none) | `info` for own crate; silence elsewhere |
| `-v` | `debug`, all crates |
| `-vv` | `trace`, all crates |
| `-vvv`+ | Same as `-vv`; a warning is printed |

### CLI (`uptrakit`)

The CLI produces no log output by default (tracing is not initialised at all). When `-v` is given, output goes to
**stderr** so that command output on stdout is not contaminated.

| Flags | Effective level |
| --- | --- |
| (none) | No log output |
| `-v` | `warn` |
| `-vv` | `info` |
| `-vvv` | `debug` |
| `-vvvv` | `trace` |
| `-vvvvv`+ | Same as `-vvvv`; a warning is printed |

### Excessive-verbosity warning

When more `-v` flags are passed than necessary to reach `trace`, a message is printed to stderr **before** the
subscriber is initialised (so it always reaches the user regardless of log level):

- Services: `warning: -vvv or more has no additional effect; maximum verbosity is -vv (trace)`
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

The verbosity flag adds a global fallback directive (e.g. `debug`). `RUST_LOG` entries, being more specific, always
override it. This means:

- Use `-v` to get debug output everywhere.
- Use `RUST_LOG=tokio=warn,h2=warn` to silence noisy crates while keeping the rest at debug.
- Use `RUST_LOG=uptrakit_agent=trace` to get trace output from one crate without enabling it elsewhere, without any `-v`
  flag.

### Common examples

```bash
# Debug everywhere except tokio and h2
RUST_LOG=tokio=warn,h2=warn uptrakit-agent -v

# Trace only the agent crate (no flag needed)
RUST_LOG=uptrakit_agent=trace uptrakit-agent

# Trace command crate, debug everywhere else
RUST_LOG=uptrakit_command=trace uptrakit-agent -v

# Info for the full CLI run (warn + info visible)
uptrakit -vv hosts list

# Debug the controller without touching RUST_LOG
uptrakit-controller -v
```

## Security Rule

**Never log secrets, tokens, passwords, or private key material.**

All secret fields in HTTP API types must use `SecretString` instead of `String`. The `tracing` macros must never be
called with values that expose sensitive data, even at `trace` level. The secret must be masked before any logging
call.

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

### Cross-references

- [Security — Secrets Handling and Encryption](../security/secrets-and-encryption.md)
- [Security — Secure Development](../security/secure-development.md)
- [Coding Standards](coding-standards.md)
