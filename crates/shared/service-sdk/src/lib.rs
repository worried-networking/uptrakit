#[macro_use]
mod macros;

pub mod backoff;
pub mod build_info;
pub mod ca;
pub mod cert_handler;
pub mod cli;
pub mod config_proxy;
pub mod connection;
pub mod dirs;
#[cfg(feature = "zeroconf")]
pub mod discovery;
pub mod error;
pub mod event_loop;
pub mod generated;
pub mod identity;
pub mod lifecycle;
pub mod main_helper;
#[cfg(feature = "sensitive-params")]
pub mod sensitive_params;
pub mod shared_types;
pub mod shutdown;
pub mod signal;
pub mod surface_proxy;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod tls;
pub mod tracing_init;
pub(crate) mod ws;

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
pub use error::{
    CaError, EnrollmentError, IdentityError, ProtocolError, Result, TlsError,
    is_rustls_cert_expired,
};
pub use identity::ServiceIdentityState;
pub use lifecycle::{default_resolve_shutdown, run_service_lifecycle};
pub use main_helper::{init_crypto, print_build_info, run_lifecycle_and_handle_errors};
#[cfg(feature = "sensitive-params")]
pub use sensitive_params::decrypt_sensitive_params;
pub use shared_types::{
    EventLoopContext, LoopError, LoopOutcome, LoopResult, ServiceHandler, ShutdownCause,
};
pub use shutdown::{ShutdownSignal, SignalShutdown, TokenShutdown};
pub use signal::{Signal, SignalWatcher};
pub use surface_proxy::{PendingSurfaceRequest, ServiceSurfaceProxy, ServiceSurfaceProxyError};
