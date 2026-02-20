# Test Coverage: uptrakit-crypto

> Generated: 2026-02-20 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 87.5% (300 / 343) |
| Function coverage | 78.4% (40 / 51) |
| Test count | 19 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| lib.rs | 87.5% | 300/343 | 78.4% | 40/51 |

## Uncovered Critical Paths

### Tier 1 — Security-Critical

- **`EncryptedString::new()` development mode branch** (line 237): When no master key is configured, the `tracing::warn!` path
  stores plaintext as the DB value without encryption. This fallback is intentional for local development but is
  security-critical: if it ever fires in production, secrets are persisted unencrypted. No test exercises this branch because the
  global `OnceLock` master key is set once per process and cannot be unset.

### Tier 2 — Business-Logic

- **`EncryptedString::from_db()` private constructor** (line 252): Constructs an `EncryptedString` from a decrypted DB value and
  its original ciphertext representation. Called by both `ValueType::try_from` and `TryGetable::try_get_by` on the read path.
  Existing tests exercise it indirectly through the SeaORM `ValueType` round-trip, but the function itself shows as partially
  uncovered because not all call-site combinations are hit.
- **`TryGetable::try_get_by` null column name fallback** (line 332): When the column index has no string name
  (`index.as_str()` returns `None`), the error path falls back to the string `"encrypted_string"`. This branch is never
  exercised because tests always use named columns.

### Tier 3 — Supporting

- **`ValueType::type_name()`** (line 297): Returns the static string `"EncryptedString"`. Trivial accessor, unlikely to be
  called directly by application code.
- **`ValueType::array_type()`** (line 301): Returns `ArrayType::String`. Required by the SeaORM trait but not exercised by any
  current query pattern.
- **`ValueType::column_type()`** (line 305): Returns `ColumnType::Text`. Required by SeaORM for DDL generation but not called in
  tests.

## Test Recommendations

1. **Development mode plaintext fallback** — Test `EncryptedString::new()` when no master key is available. Requires either
   running a separate test binary without calling `init_master_key`, or refactoring to accept a key provider trait so the
   no-key path can be exercised in isolation. Covers the Tier 1 `tracing::warn!` branch (line 237). High priority: confirms the
   fallback works correctly and can be detected by monitoring.
2. **`TryGetable` with numeric column index** — Test `try_get_by` using a numeric `ColIdx` (not a named column) to exercise the
   `None` branch of `index.as_str()` at line 332. Requires constructing a mock `QueryResult` with positional indexing. Covers
   the Tier 2 null column name fallback.
3. **`TryGetable` decryption failure mapping** — Test that `try_get_by` correctly maps a decryption error to
   `TryGetError::DbErr(DbErr::Type(...))` when the stored ciphertext is corrupted. Requires a mock `QueryResult` returning a
   tampered `ENC:v1:` string. Covers the `map_err` closure at line 340.
4. **SeaORM trait accessor coverage** — Call `type_name()`, `array_type()`, and `column_type()` directly and assert their return
   values. Simple assertions, low effort. Covers Tier 3 supporting functions.
