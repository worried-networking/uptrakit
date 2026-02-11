# Testing Expectations

Changes should be covered by tests, especially if they touch behavior, parsing, or provider logic. If an integration test is infeasible (e.g., OS
integration) include at least one of:

- Unit tests around decision logic
- Contract tests for serialization/parsing
- Integration tests backed by fixtures or mocks

## Run Tests Locally

```bash
cargo test --all-features
```

If you prefer `nextest`:

```bash
cargo nextest run --all-features
```

### What We Test

- Pure logic (unit tests)
- Provider behavior (parsing, version comparison, metadata mapping)
- API boundaries (request/response types, compatibility)
- Error paths with clear messaging
- Reverse proxy integration tests (Docker-based, ignored by default):

  ```bash
  cargo test -p uptrakit-controller reverse_proxy -- --ignored
  ```

  Requires Docker and covers L4/L7 TLS modes, CRL/OCSP revocation, and proxy-specific flows.
