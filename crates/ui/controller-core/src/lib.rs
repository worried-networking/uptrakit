//! `uptrakit-controller-core` — pure business-logic state types.
//!
//! **Invariant**: this crate must never import `uptrakit-web-api`, `uptrakit-mcp`,
//! or any crate that depends on them (axum, utoipa, etc.). Enforced by the
//! absence of those path deps in `Cargo.toml`. Any contributor adding a dep
//! that pulls in axum must stop and reconsider the design.

pub mod audit;
pub mod auth;
pub mod connections;
pub mod db;
pub mod notification;
pub mod settings;
pub mod update;
pub mod workload_claims;
