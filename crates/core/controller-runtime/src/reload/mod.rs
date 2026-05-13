//! Reloadable subsystems for the controller runtime.
//!
//! Each submodule implements [`uptrakit_config_reload::reloadable::Reloadable`]
//! for one long-lived subsystem.  The macro
//! [`uptrakit_config_reload::reloadable_erased_impl!`] generates the
//! object-safe [`uptrakit_config_reload::reloadable::ReloadableErased`] wrapper
//! that the coordinator stores in its heterogeneous registry.

pub(crate) mod db_pool;
pub(crate) mod https_listener;
#[cfg(feature = "nats")]
#[expect(
    dead_code,
    reason = "NatsReloadable is wired into the coordinator in Task 14; struct and constructor are unused until then"
)]
pub(crate) mod nats;
pub(crate) mod pki_listener;
pub(crate) mod probe;
pub(crate) mod tls_snapshot;
