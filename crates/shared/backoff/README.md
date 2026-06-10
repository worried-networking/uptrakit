# uptrakit-backoff

Exponential backoff with jitter for reconnect loops. The guard pattern (`Backoff::attempt() → AttemptGuard`) forces explicit `.reset()` /
`.escalate()` resolution, so the common reset-on-partial-success bug becomes a compile error via `#[must_use]`.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
uptrakit-backoff = "0.1"
```

## Examples

### Success-on-Ok pattern

Reconnect loop that resets on successful work and escalates on failure:

```rust
use uptrakit_backoff::Backoff;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));

    loop {
        let guard = backoff.attempt();
        let delay = guard.sample_delay();

        match connect_to_service().await {
            Ok(_) => {
                guard.reset(); // cycle was healthy
                break;
            }
            Err(_) => {
                guard.escalate(); // cycle was unhealthy
                tokio::time::sleep(delay).await;
            }
        }
    }
}

async fn connect_to_service() -> Result<(), String> {
    // application-specific connection logic
    Ok(())
}
```

### Partial-progress pattern

Distinguish between progress and no-progress failures. This is the headline bug fix: a WebSocket upgrade that fails post-connection (progress made)
resets backoff, while a pre-upgrade TCP error (no progress) escalates:

```rust
use uptrakit_backoff::Backoff;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));

    loop {
        let guard = backoff.attempt();
        let delay = guard.sample_delay();

        match enroll_with_service().await {
            Ok(_) => {
                guard.reset(); // success
                break;
            }
            Err(e) if e.is_receive_closed_report() => {
                guard.reset(); // WebSocket closed post-upgrade: we made progress
                tokio::time::sleep(delay).await;
            }
            Err(e) if e.is_transient_network_error() => {
                guard.escalate(); // TCP error before upgrade: no progress made
                tokio::time::sleep(delay).await;
            }
            Err(e) => {
                return Err(format!("fatal enrollment error: {e}"));
            }
        }
    }
    Ok(())
}

struct EnrollError;

impl EnrollError {
    fn is_receive_closed_report(&self) -> bool {
        todo!()
    }
    fn is_transient_network_error(&self) -> bool {
        todo!()
    }
}

async fn enroll_with_service() -> Result<(), EnrollError> {
    todo!()
}
```

### Bounded retry pattern

Retry a fixed number of times within a `for` loop. Each attempt gets its own guard, resolved in both success and failure arms:

```rust
use uptrakit_backoff::Backoff;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(10));
    const MAX_ATTEMPTS: usize = 5;

    for attempt in 1..=MAX_ATTEMPTS {
        let guard = backoff.attempt();

        match fetch_config().await {
            Ok(config) => {
                guard.reset(); // success
                println!("Config loaded: {config:?}");
                break;
            }
            Err(e) => {
                let delay = guard.sample_delay();
                guard.escalate(); // failed attempt
                tracing::warn!(
                    attempt,
                    max_attempts = MAX_ATTEMPTS,
                    delay_ms = delay.as_millis(),
                    error = %e,
                    "fetch attempt failed; retrying"
                );
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
}

async fn fetch_config() -> Result<String, String> {
    todo!()
}
```

## API Overview

- `Backoff::new(base: Duration, max: Duration)` — Create a new backoff controller with initial delay equal to `base`.
- `Backoff::attempt(&mut self) -> AttemptGuard` — Begin an attempt. Returns a guard that must be resolved via `.reset()` or `.escalate()` before any
  `.await` or early return that would drop it.
- `AttemptGuard::reset(self)` — Resolve the guard: the cycle was healthy (work returned `Ok` or made meaningful progress before failing). Resets the
  backoff delay to `base`.
- `AttemptGuard::escalate(self)` — Resolve the guard: the cycle was unhealthy (fast-fail with no progress). Doubles the backoff delay, capped at
  `max`.
- `AttemptGuard::sample_delay(&self) -> Duration` — Read-only: sample the delay for the next sleep. Jitter is resampled per call; store the result
  once if you need a stable value across multiple sleep operations.
- `Backoff::sample_base_jitter(&self) -> Duration` — Read-only: sample `base + jitter` without advancing state. Used when the backoff controller is
  already reset and you just want the jittered base delay.

Forgetting to resolve a guard is caught by workspace lints (`unused_must_use`, `clippy::let_underscore_must_use = "deny"`) for the common cases.
Holding a guard across a `?` operator is forbidden by a workspace compile-time test; escape hatch:
`// uptrakit-backoff: allow ? in attempt scope — <reason>` comment.

For the full API documentation, see [docs.rs/uptrakit-backoff](https://docs.rs/uptrakit-backoff).

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
