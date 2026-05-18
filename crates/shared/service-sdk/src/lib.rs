//! Shared SDK for uptrakit Services.
//!
//! Provides the lifecycle, transport, identity, and shutdown plumbing that
//! every Service relies on. Designed to be published independently of the
//! workspace: no internal-crate dependencies; all required wire types are
//! generated locally via xtask.
//!
//! ## Run modes
//!
//! A Service can run in one of two modes; both go through the same
//! [`ServiceHandler`] trait.
//!
//! - **Standalone (binary)** — the Service connects to the controller over
//!   WebSocket, does enrollment, manages its own certificate lifecycle, and
//!   handles OS signals. Drive it with [`run_lifecycle_and_handle_errors`]
//!   (the typical `main()` entry point) or [`run_service_lifecycle`] for
//!   finer control.
//! - **Embedded (in-process)** — the controller constructs the handler
//!   itself, hands it controller-side dependencies (DB, state dir, identity
//!   keypair), and runs it on an in-process transport. Drive it with
//!   [`run_embedded_service`]. Embedded services skip enrollment, certificate
//!   management, and signal handling; shutdown is driven by two
//!   `CancellationToken`s.
//!
//! See ADR-0004 (`docs/adr/0004-service-handler-transport-abstraction.md`)
//! for the transport abstraction rationale and ADR-0005
//! (`docs/adr/0005-service-binary-runtime-boundary.md`) for the
//! binary-vs-runtime crate split.
//!
//! ## Service-owned migrations
//!
//! Services that own a local DB override
//! [`ServiceHandler::service_migrations`] (gated by the `service-migrations`
//! feature). The embedding controller calls this static method at startup and
//! merges the migrations with its own. See
//! `docs/development/database-migrations.md` for the implementation guide.
//!
//! ## Most-used re-exports
//!
//! - Entry points: [`run_lifecycle_and_handle_errors`],
//!   [`run_service_lifecycle`], [`run_embedded_service`].
//! - Trait: [`ServiceHandler`] (impl this on your Service).
//! - Identity helpers: [`ServiceIdentityState`].
//! - Shutdown: [`ShutdownCause`], [`default_resolve_shutdown`].
//! - Errors: [`LoopError`], [`LoopOutcome`], [`LoopResult`].

#[macro_use]
mod macros;
mod embedded;

pub mod backoff;
pub mod build_info;
pub mod ca;
pub mod cert_handler;
pub mod cert_resolver;
pub mod cli;
pub mod config_proxy;
pub mod connection;
pub mod dirs;
#[cfg(feature = "zeroconf")]
pub mod discovery;
pub mod error;
pub mod event_loop;
pub mod identity;
pub mod lifecycle;
pub mod main_helper;
#[cfg(feature = "sensitive-params")]
pub mod sensitive_params;
pub mod session_store;
pub mod shared_types;
pub mod shutdown;
pub mod signal;
pub mod surface_proxy;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod tls;
pub mod tofu;
pub mod tracing_init;
pub(crate) mod ws;

pub(crate) mod wire_api {
    pub(crate) use uptrakit_wire::*;
}

pub(crate) mod shared_types_api {
    pub(crate) use uptrakit_shared_types::*;
}

#[cfg(feature = "cli")]
pub use tracing_init::init_cli_tracing;
#[cfg(feature = "test-support")]
pub use tracing_init::init_test_tracing;
pub use tracing_init::{BoxedLayer, TracingBuilder};

pub use backoff::Backoff;
pub use cert_handler::{
    CertificateRenewalHandler, FAR_FUTURE, compute_renewal_delay, create_renewal_sleep,
    update_renewal_schedule,
};
pub use config_proxy::{PendingServiceConfigRequest, ServiceConfigProxy, ServiceConfigProxyError};
pub use connection::ControllerConnection;
pub use embedded::run_embedded_service;
pub use error::{
    CaError, EnrollmentError, IdentityError, ProtocolError, Result, TlsError,
    is_rustls_cert_expired,
};
pub use identity::ServiceIdentityState;
pub use lifecycle::{default_resolve_shutdown, run_service_lifecycle};
pub use main_helper::{init_crypto, print_build_info, run_lifecycle_and_handle_errors};
#[cfg(feature = "sensitive-params")]
pub use sensitive_params::decrypt_sensitive_params;
pub use session_store::CertScopedClientSessionStore;
pub use shared_types::{
    EventLoopContext, LoopError, LoopOutcome, LoopResult, ServiceHandler, ShutdownCause,
};
pub use shutdown::{ShutdownSignal, SignalShutdown, TokenShutdown};
pub use signal::{Signal, SignalWatcher};
pub use surface_proxy::{PendingSurfaceRequest, ServiceSurfaceProxy, ServiceSurfaceProxyError};
