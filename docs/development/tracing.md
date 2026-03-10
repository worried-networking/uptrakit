# Tracing and Distributed Tracing

This document covers span conventions, `#[instrument]` usage, request ID generation, and
the wire-protocol `TraceContext` for distributed tracing correlation. For log-level guidelines
and `RUST_LOG` usage, see [Logging](logging.md).

## Subscriber Architecture

All binaries use a **registry-based** tracing subscriber so that adding an OpenTelemetry
exporter layer later is a one-line change:

```rust
use tracing_subscriber::prelude::*;
tracing_subscriber::registry()
    .with(tracing_subscriber::fmt::layer().with_filter(filter))
    // Future: .with(tracing_opentelemetry::layer().with_tracer(tracer))
    .init();
```

Each binary owns its own `init_tracing()` function in `src/main.rs`. The controller has
its own setup (with an optional journald layer). The service-sdk does not provide tracing
initialization — libraries must not configure the global dispatcher.

## Span Naming Conventions

Span names use **dot-separated** `module.operation` format:

| Span name | Location |
| --- | --- |
| `http.request` | Request-ID middleware |
| `ws.deserialize` | WebSocket protocol |
| `service.event_loop` | Service-SDK event loop |
| `scheduler.poll_cycle` | Scheduler poll loop |

For `#[instrument]` annotations, the span name defaults to the function name.
Use `name = "module.operation"` only when the function name is unclear.

## `#[instrument]` Conventions

Every `#[instrument]` annotation follows this pattern:

```rust
#[tracing::instrument(skip_all, fields(key = %value, ...))]
async fn my_function(...) { ... }
```

### Rules

1. **Always `skip_all`** — never auto-capture function arguments (they may contain
   secrets, large payloads, or non-Display types).
2. **Explicitly list fields** — only include identifiers relevant for correlation.
3. **Use `%` for Display types**, `?` for Debug types.
4. **Place below `#[utoipa::path]`** — for route handlers, the instrument annotation
   goes between the OpenAPI attribute and `pub async fn`.

### Fields to include

| Context | Fields |
| --- | --- |
| HTTP handlers | (inherit from `http.request` span: `request_id`, `method`, `path`) |
| WebSocket handlers | `service_id`, `msg_type` |
| Update flows | `software_item`, `update_history_id`, `plugin_type` |
| Scheduler tasks | `task` (task name), `controller_id` |
| MQTT handlers | `service_id`, `mqtt_client_id` |
| Discovery | `host_id`, `plugin_count` |
| Version checks | `assignment_count` |

### Fields to never capture

- Request/response payloads
- Secrets, tokens, passwords, private keys
- Database connections or connection pools
- Executors or runtime handles
- Large collections (log the `.len()` instead)

## Request ID Middleware

The `request_id` middleware in `web-api` ensures every HTTP request has a unique identifier:

1. Reads `x-request-id` header from the incoming request.
2. If absent or empty, generates a UUID v7.
3. Stores `RequestId` in request extensions (available to handlers).
4. Creates an `info_span!("http.request", request_id, method, path)`.
5. Sets `x-request-id` on the response header.
6. Propagates `RequestId` to response extensions for `request_log` to include.

Clients can supply their own request ID for end-to-end correlation by setting the
`x-request-id` header on the request.

## Wire Protocol TraceContext

Every WebSocket and NATS envelope carries a `trace_context` object for distributed
tracing correlation:

```json
{
  "protocol_version": 1,
  "seq": 1,
  "trace_context": {
    "trace_id": "0123456789abcdef0123456789abcdef",
    "span_id": "fedcba9876543210"
  },
  "type": "ping",
  "service_ts": 1706400000000
}
```

### TraceContext Fields

| Field | Type | Description |
| --- | --- | --- |
| `trace_id` | string | 32 lowercase hex chars (128-bit W3C trace ID) |
| `span_id` | string (optional) | 16 lowercase hex chars (64-bit W3C span ID) |

### Propagation

- **Outbound messages**: `OutgoingSeq::wrap_service()` and `wrap_controller()` accept
  a `TraceContext` parameter. Call sites pass `current_trace_context()`.
- **NATS events**: `NatsEventEnvelope` includes a `trace_context` field.
- **Inbound tolerance**: The `trace_context` field uses `#[serde(default)]`, so older
  peers that don't send it won't break deserialization.

### `current_trace_context()`

Currently generates a random trace ID (UUID v4 without hyphens). When
`tracing-opentelemetry` is integrated, this function will extract the real trace/span
ID from the current span context — making all propagation plumbing light up automatically.

## Future: OpenTelemetry Integration

Adding OpenTelemetry requires:

1. Add `tracing-opentelemetry` and an exporter (e.g., `opentelemetry-otlp`) to
   workspace dependencies.
2. In each binary's subscriber setup, add the OTel layer:

   ```rust
   .with(tracing_opentelemetry::layer().with_tracer(tracer))
   ```

3. Update `current_trace_context()` to extract from the current span:

   ```rust
   pub fn current_trace_context() -> TraceContext {
       use tracing_opentelemetry::OpenTelemetrySpanExt;
       let ctx = tracing::Span::current().context();
       // Extract trace_id and span_id from ctx
   }
   ```

No other code changes are needed — all spans, request IDs, and trace context propagation
are already in place.

## Cross-References

- [Logging](logging.md) — log levels, verbosity flags, `RUST_LOG`
- [Coding Standards](coding-standards.md) — error handling, security audit logging
- [Wire Protocol](../api/wire-protocol.md) — envelope format, `TraceContext` on the wire
- [Security — Secrets](../security/secrets-and-encryption.md) — never log secrets
