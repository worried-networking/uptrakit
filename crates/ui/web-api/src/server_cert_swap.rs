use std::sync::Arc;

use rustls::sign::CertifiedKey;

/// Trait for hot-swapping the server TLS certificate without rebuilding `ServerConfig`.
///
/// Implemented by [`ControllerServerCertResolver`] in `controller-runtime`.
/// Stored in [`ServerState`] as `Option<Arc<dyn ServerCertSwap>>` so that
/// `web-api` (which does not depend on `controller-runtime`) can call into it
/// without a direct type dependency.
pub trait ServerCertSwap: Send + Sync + std::fmt::Debug {
    /// Replace the current [`CertifiedKey`] with `cert`.
    ///
    /// Subsequent TLS handshakes will use the new key; in-progress handshakes
    /// are not affected.
    fn swap_cert(&self, cert: Arc<CertifiedKey>);
}
