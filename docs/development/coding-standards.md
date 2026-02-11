# Coding Standards

## Error Handling

- Wrap errors in `rootcause::Report` and define a `Result<T>` alias per boundary (e.g. `pub type Result<T> = Report<MyError>`).
- Use `thiserror::Error` with `#[derive(Debug, Error)]` to describe failures.
- Implement `ReportConversion` (via `impl_report_conversion!`) for all downstream errors and prefer `.context_to()?` to preserve the chain.
- Use `report!()` for creating new reports and `bail!()` for early returns.
- Avoid `Result<T, String>`; prefer typed enums.
- Logging should never expose secrets (tokens, passwords, keys).

## Panic Policy

- `unwrap()`, `expect()`, and `panic!()` are forbidden in production code (tests are the only exception).
- Locking primitives (`Mutex::lock()`, `RwLock::read()`, `RwLock::write()`) may use `.unwrap()` because the release build aborts on panic, rendering
  poisoning impossible.
- When serialization/parsing can fail, use `match` to handle errors gracefully or propagate them with context.

## Design Principles

- Keep every boundary clear: the controller orchestrates scheduling, upstream checks, API/UI; the MQTT service handles MQTT/Home Assistant
  integration; agents manage installed versions and update execution; providers focus on version detection/updating logic.
- Treat custom scripts and command execution paths as handling untrusted input; validate before execution to avoid shell injection.
- Agent connections are outbound-only, unprivileged, and rely on explicit sudo allowlists for privileged update commands.
- Logs should contain operational summaries only; do not store full command output internally.
- New protected routes must check the typed `Permission` enum rather than comparing raw role names.
- Document every behavioral change either in code or via an external docs page (e.g., update `docs/api` or `docs/development`).

Refer to [docs/security/secure-development.md](../security/secure-development.md) when the change touches PKI, secrets, reverse proxies, or filesystem
security.
