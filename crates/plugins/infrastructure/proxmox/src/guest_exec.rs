//! Execute commands inside PVE guests (LXC containers and QEMU VMs).
//!
//! Uses `pct exec` for LXC containers and `qm guest exec` for QEMU VMs.
//! All commands are run via a [`RemoteExecutor`] connected to the PVE host.

use rootcause::prelude::*;
use serde::Deserialize;
use uptrakit_command::RemoteExecutor;

use crate::{ProxmoxError, Result};

/// Type of PVE guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PveGuestType {
    /// LXC container — commands run via `pct exec`.
    Lxc,
    /// QEMU virtual machine — commands run via `qm guest exec`.
    Qemu,
}

impl PveGuestType {
    /// Returns the string representation used in PVE API responses.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Lxc => "lxc",
            Self::Qemu => "qemu",
        }
    }
}

impl std::fmt::Display for PveGuestType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Result of executing a command inside a PVE guest.
#[derive(Debug, Clone)]
pub struct GuestExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// A discovered PVE guest (VM or container).
#[derive(Debug, Clone)]
pub struct PveGuest {
    /// VM/container ID.
    pub vmid: u32,
    /// Guest name (may be empty).
    pub name: String,
    /// Guest type.
    pub guest_type: PveGuestType,
    /// Current status (e.g. "running", "stopped").
    pub status: String,
    /// PVE node name.
    pub node: String,
}

/// Execute a command inside a PVE guest.
///
/// For LXC containers, uses `pct exec {vmid} -- bash -c '{command}'`.
/// For QEMU VMs, uses `qm guest exec {vmid} -- bash -c '{command}'`
/// and parses the JSON output.
pub async fn exec_in_guest(
    executor: &dyn RemoteExecutor,
    vmid: u32,
    guest_type: PveGuestType,
    command: &str,
) -> Result<GuestExecResult> {
    let escaped = shell_escape_single(command);

    match guest_type {
        PveGuestType::Lxc => exec_in_lxc(executor, vmid, &escaped).await,
        PveGuestType::Qemu => exec_in_qemu(executor, vmid, &escaped).await,
    }
}

async fn exec_in_lxc(
    executor: &dyn RemoteExecutor,
    vmid: u32,
    escaped_command: &str,
) -> Result<GuestExecResult> {
    let cmd = format!("sudo /usr/sbin/pct exec {vmid} -- bash -c {escaped_command}");
    let result = executor
        .exec_command(&cmd)
        .await
        .context_to::<ProxmoxError>()?;

    Ok(GuestExecResult {
        stdout: result.stdout,
        stderr: result.stderr,
        exit_code: result.exit_code as i32,
    })
}

async fn exec_in_qemu(
    executor: &dyn RemoteExecutor,
    vmid: u32,
    escaped_command: &str,
) -> Result<GuestExecResult> {
    // qm guest exec returns JSON with the execution result
    let cmd = format!("sudo /usr/sbin/qm guest exec {vmid} -- bash -c {escaped_command}");
    let result = executor
        .exec_command(&cmd)
        .await
        .context_to::<ProxmoxError>()?;

    if result.exit_code != 0 {
        // qm guest exec itself failed (not the guest command)
        bail!(ProxmoxError::Plugin(format!(
            "qm guest exec {vmid} failed (exit {}): {}",
            result.exit_code,
            result.stderr.trim()
        )));
    }

    // Parse the JSON output from qm guest exec
    parse_qm_guest_exec_output(&result.stdout)
}

/// Get the IP address of a PVE guest.
///
/// - LXC: uses `pct exec {vmid} -- hostname -I` to get IPs, returns the first
/// - QEMU: uses `qm guest cmd {vmid} network-get-interfaces` to query QEMU
///   guest agent
pub async fn get_guest_ip(
    executor: &dyn RemoteExecutor,
    vmid: u32,
    guest_type: PveGuestType,
) -> Result<String> {
    match guest_type {
        PveGuestType::Lxc => get_lxc_guest_ip(executor, vmid).await,
        PveGuestType::Qemu => get_qemu_guest_ip(executor, vmid).await,
    }
}

