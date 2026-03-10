//! Per-transport extension manifests and action definitions.
//!
//! Each sub-module exposes `manifest()` and `actions()` for a single
//! notification channel type. Only this crate knows about specific
//! transports — the web API treats all notification extensions generically.

#[cfg(feature = "email")]
pub mod email;
#[cfg(feature = "telegram")]
pub mod telegram;
#[cfg(feature = "webhook")]
pub mod webhook;
