//! `uptrakit-controller-core` — controller core state types.
//!
//! **Invariant**: no direct dependency on `uptrakit-web-api` (HTTP routing/Axum handlers)
//! or `uptrakit-mcp`. Shared library crates (`uptrakit-web-api-auth`, `uptrakit-web-api-queries`)
//! are permitted — they provide auth and query primitives without importing the routing layer.
//! Enforced by the absence of `uptrakit-web-api` and `uptrakit-mcp` in `Cargo.toml`.

pub mod access;
pub mod audit;
pub mod auth;
pub mod connections;
pub mod db;
pub mod notification;
pub mod settings;
pub mod update;
pub mod workload_claims;

/// Test-only re-exports for in-tree functional tests.
///
/// Items exposed here are gated on `feature = "testing"` and exist solely so
/// the in-tree `uptrakit-functional-tests` crate can drive controller
/// orchestration end-to-end. They are **not** part of the stable public API:
/// signatures, naming, and contract may change without semver impact, and
/// out-of-tree callers are unsupported.
#[cfg(feature = "testing")]
pub mod testing {
    pub use crate::update::controller::run_protection_and_dispatch;
}