async fn get_lxc_guest_ip(executor: &dyn RemoteExecutor, vmid: u32) -> Result<String> {
    let result = exec_in_guest(executor, vmid, PveGuestType::Lxc, "hostname -I").await?;
    if result.exit_code != 0 {
        bail!(ProxmoxError::Plugin(format!(
            "hostname -I failed in CT {vmid} (exit {}): {}",
            result.exit_code,
            result.stderr.trim()
        )));
    }

    // hostname -I returns space-separated IPs — take the first non-loopback
    result
        .stdout
        .split_whitespace()
        .find(|ip| !ip.starts_with("127."))
        .map(String::from)
        .ok_or_else(|| {
            report!(ProxmoxError::Plugin(format!(
                "no non-loopback IP found in CT {vmid}"
            )))
        })
}

async fn get_qemu_guest_ip(executor: &dyn RemoteExecutor, vmid: u32) -> Result<String> {
    let cmd = format!("sudo /usr/sbin/qm guest cmd {vmid} network-get-interfaces");
    let result = executor
        .exec_command(&cmd)
        .await
        .context_to::<ProxmoxError>()?;

    if result.exit_code != 0 {
        bail!(ProxmoxError::Plugin(format!(
            "qm guest cmd network-get-interfaces failed for VM {vmid} (exit {}): {}",
            result.exit_code,
            result.stderr.trim()
        )));
    }

    parse_qemu_network_interfaces(&result.stdout).ok_or_else(|| {
        report!(ProxmoxError::Plugin(format!(
            "no usable IPv4 address found for VM {vmid}"
        )))
    })
}

/// List all guests (VMs and containers) on the PVE cluster.
///
/// Uses `pvesh get /cluster/resources --type vm --output-format json`.
pub async fn list_guests(executor: &dyn RemoteExecutor) -> Result<Vec<PveGuest>> {
    let cmd = "pvesh get /cluster/resources --type vm --output-format json";
    let result = executor
        .exec_command(cmd)
        .await
        .context_to::<ProxmoxError>()?;

    if result.exit_code != 0 {
        bail!(ProxmoxError::Plugin(format!(
            "pvesh get /cluster/resources failed (exit {}): {}",
            result.exit_code,
            result.stderr.trim()
        )));
    }

    parse_cluster_resources(&result.stdout)
}

// ── Parsing helpers ─────────────────────────────────────────────────────────

