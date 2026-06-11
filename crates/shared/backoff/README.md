# uptrakit-backoff

Exponential backoff with jitter for reconnect loops. Four plain methods on `Backoff`: `new`, `reset`, `escalate`, `sample_base_jitter`. No guard
ceremony — verb choice is made at the call site with an inline comment as the audit log.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
uptrakit-backoff = "0.1"
```

## Examples

### Reset on success

A reconnect loop that resets on success and escalates on failure:

```rust
use uptrakit_backoff::Backoff;
use std::time::Duration;

async fn reconnect_loop() {
    let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));

    loop {
        match connect_to_service().await {
            Ok(conn) => {
                // reset chosen: connection succeeded.
                backoff.reset();
                handle(conn).await;
                break;
            }
            Err(e) => {
                // escalate chosen: pre-connection failure; no milestone reached.
                let delay = backoff.escalate();
                tracing::warn!(error = %e, ?delay, "connection failed; retrying");
                tokio::select! {
                    _ = shutdown_token.cancelled() => break,
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        }
    }
}
```

### Partial-progress (headline bug-fix shape)

Distinguish progress from no-progress failures. This is the shape that fixed the 60-second delay regression: a WebSocket close _after_ upgrade
(progress made) resets backoff to base; a TCP error _before_ upgrade (no progress) escalates it:

```rust
use uptrakit_backoff::Backoff;
use std::time::Duration;

async fn enrollment_loop(mut backoff: Backoff) {
    loop {
        match enroll().await {
            Ok(()) => break,
            Err(e) if e.is_receive_closed() => {
                // reset chosen: post-WS-upgrade server close — connection was healthy.
                backoff.reset();
                let delay = backoff.sample_base_jitter();
                tracing::info!(error = %e, ?delay, "post-upgrade close, reconnecting");
                tokio::time::sleep(delay).await;
            }
            Err(e) if e.is_transient_network() => {
                // escalate chosen: pre-upgrade TCP/DNS failure — no milestone reached.
                let delay = backoff.escalate();
                tracing::warn!(error = %e, ?delay, "transient error, reconnecting");
                tokio::time::sleep(delay).await;
            }
            Err(e) => return Err(e),
        }
    }
}
```

### `LoopOutcome::Disconnected`

When an upstream `Ok` arm already called `reset()`, use `sample_base_jitter()` to get the base-plus-jitter delay without further state mutation:

```rust
use uptrakit_backoff::Backoff;
use std::time::Duration;

async fn handle_outcome(outcome: LoopOutcome, backoff: &mut Backoff) {
    match outcome {
        LoopOutcome::Shutdown => return,
        LoopOutcome::Disconnected => {
            // reset() was called in the Ok(outcome) arm above;
            // current == base. Sample without advancing state.
            let delay = backoff.sample_base_jitter();
            tracing::warn!(?delay, "disconnected by controller, reconnecting");
            tokio::time::sleep(delay).await;
        }
    }
}
```

### Bounded retry

Retry a fixed number of times, escalating on each failure:

```rust
use uptrakit_backoff::Backoff;
use std::time::Duration;

async fn fetch_with_retry() -> Result<String, String> {
    let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(10));
    const MAX_ATTEMPTS: u32 = 5;

    for attempt in 1..=MAX_ATTEMPTS {
        match fetch_config().await {
            Ok(config) => return Ok(config),
            Err(e) => {
                // escalate chosen: each failure is a fresh unhealthy cycle.
                let delay = backoff.escalate();
                tracing::warn!(attempt, max_attempts = MAX_ATTEMPTS, delay_ms = delay.as_millis(), error = %e, "fetch failed; retrying");
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
    Err("all attempts exhausted".into())
}
```

## API Overview

- `Backoff::new(base, max)` — Create a backoff controller starting at `base`, doubling up to `max`.
- `Backoff::reset(&mut self)` — Healthy cycle: set `current` back to `base`. Call on `Ok` or after meaningful milestone.
- `Backoff::escalate(&mut self) -> Duration` — Unhealthy cycle: sample pre-escalation `current + jitter`, then double `current` (capped at `max`).
  Returns the sampled delay. `#[must_use]` — workspace `clippy::let_underscore_must_use = "deny"` closes the `let _ = backoff.escalate();` escape.
- `Backoff::sample_base_jitter(&self) -> Duration` — Sample `base + jitter` without state mutation. Used after `reset()` for the post-reset delay.

Standard parameters: **base 2 s, cap 60 s** with ~25 % jitter. For full API documentation, see
[docs.rs/uptrakit-backoff](https://docs.rs/uptrakit-backoff).

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
