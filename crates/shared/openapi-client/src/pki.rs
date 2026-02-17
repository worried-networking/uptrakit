use crate::Result;
use crate::UptrakitClient;

impl UptrakitClient {
    /// Download the CA certificate in PEM format.
    ///
    /// This endpoint does not require authentication.
    pub async fn ca_cert(&self) -> Result<String> {
        self.get_text_unauth("/api/v1/pki/ca.crt").await
    }

    /// Download the CA certificate revocation list in PEM format.
    ///
    /// This endpoint does not require authentication.
    pub async fn ca_crl(&self) -> Result<String> {
        self.get_text_unauth("/api/v1/pki/ca.crl").await
    }
}
