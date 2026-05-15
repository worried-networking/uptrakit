//! Cross-crate hook for triggering a process reexec when irreversibly-bound
//! config keys change. Defined here so `uptrakit-config-reload` stays ignorant
//! of `controller-runtime`'s reexec internals.

use rootcause::Report;

use crate::config::RuntimeConfig;

/// Result of a reexec eligibility check.
///
/// `exec()` on success diverges and never returns, so this type is only
/// ever constructed on the two non-diverging paths.
///
/// Not `#[non_exhaustive]` — this is a closed two-variant result-like enum
/// whose only match site is `ControllerReexecHook::check_and_trigger`. Adding
/// `#[non_exhaustive]` to an enum in a shared crate forces every external
/// consumer to add a wildcard arm, which would make the exhaustive match in
/// `controller-runtime` fail to compile.
#[must_use]
pub enum ReexecOutcome {
    /// Reexec was attempted but `exec()` returned an error. The process is
    /// still running; the coordinator treats this as a reload failure.
    ExecFailed(Report),
    /// No irreversibly-bound key changed; proceed with in-process reload.
    NotNeeded,
}

/// Hook called by the coordinator before applying file-sourced deltas.
///
/// The implementation lives in `controller-runtime` and is registered at
/// startup via [`crate::coordinator::ReloadCoordinator::set_reexec_hook`].
/// This keeps the shared `uptrakit-config-reload` crate ignorant of
/// `triage::decide` and `perform_reexec`.
pub trait ReexecHook: Send + Sync {
    /// Inspect `prior` vs `new`; decide and perform reexec if needed.
    ///
    /// On a successful `exec()` the function diverges and never returns.
    /// Returns `ReexecOutcome::ExecFailed(err)` when `exec()` fails.
    /// Returns `ReexecOutcome::NotNeeded` when no irreversibly-bound key
    /// changed.
    ///
    /// **Pre-exec requirement**: flush any async log writers synchronously
    /// before calling `perform_reexec`, because the Tokio runtime is
    /// killed when the process image is replaced. Prefer a synchronous
    /// tracing writer for the controller binary.
    ///
    /// # Errors (via `ReexecOutcome::ExecFailed`)
    ///
    /// Wraps the OS error from `exec()` when the binary path is inaccessible
    /// or cleared `FD_CLOEXEC` failed.
    fn check_and_trigger(&self, prior: &RuntimeConfig, new: &RuntimeConfig) -> ReexecOutcome;
}
