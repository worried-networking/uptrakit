//! Reloadable subsystems for the controller runtime.
//!
//! Each submodule implements [`uptrakit_config_reload::reloadable::Reloadable`]
//! for one long-lived subsystem.  The macro
//! [`uptrakit_config_reload::reloadable_erased_impl!`] generates the
//! object-safe [`uptrakit_config_reload::reloadable::ReloadableErased`] wrapper
//! that the coordinator stores in its heterogeneous registry.

// Subsystem types are wired into the coordinator in Task 14; suppress
// dead_code lint until that wiring is in place.
#[expect(
    dead_code,
    reason = "DbConnHandle and DbPoolReloadable are wired into the coordinator in a follow-up task"
)]
pub(crate) mod db_pool;
pub(crate) mod embedded;
pub(crate) mod zeroconf;
