use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;

use crate::AppState;
use crate::middleware::permission::CanManageGlobalSettings;

pub use uptrakit_web_api_types::system_alerts::{AlertSeverity, SystemAlert, SystemAlertsResponse};

/// Get system alerts for the admin dashboard.
#[utoipa::path(
    get,
    path = "/api/v1/system/alerts",
    tag = "System",
    responses(
        (status = 200, description = "System alerts", body = SystemAlertsResponse),
        (status = 403, description = "Not authorized")
    ),
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_system_alerts(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
) -> Response {
    let snapshot = state.cert.ca_snapshot.borrow().clone();
    let mut alerts = Vec::new();

    // Check if managed CA is approaching expiration
    if snapshot.managed {
        let now = time::OffsetDateTime::now_utc();
        let days_until_expiry = (snapshot.active_not_after - now).whole_days();

        if days_until_expiry <= 183 {
            alerts.push(SystemAlert {
                id: "ca_expiring".to_string(),
                severity: AlertSeverity::Warning,
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

    // Check if server cert is signed by a non-active trusted CA (needs re-issue)
    let server_cert_path = state.pki_path.join("server.crt");
    if let Ok(server_cert_pem) = std::fs::read_to_string(&server_cert_path)
        && let Ok(server_not_after) = cert_not_after_from_pem(&server_cert_pem)
    {
        let signed_by_active =
            crate::pki_utils::cert_signed_by_ca(&server_cert_pem, &snapshot.active_cert_pem)
                .unwrap_or(false);

        let signed_by_trusted = snapshot.trusted_cas.iter().any(|ca| {
            crate::pki_utils::cert_signed_by_ca(&server_cert_pem, &ca.cert_pem).unwrap_or(false)
        });

        if signed_by_trusted && !signed_by_active {
            alerts.push(SystemAlert {
                id: "server_cert_old_ca".to_string(),
                severity: AlertSeverity::Info,
                title: "Server Certificate Under Non-Active CA".to_string(),
                message: "The HTTPS server certificate was signed by a non-active CA. \
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
                severity: AlertSeverity::Warning,
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

    (StatusCode::OK, Json(SystemAlertsResponse { alerts })).into_response()
}

/// Extract not_after timestamp from a PEM cert.
fn cert_not_after_from_pem(pem: &str) -> Result<time::OffsetDateTime, ()> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(pem.as_bytes()).map_err(|_| ())?;
    let cert = pem_block.parse_x509().map_err(|_| ())?;
    time::OffsetDateTime::from_unix_timestamp(cert.validity().not_after.timestamp()).map_err(|_| ())
}
