use crate::Result;
use crate::UptrakitClient;

impl UptrakitClient {
    /// Download the CA certificate in PEM format.
    ///
    /// This endpoint does not require authentication.
    pub async fn ca_cert(&self) -> Result<String> {
        self.get_text_unauth(crate::paths::pki::CA_CERT).await
    }

    /// Download the CA certificate revocation list in PEM format.
    ///
    /// This endpoint does not require authentication.
    pub async fn ca_crl(&self) -> Result<String> {
        self.get_text_unauth(crate::paths::pki::CA_CRL).await
    }
}
