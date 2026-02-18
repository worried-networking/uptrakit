# Code Review: `uptrakit-openapi-client`

**Date:** 2026-02-17
**Reviewer:** AI-assisted
**Scope:** Full crate review (`crates/shared/openapi-client/`)
**Overall quality:** HIGH

## Executive Summary

The crate provides a typed HTTP client around the Uptrakit web API with strong
error handling and solid type safety. No critical issues were found. The main
improvement area is resiliency (timeouts and optional retry strategy).

## Findings

### O-1 (Medium): Missing request/connect timeouts

**Location:** `crates/shared/openapi-client/src/lib.rs`

`reqwest::Client` is built without explicit connect/request timeouts. Hanging
network operations can block CLI and automation flows indefinitely.

**Suggested fix:** Configure `.connect_timeout(...)` and `.timeout(...)` on the
client builder (or make them configurable).

### O-2 (Medium): 429 handling lacks actionable retry metadata

**Location:** response handlers in `crates/shared/openapi-client/src/lib.rs`

`429 Too Many Requests` is mapped to `ClientError::RateLimited`, but retry
information (e.g. `Retry-After`) is not surfaced.

**Suggested fix:** Parse and expose `Retry-After` (or related headers) in the
error variant so callers can implement robust backoff.

### O-3 (Low): 401 is not represented as a dedicated error variant

**Location:** response handlers in `crates/shared/openapi-client/src/lib.rs`

`401 Unauthorized` is currently returned as a generic API error variant, which
makes auth-refresh flows less explicit for consumers.

**Suggested fix:** Add a dedicated unauthorized error variant and map 401
responses before generic API error mapping.

### O-4 (Low): One endpoint bypasses helper pattern

**Location:** `crates/shared/openapi-client/src/auth.rs`

`device_auth_poll` manually constructs and sends the request while most methods
use shared helper functions.

**Suggested fix:** Route through the same helper used by comparable unauth
POST endpoints for consistency.

### O-5 (Info): Raw response fallback is intentional and acceptable

**Location:** `crates/shared/openapi-client/src/lib.rs`

`raw_request` falls back from JSON parsing to a string JSON value for non-JSON
responses. This is a safe display fallback.

## Strengths

- Consistent typed API surface and endpoint coverage.
- Entity ID parameters use `&Uuid`.
- Proper typed errors with `thiserror` + `rootcause` context propagation.
- No `unsafe`, no `panic!`, and no production `unwrap()` usage.
- Clean module split and re-exports for downstream ergonomics.

## AGENTS.md Compliance Check

- No `unsafe`: pass
- No `#[allow(...)]`: pass
- Typed error boundary + context propagation: pass
- Entity IDs as `&Uuid`: pass
- API coverage parity with web-api (excluding documented exceptions): pass
