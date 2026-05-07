//! `uptrakit-controller-core` — pure business-logic state types.
//!
//! **Invariant**: this crate must never directly import `uptrakit-web-api` (the HTTP
//! routing layer), `uptrakit-mcp`, or any crate that depends on either of those two.
//! `uptrakit-web-api-auth` is an allowed dep (shared auth library, not the routing layer).
//! Enforced by the absence of `uptrakit-web-api` and `uptrakit-mcp` in `Cargo.toml`.

pub mod audit;
pub mod auth;
pub mod connections;
pub mod db;
pub mod notification;
pub mod settings;
pub mod update;
pub mod workload_claims;
