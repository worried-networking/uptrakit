//! `uptrakit-controller-core` — controller core state types.
//!
//! **Invariant**: no direct dependency on `uptrakit-web-api` (HTTP routing/Axum handlers)
//! or `uptrakit-mcp`. Shared library crates (`uptrakit-web-api-auth`, `uptrakit-web-api-queries`)
//! are permitted — they provide auth and query primitives without importing the routing layer.
//! Enforced by the absence of `uptrakit-web-api` and `uptrakit-mcp` in `Cargo.toml`.

pub mod audit;
pub mod auth;
pub mod connections;
pub mod db;
pub mod notification;
pub mod settings;
pub mod update;
pub mod workload_claims;
