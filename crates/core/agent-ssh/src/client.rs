use std::time::Duration;

use uptrakit_internal_wire::{HostInfo, ReportHostsPayload, ServiceMessage};
use uptrakit_service_sdk::ControllerConnection;

use crate::host_info::collect_remote_host_info;
use crate::host_ops::list_hosts;
use crate::ssh_transport::{AuthMethod, SshConnectionConfig};

/// Connect to each enrolled SSH host, collect system info, and send a
/// `ReportHosts` message to the controller.
///
/// Errors for individual hosts are logged as warnings and skipped.
pub(crate) async fn report_enrolled_hosts(
    local_db: &sea_orm::DatabaseConnection,
    conn: &mut ControllerConnection,
) {
    let hosts = match list_hosts(local_db).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list SSH hosts for reporting");
            return;
        }
    };

    let mut host_infos: Vec<HostInfo> = Vec::with_capacity(hosts.len());

    for host in &hosts {
        tracing::debug!(host_name = %host.name, hostname = %host.hostname, "collecting host info");

        let config = SshConnectionConfig {
            hostname: host.hostname.clone(),
            port: host.port as u16,
            connect_timeout: Duration::from_secs(10),
        };

        let private_key_pem = host.private_key.expose_secret();
        let auth = AuthMethod::PrivateKey(private_key_pem);

        let (session, _fingerprint) = match crate::ssh_transport::connect_and_authenticate(
            &config,
            &host.username,
            &auth,
            host.host_key_fingerprint.as_deref(),
        )
        .await
        {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!(
                    host_name = %host.name,
                    hostname = %host.hostname,
                    error = %e,
                    "failed to connect to SSH host for reporting, skipping"
                );
                continue;
            }
        };

        let mut info = collect_remote_host_info(&session).await;
        // Set the SSH target address as the host's ip_address.
        info.ip_address = Some(host.hostname.clone());

        session.disconnect().await;

        tracing::debug!(
            host_name = %host.name,
            machine_id = %info.machine_id,
            hostname = ?info.hostname,
            "collected remote host info"
        );

        host_infos.push(info);
    }

    let agent_version = env!("CARGO_PKG_VERSION").to_string();
    let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
        hosts: host_infos,
        agent_version,
        protocol_version: uptrakit_internal_wire::PROTOCOL_VERSION,
    });

    if let Err(e) = conn.send(msg).await {
        tracing::warn!(error = %e, "failed to send ReportHosts message");
    } else {
        tracing::info!(
            host_count = hosts.len(),
            "reported enrolled hosts to controller"
        );
    }
}
