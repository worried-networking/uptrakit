# Code Review — `crates/shared/openapi-client`

> Review date: 2026-02-28 | Reviewer: AI multi-agent review (6 specialist dimensions)
> Dimensions covered: Architecture · Security & Safety · Code Quality ·
> High Availability · Coding Standards · Extensibility

## Summary

`uptrakit-openapi-client` (~4,186 LOC) is the typed HTTP client library generated from the
OpenAPI spec, consumed by `uptrakit-cli` and integration tests. It provides a well-structured
retry/backoff mechanism with `Retry-After` header support, correct exponential backoff with
overflow guards, and a `mock` feature for hermetic testing. The primary concerns are that the
retry-backoff tests do not verify delay durations (meaning timing regressions would go
undetected) and that `Retry-After` parsing handles only numeric seconds (not HTTP-date format).

---

## What's Well-Implemented

- **[High Availability]** `RetryConfig` with exponential backoff is correctly bounded: the
  backoff factor uses `checked_shl` with `u32::MAX` fallback and `saturating_mul`, then
  `.min(max_delay)`. Delay is guaranteed to stay within `max_delay` regardless of retry count.

- **[High Availability]** `Retry-After` header is respected for 429 responses: the numeric
  value is parsed and clamped to `max_delay`, preventing a server from forcing an arbitrarily
  long pause.

- **[Code Quality]** `fetch_all_pages` is a clean generic implementation handling the
  zero-pages edge case without panicking, and overrides only the `page` and `per_page` keys
  on each iteration.

- **[Code Quality]** `RetryConfig` documentation is detailed: which status codes trigger
  retry, how the backoff is computed, and which codes are never retried. This is uncommon and
  valuable for a retry implementation.

- **[Extensibility]** The `mock` feature is correctly test-only: `httpmock` is declared as
  `optional = true` in `[dependencies]` and in `[dev-dependencies]`. Production builds never
  compile the mock module. Downstream crates activate the feature only in
  `[dev-dependencies]`.

- **[Coding Standards]** Re-exports `uptrakit-web-api-types` as `types`, `reqwest::StatusCode`,
  `reqwest::Error`, and `uuid::Uuid` under a single facade so the CLI does not accumulate a
  web of transitive explicit dependencies.

---

## What Requires Attention

### Major

- **[Code Quality]** `src/lib.rs:687–885` — Retry-backoff tests run with `start_paused =
  false` and assert only that operations eventually succeed or fail the expected number of
  times. The actual delay values — the functional correctness of the backoff algorithm — are
  never asserted. A regression that changed `Duration::from_secs` to `Duration::from_millis`,
  inverted the `min(max_delay)` cap, or broke `Retry-After` parsing would pass all existing
  tests. Convert to `#[tokio::test(start_paused = true)]` and measure elapsed virtual time
  between mock invocations to validate exponential growth, jitter range, and cap behaviour.

### Minor

- **[High Availability]** `src/lib.rs` — `Retry-After` parsing handles only numeric seconds.
  RFC 9110 §10.2.4 permits an HTTP-date string (e.g., `Wed, 21 Oct 2025 07:28:00 GMT`). A
  server responding with the HTTP-date format causes `parse_retry_after` to return `None`,
  silently falling back to `initial_delay`. Add a doc comment at the call site noting this
  limitation, or implement HTTP-date parsing.

- **[Code Quality]** `src/mock.rs:232,259` — `unwrap()` on `serde_json::to_string` in mock
  helper methods. Serialisation of in-memory `serde_json::Value` is effectively infallible,
  but the `unwrap()` should be `expect("BUG: serde_json::Value serialization is infallible")`
  to self-document the infallibility assumption.

### Observations

- **[Coding Standards]** `src/mock.rs:221,276` — `StatusCode::NO_CONTENT.as_u16()` and
  `StatusCode::TOO_MANY_REQUESTS.as_u16()` are called to satisfy `httpmock`'s `u16` API.
  Per the coding standards review, `as_u16()` usage at non-serialisation sites should carry
  an inline comment: `// httpmock requires u16 — StatusCode::as_u16() approved here`.

- **[High Availability]** The `openapi-client` `Retry-After` parsing tests (three tests in
  the retry suite) assert call counts but not delay durations, leaving the timing behaviour
  fully unverified. This is the most consequential gap in the test suite for a retry client
  used in production CLI operations.

- **[Extensibility]** The client is generated from the OpenAPI spec but there is no automated
  check that the generated Rust types match the live spec in CI. A spec drift (e.g., a new
  required field added to a response type) would compile successfully but fail at runtime.
  Consider adding a spec-conformance test analogous to the `asyncapi.yaml` tests in the
  `wire` crate.
