use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

use crate::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct SystemAlert {
    pub id: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SystemAlertsResponse {
    pub alerts: Vec<SystemAlert>,
}

/// Get system alerts for the admin dashboard.
#[utoipa::path(
    get,
    path = "/api/v1/system/alerts",
    tag = "System",
    responses(
        (status = 200, description = "System alerts", body = SystemAlertsResponse)
    ),
    security(("bearer_token" = []))
)]
pub async fn get_system_alerts(
    State(state): State<Arc<AppState>>,
) -> Json<SystemAlertsResponse> {
    let snapshot = state.ca_snapshot.borrow().clone();
    let mut alerts = Vec::new();

    // Check if managed CA is approaching expiration
    if snapshot.managed {
        let now = time::OffsetDateTime::now_utc();
        let days_until_expiry = (snapshot.active_not_after - now).whole_days();

        if days_until_expiry <= 183 {
            alerts.push(SystemAlert {
                id: "ca_expiring".to_string(),
                severity: "warning".to_string(),
                title: "CA Certificate Expiring".to_string(),
                message: format!(
                    "The internal CA certificate expires in {} days. \
                     Automatic rotation will occur during the next check cycle.",
                    days_until_expiry
                ),
                action: None,
            });
        }
    }

    // Check if server cert is signed by a previous CA (needs re-issue)
    if snapshot.previous_fingerprint.is_some() {
        let server_cert_path = state.pki_path.join("server.crt");
        if let Ok(server_cert_pem) = std::fs::read_to_string(&server_cert_path) {
            // Check if server cert's issuer matches the active CA
            if let (Ok(server_not_after), Ok(active_fp)) = (
                cert_not_after_from_pem(&server_cert_pem),
                cert_issuer_check(&server_cert_pem, &snapshot.active_cert_pem),
            ) {
                if !active_fp {
                    alerts.push(SystemAlert {
                        id: "server_cert_old_ca".to_string(),
                        severity: "info".to_string(),
                        title: "Server Certificate Under Previous CA".to_string(),
                        message: "The HTTPS server certificate was signed by the previous CA. \
                                  Consider renewing it to use the current active CA."
                            .to_string(),
                        action: Some("renew_server_certificate".to_string()),
                    });
                }

                // Check if server cert is nearing expiry
                let now = time::OffsetDateTime::now_utc();
                let days_until_expiry = (server_not_after - now).whole_days();
                if days_until_expiry <= 30 {
                    alerts.push(SystemAlert {
                        id: "server_cert_expiring".to_string(),
                        severity: "warning".to_string(),
                        title: "Server Certificate Expiring".to_string(),
                        message: format!(
                            "The HTTPS server certificate expires in {} days. \
                             Automatic renewal will occur during the next check cycle.",
                            days_until_expiry
                        ),
                        action: Some("renew_server_certificate".to_string()),
                    });
                }
            }
        }
    }

    Json(SystemAlertsResponse { alerts })
}

/// Extract not_after timestamp from a PEM cert.
fn cert_not_after_from_pem(pem: &str) -> Result<time::OffsetDateTime, ()> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(pem.as_bytes()).map_err(|_| ())?;
    let cert = pem_block.parse_x509().map_err(|_| ())?;
    time::OffsetDateTime::from_unix_timestamp(cert.validity().not_after.timestamp()).map_err(|_| ())
}

/// Check if the given cert was signed by (has the same issuer as) the given CA.
/// Returns true if the cert's issuer DN matches the CA's subject DN.
fn cert_issuer_check(cert_pem: &str, ca_pem: &str) -> Result<bool, ()> {
    let (_, cert_block) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes()).map_err(|_| ())?;
    let cert = cert_block.parse_x509().map_err(|_| ())?;

    let (_, ca_block) = x509_parser::pem::parse_x509_pem(ca_pem.as_bytes()).map_err(|_| ())?;
    let ca = ca_block.parse_x509().map_err(|_| ())?;

    Ok(cert.issuer() == ca.subject())
}
