//! Reloadable subsystems for the controller runtime.
//!
//! Each submodule implements [`uptrakit_config_reload::reloadable::Reloadable`]
//! for one long-lived subsystem.  The macro
//! [`uptrakit_config_reload::reloadable_erased_impl!`] generates the
//! object-safe [`uptrakit_config_reload::reloadable::ReloadableErased`] wrapper
//! that the coordinator stores in its heterogeneous registry.

pub(crate) mod db_pool;
pub(crate) mod https_listener;
pub(crate) mod probe;