/// Escape a command string for use inside single quotes.
fn shell_escape_single(s: &str) -> String {
    // Replace ' with '\'' (end quote, escaped quote, start quote)
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Parse `qm guest exec` JSON output.
///
/// Expected format (PVE 8.x):
/// ```json
/// {"exitcode":0,"out-data":"...\n","err-data":"...\n","exited":true}
/// ```
fn parse_qm_guest_exec_output(json: &str) -> Result<GuestExecResult> {
    #[derive(Deserialize)]
    struct QmExecOutput {
        #[serde(default)]
        exitcode: Option<i32>,
        #[serde(default, rename = "out-data")]
        out_data: Option<String>,
        #[serde(default, rename = "err-data")]
        err_data: Option<String>,
    }

    let parsed: QmExecOutput = serde_json::from_str(json.trim()).map_err(|e| {
        report!(ProxmoxError::ParseResponse(format!(
            "invalid JSON from qm guest exec: {e}"
        )))
    })?;

    Ok(GuestExecResult {
        stdout: parsed.out_data.unwrap_or_default(),
        stderr: parsed.err_data.unwrap_or_default(),
        exit_code: parsed.exitcode.unwrap_or(-1),
    })
}

/// Parse `qm guest cmd network-get-interfaces` output for the first usable IPv4.
fn parse_qemu_network_interfaces(json: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct NetworkInterface {
        #[serde(default)]
        name: String,
        #[serde(default, rename = "ip-addresses")]
        ip_addresses: Vec<IpEntry>,
    }

    #[derive(Deserialize)]
    struct IpEntry {
        #[serde(default, rename = "ip-address")]
        ip_address: String,
        #[serde(default, rename = "ip-address-type")]
        ip_address_type: String,
    }

    let interfaces: Vec<NetworkInterface> = serde_json::from_str(json.trim()).ok()?;

    for iface in &interfaces {
        if iface.name == "lo" {
            continue;
        }
        for addr in &iface.ip_addresses {
            if addr.ip_address_type == "ipv4" && !addr.ip_address.starts_with("127.") {
                return Some(addr.ip_address.clone());
            }
        }
    }

    None
}

/// Parse `pvesh get /cluster/resources --type vm` JSON output.
fn parse_cluster_resources(json: &str) -> Result<Vec<PveGuest>> {
    #[derive(Deserialize)]
    struct ClusterResource {
        #[serde(default)]
        vmid: Option<u32>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default, rename = "type")]
        resource_type: Option<String>,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        node: Option<String>,
    }

    let resources: Vec<ClusterResource> = serde_json::from_str(json.trim()).map_err(|e| {
        report!(ProxmoxError::ParseResponse(format!(
            "invalid JSON from pvesh: {e}"
        )))
    })?;

    let guests = resources
        .into_iter()
        .filter_map(|r| {
            let vmid = r.vmid?;
            let resource_type = r.resource_type?;
            let guest_type = match resource_type.as_str() {
                "lxc" => PveGuestType::Lxc,
                "qemu" => PveGuestType::Qemu,
                _ => return None,
            };

            Some(PveGuest {
                vmid,
                name: r.name.unwrap_or_default(),
                guest_type,
                status: r.status.unwrap_or_default(),
                node: r.node.unwrap_or_default(),
            })
        })
        .collect();

    Ok(guests)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_qm_exec_output_success() {
        let json = r#"{"exitcode":0,"out-data":"hello\n","err-data":"","exited":true}"#;
        let result = parse_qm_guest_exec_output(json).expect("should parse");
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "hello\n");
        assert!(result.stderr.is_empty());
    }

    #[test]
    fn parse_qm_exec_output_failure() {
        let json = r#"{"exitcode":1,"out-data":"","err-data":"error msg\n","exited":true}"#;
        let result = parse_qm_guest_exec_output(json).expect("should parse");
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.stderr, "error msg\n");
    }

    #[test]
    fn parse_qm_exec_output_missing_fields() {
        let json = r#"{"exited":true}"#;
        let result = parse_qm_guest_exec_output(json).expect("should parse");
        assert_eq!(result.exit_code, -1);
        assert!(result.stdout.is_empty());
    }

    #[test]
    fn parse_qemu_network_interfaces_valid() {
        let json = r#"[
            {"name":"lo","ip-addresses":[{"ip-address":"127.0.0.1","ip-address-type":"ipv4"}]},
            {"name":"eth0","ip-addresses":[
                {"ip-address":"192.168.1.100","ip-address-type":"ipv4"},
                {"ip-address":"fe80::1","ip-address-type":"ipv6"}
            ]}
        ]"#;
        let ip = parse_qemu_network_interfaces(json);
        assert_eq!(ip, Some("192.168.1.100".to_string()));
    }

    #[test]
    fn parse_qemu_network_interfaces_loopback_only() {
        let json = r#"[{"name":"lo","ip-addresses":[{"ip-address":"127.0.0.1","ip-address-type":"ipv4"}]}]"#;
        assert!(parse_qemu_network_interfaces(json).is_none());
    }

    #[test]
    fn parse_cluster_resources_mixed() {
        let json = r#"[
            {"vmid":100,"name":"web","type":"lxc","status":"running","node":"pve1"},
            {"vmid":200,"name":"db","type":"qemu","status":"stopped","node":"pve1"},
            {"vmid":null,"name":"storage","type":"storage","status":"available","node":"pve1"}
        ]"#;
        let guests = parse_cluster_resources(json).expect("should parse");
        assert_eq!(guests.len(), 2);
        assert_eq!(guests[0].vmid, 100);
        assert_eq!(guests[0].guest_type, PveGuestType::Lxc);
        assert_eq!(guests[1].vmid, 200);
        assert_eq!(guests[1].guest_type, PveGuestType::Qemu);
    }

    #[test]
    fn parse_cluster_resources_empty() {
        let guests = parse_cluster_resources("[]").expect("should parse");
        assert!(guests.is_empty());
    }

    #[test]
    fn shell_escape_single_basic() {
        assert_eq!(shell_escape_single("echo hello"), "'echo hello'");
    }

    #[test]
    fn shell_escape_single_with_quotes() {
        assert_eq!(
            shell_escape_single("echo 'hello'"),
            "'echo '\\''hello'\\'''",
        );
    }

    #[test]
    fn pve_guest_type_display() {
        assert_eq!(PveGuestType::Lxc.to_string(), "lxc");
        assert_eq!(PveGuestType::Qemu.to_string(), "qemu");
    }
}
