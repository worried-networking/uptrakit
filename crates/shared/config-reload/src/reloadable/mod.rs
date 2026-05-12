//! Typed and erased reload contracts for long-lived subsystems.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rootcause::Report;

use crate::delta::RuntimeConfigDelta;

/// Typed reload contract for a long-lived subsystem.
///
/// This trait is **not** object-safe (it has an associated type and uses
/// `impl Future` return types). Use [`ReloadableErased`] for heterogeneous
/// collections inside the coordinator.
pub trait Reloadable: Send + Sync {
    /// The config section type this subsystem reacts to.
    type Config: Send + Sync + 'static;

    /// Stable identifier used in audit events and log messages.
    fn name(&self) -> &'static str;

    /// Validate the incoming config *before* any state mutation.
    ///
    /// # Errors
    ///
    /// Returns an error if the new config is logically invalid for this
    /// subsystem (e.g. URL format, port range, conflicting options).
    fn validate(&self, new: &Self::Config) -> Result<(), Report>;

    /// Apply the new config. Called only after [`validate`](Self::validate)
    /// succeeds for *all* registered subsystems.
    ///
    /// # Errors
    ///
    /// Returns an error if the subsystem could not switch to the new config.
    fn apply(&self, new: Arc<Self::Config>)
    -> impl Future<Output = Result<(), Report>> + Send + '_;

    /// Revert to the previous config. Called when the watchdog detects
    /// a failure after `apply`.
    ///
    /// # Errors
    ///
    /// Returns an error if the revert itself failed, which will put the
    /// coordinator into [`CoordinatorState::Degraded`](crate::coordinator::CoordinatorState::Degraded).
    fn revert(&self) -> impl Future<Output = Result<(), Report>> + Send + '_;

    /// Confirm the subsystem is healthy after `apply`.
    ///
    /// The coordinator runs health checks concurrently within
    /// [`rollback_window`](Self::rollback_window). A timeout or `Err` triggers
    /// [`revert`](Self::revert) for all applied subsystems.
    ///
    /// # Errors
    ///
    /// Returns an error if the subsystem is not healthy.
    fn health_check(&self) -> impl Future<Output = Result<(), Report>> + Send + '_;

    /// Maximum time allowed for [`health_check`](Self::health_check) to
    /// succeed before the coordinator treats the subsystem as failed.
    fn rollback_window(&self) -> Duration;
}

/// Object-safe wrapper used by the coordinator's heterogeneous registry.
///
/// Implementors receive a [`RuntimeConfigDelta`] and are responsible for
/// filtering to their own variant. Use [`reloadable_erased_impl!`] to generate
/// the boilerplate from a [`Reloadable`] implementation.
#[async_trait]
pub trait ReloadableErased: Send + Sync {
    /// Stable identifier used in audit events and log messages.
    fn name(&self) -> &'static str;

    /// Validate the incoming delta. Should return `Ok(())` for unrecognised
    /// variants.
    ///
    /// # Errors
    ///
    /// Returns an error if the relevant config section is logically invalid.
    fn validate(&self, delta: &RuntimeConfigDelta) -> Result<(), Report>;

    /// Apply the relevant section of `delta`.
    ///
    /// # Errors
    ///
    /// Returns an error if the subsystem could not switch to the new config.
    async fn apply(&self, delta: &RuntimeConfigDelta) -> Result<(), Report>;

    /// Revert to the previous config.
    ///
    /// # Errors
    ///
    /// Returns an error if the revert itself failed.
    async fn revert(&self) -> Result<(), Report>;

    /// Confirm the subsystem is healthy after `apply`.
    ///
    /// # Errors
    ///
    /// Returns an error if the subsystem is not healthy.
    async fn health_check(&self) -> Result<(), Report>;

    /// Maximum time allowed for [`health_check`](Self::health_check) before
    /// the coordinator treats the subsystem as failed.
    fn rollback_window(&self) -> Duration;
}

/// Generate the `#[async_trait] impl ReloadableErased for $struct` body from
/// an existing [`Reloadable`] implementation.
///
/// # Usage
///
/// ```rust,ignore
/// reloadable_erased_impl!(MyDbSubsystem, RuntimeConfigDelta::Db);
/// ```
///
/// This expands to a full `impl ReloadableErased for MyDbSubsystem` that
/// delegates each method to the `Reloadable` impl, matching only the given
/// `RuntimeConfigDelta` variant and returning `Ok(())` for all others.
#[macro_export]
macro_rules! reloadable_erased_impl {
    ($struct:ty, $variant:path) => {
        #[::async_trait::async_trait]
        impl $crate::reloadable::ReloadableErased for $struct {
            fn name(&self) -> &'static str {
                <Self as $crate::reloadable::Reloadable>::name(self)
            }

            fn validate(
                &self,
                delta: &$crate::delta::RuntimeConfigDelta,
            ) -> ::std::result::Result<(), ::rootcause::Report> {
                if let $variant(cfg) = delta {
                    <Self as $crate::reloadable::Reloadable>::validate(self, cfg)
                } else {
                    Ok(())
                }
            }

            async fn apply(
                &self,
                delta: &$crate::delta::RuntimeConfigDelta,
            ) -> ::std::result::Result<(), ::rootcause::Report> {
                if let $variant(cfg) = delta {
                    <Self as $crate::reloadable::Reloadable>::apply(self, cfg.clone()).await
                } else {
                    Ok(())
                }
            }

            async fn revert(&self) -> ::std::result::Result<(), ::rootcause::Report> {
                <Self as $crate::reloadable::Reloadable>::revert(self).await
            }

            async fn health_check(&self) -> ::std::result::Result<(), ::rootcause::Report> {
                <Self as $crate::reloadable::Reloadable>::health_check(self).await
            }

            fn rollback_window(&self) -> ::std::time::Duration {
                <Self as $crate::reloadable::Reloadable>::rollback_window(self)
            }
        }
    };
}
