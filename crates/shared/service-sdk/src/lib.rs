pub mod ca;
pub mod cert_handler;
pub mod cli;
pub mod config_proxy;
pub mod connection;
#[cfg(feature = "zeroconf")]
pub mod discovery;
pub mod error;
pub mod event_loop;
pub mod extension_proxy;
pub mod identity;
pub mod lifecycle;
pub mod main_helper;
pub mod sensitive_params;
pub mod shared_types;
pub mod signal;
pub mod tls;
pub(crate) mod ws;

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
pub use extension_proxy::{
    PendingExtensionRequest, ServiceExtensionProxy, ServiceExtensionProxyError,
};
pub use identity::ServiceIdentityState;
pub use lifecycle::{default_resolve_shutdown, run_service_lifecycle};
pub use main_helper::{init_crypto, print_build_info, run_lifecycle_and_handle_errors};
pub use sensitive_params::decrypt_sensitive_params;
pub use shared_types::{
    EventLoopContext, LoopError, LoopOutcome, LoopResult, ServiceHandler, ShutdownCause,
};
pub use signal::{Signal, SignalWatcher};
pub use uptrakit_backoff::Backoff;
