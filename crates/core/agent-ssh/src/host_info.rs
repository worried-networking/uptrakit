//! Remote host information collection over SSH.
//!
//! Collects system information from a remote host by executing standard
//! shell commands over an established SSH session. Each field is collected
//! independently — failures for individual fields produce `None` (or
//! `"unknown"` for `machine_id`) without aborting the entire collection.

use uptrakit_command::CommandExecutor;
use uptrakit_wire::HostInfo;

use crate::ssh_transport::SshSession;

/// Collect host information from a remote machine via SSH.
///
/// Runs lightweight commands (`cat /etc/machine-id`, `uname`, `hostname`)
/// over the given session. Feature probing is delegated to
/// [`uptrakit_agent_core::host_info::probe_host_features`] via the
/// provided `executor` (typically an `SshCommandExecutor` wrapping the
/// same session).
pub(crate) async fn collect_remote_host_info(
    session: &SshSession,
    executor: &dyn CommandExecutor,
) -> HostInfo {
    let machine_id = read_remote_machine_id(session).await;
    let os_type = read_remote_os_type(session).await;
    let os_version = read_remote_os_version(session).await;
    let architecture = read_remote_architecture(session).await;
    let hostname = read_remote_hostname(session).await;

    // Probe features via the CommandExecutor abstraction (shared with agent-core).
    let features = uptrakit_agent_core::host_info::probe_host_features(executor).await;

    HostInfo {
        machine_id,
        os_type,
        os_version,
        architecture,
        hostname,
        ip_address: None,    // Set by caller from the SSH host's address.
        agent_host_id: None, // Set by caller from the local host DB UUID.
        features: Some(features),
    }
}

/// Read the persistent machine identifier from a remote host.
///
/// Tries `/etc/machine-id` (Linux), then `ioreg` (macOS).
///
/// # Fallback behaviour
///
/// When neither source is available (containers, exotic OS, permission errors),
/// a session-unique fallback of the form `"unknown-<uuidv7>"` is generated and
/// a `WARN`-level log line is emitted.  The fallback is unique per process
/// invocation but does **not** persist across SSH agent restarts, so the remote
/// host will appear with a different machine ID after each reconnect.  Operators
/// should provision `/etc/machine-id` in containerised remote environments.
///
/// See [`docs/security/security-architecture.md`] for security implications.
async fn read_remote_machine_id(session: &SshSession) -> String {
    // Linux: /etc/machine-id
    if let Ok(result) = session
        .exec_command("cat /etc/machine-id 2>/dev/null")
        .await
        && result.exit_code == 0
    {
        let trimmed = result.stdout.trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }

    // macOS: ioreg IOPlatformUUID
    if let Ok(result) = session
        .exec_command("ioreg -rd1 -c IOPlatformExpertDevice 2>/dev/null")
        .await
        && result.exit_code == 0
        && let Some(uuid) = parse_ioplatform_uuid(&result.stdout)
    {
        return uuid;
    }

    let fallback = format!("unknown-{}", uuid::Uuid::now_v7());
    tracing::warn!(
        fallback,
        "remote machine-ID could not be determined; using session-unique fallback. \
         Host identity will not persist across restarts. \
         Provision /etc/machine-id on the remote host to fix this. \
         See docs/security/security-architecture.md."
    );
    fallback
}

/// Read the OS type from a remote host via `uname -s`.
async fn read_remote_os_type(session: &SshSession) -> Option<String> {
    let result = session.exec_command("uname -s 2>/dev/null").await.ok()?;
    if result.exit_code != 0 {
        return None;
    }
    let raw = result.stdout.trim();
    if raw.is_empty() {
        return None;
    }
    Some(normalize_os_type(raw))
}

/// Read a human-readable OS version string from a remote host.
///
/// Tries `/etc/os-release` `PRETTY_NAME` (Linux), then `sw_vers` (macOS).
async fn read_remote_os_version(session: &SshSession) -> Option<String> {
    // Linux: /etc/os-release
    if let Ok(result) = session
        .exec_command("cat /etc/os-release 2>/dev/null")
        .await
        && result.exit_code == 0
        && let Some(pretty) = parse_pretty_name(&result.stdout)
    {
        return Some(pretty);
    }

    // macOS: sw_vers
    if let Ok(result) = session.exec_command("sw_vers 2>/dev/null").await
        && result.exit_code == 0
        && let Some(ver) = parse_sw_vers(&result.stdout)
    {
        return Some(ver);
    }

    None
}

/// Read the CPU architecture from a remote host via `uname -m`.
async fn read_remote_architecture(session: &SshSession) -> Option<String> {
    let result = session.exec_command("uname -m 2>/dev/null").await.ok()?;
    if result.exit_code != 0 {
        return None;
    }
    let trimmed = result.stdout.trim().to_string();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}

/// Read the hostname from a remote host.
///
/// Tries FQDN first (`hostname -f`), falls back to short hostname.
async fn read_remote_hostname(session: &SshSession) -> Option<String> {
    // Try FQDN first.
    if let Ok(result) = session.exec_command("hostname -f 2>/dev/null").await
        && result.exit_code == 0
    {
        let trimmed = result.stdout.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }

    // Fall back to short hostname.
    if let Ok(result) = session.exec_command("hostname 2>/dev/null").await
        && result.exit_code == 0
    {
        let trimmed = result.stdout.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }

    None
}

// ── Parsing helpers (public for testing) ─────────────────────────────

/// Normalize OS type string: "Linux" -> "linux", "Darwin" -> "macos".
fn normalize_os_type(raw: &str) -> String {
    match raw.trim() {
        "Darwin" => "macos".to_string(),
        other => other.to_lowercase(),
    }
}

/// Extract `IOPlatformUUID` from `ioreg` output.
fn parse_ioplatform_uuid(output: &str) -> Option<String> {
    for line in output.lines() {
        if line.contains("IOPlatformUUID") {
            // Line format: "IOPlatformUUID" = "XXXXXXXX-XXXX-..."
            return line.split('"').nth(3).map(|s| s.to_string());
        }
    }
    None
}

/// Extract `PRETTY_NAME` value from `/etc/os-release` content.
fn parse_pretty_name(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

/// Parse macOS `sw_vers` output into "ProductName ProductVersion".
fn parse_sw_vers(output: &str) -> Option<String> {
    let mut product_name = None;
    let mut product_version = None;
    for line in output.lines() {
        if let Some(val) = line.strip_prefix("ProductName:") {
            product_name = Some(val.trim().to_string());
        }
        if let Some(val) = line.strip_prefix("ProductVersion:") {
            product_version = Some(val.trim().to_string());
        }
    }
    match (product_name, product_version) {
        (Some(name), Some(version)) => Some(format!("{name} {version}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── normalize_os_type ────────────────────────────────────────────

    #[test]
    fn normalizes_linux() {
        assert_eq!(normalize_os_type("Linux"), "linux");
    }

    #[test]
    fn normalizes_darwin_to_macos() {
        assert_eq!(normalize_os_type("Darwin"), "macos");
    }

    #[test]
    fn normalizes_freebsd() {
        assert_eq!(normalize_os_type("FreeBSD"), "freebsd");
    }

    #[test]
    fn normalizes_with_whitespace() {
        assert_eq!(normalize_os_type("  Linux  "), "linux");
    }

    // ── parse_ioplatform_uuid ────────────────────────────────────────

    #[test]
    fn parses_ioreg_uuid() {
        let output = r#"+-o Root  <class IORegistryEntry>
    | {
    |   "IOPlatformUUID" = "ABCD1234-5678-EFGH-IJKL-MNOPQRSTUVWX"
    |   "IOPlatformSerialNumber" = "XYZ"
    | }"#;
        assert_eq!(
            parse_ioplatform_uuid(output),
            Some("ABCD1234-5678-EFGH-IJKL-MNOPQRSTUVWX".to_string())
        );
    }

    #[test]
    fn ioreg_no_uuid() {
        assert_eq!(parse_ioplatform_uuid("no uuid here"), None);
    }

    // ── parse_pretty_name ────────────────────────────────────────────

    #[test]
    fn parses_pretty_name_quoted() {
        let content =
            "ID=debian\nPRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\nVERSION_ID=\"12\"";
        assert_eq!(
            parse_pretty_name(content),
            Some("Debian GNU/Linux 12 (bookworm)".to_string())
        );
    }

    #[test]
    fn parses_pretty_name_unquoted() {
        let content = "PRETTY_NAME=Ubuntu 24.04 LTS";
        assert_eq!(
            parse_pretty_name(content),
            Some("Ubuntu 24.04 LTS".to_string())
        );
    }

    #[test]
    fn pretty_name_missing() {
        assert_eq!(parse_pretty_name("ID=alpine\nVERSION=3.19"), None);
    }

    // ── parse_sw_vers ────────────────────────────────────────────────

    #[test]
    fn parses_sw_vers_output() {
        let output = "ProductName:\tmacOS\nProductVersion:\t15.2\nBuildVersion:\t24C101";
        assert_eq!(parse_sw_vers(output), Some("macOS 15.2".to_string()));
    }

    #[test]
    fn sw_vers_missing_fields() {
        assert_eq!(parse_sw_vers("ProductName:\tmacOS"), None);
    }

    // ── Additional parse_pretty_name tests ────────────────────────────

    #[test]
    fn parses_pretty_name_alpine() {
        let content = "NAME=\"Alpine Linux\"\nID=alpine\nVERSION_ID=3.19\nPRETTY_NAME=\"Alpine Linux v3.19\"\nHOME_URL=\"https://alpinelinux.org/\"";
        assert_eq!(
            parse_pretty_name(content),
            Some("Alpine Linux v3.19".to_string())
        );
    }

    #[test]
    fn pretty_name_empty_content() {
        assert_eq!(parse_pretty_name(""), None);
    }

    #[test]
    fn pretty_name_no_quotes() {
        // Some distributions use unquoted PRETTY_NAME.
        let content = "PRETTY_NAME=Arch Linux";
        assert_eq!(parse_pretty_name(content), Some("Arch Linux".to_string()));
    }

    // ── Additional normalize_os_type tests ────────────────────────────

    #[test]
    fn normalizes_empty_string() {
        assert_eq!(normalize_os_type(""), "");
    }

    // ── parse_sw_vers edge cases ──────────────────────────────────────

    #[test]
    fn sw_vers_empty_output() {
        assert_eq!(parse_sw_vers(""), None);
    }

    #[test]
    fn sw_vers_only_version() {
        assert_eq!(parse_sw_vers("ProductVersion:\t15.2"), None);
    }

    // ── parse_ioplatform_uuid edge cases ──────────────────────────────

    #[test]
    fn ioreg_empty_output() {
        assert_eq!(parse_ioplatform_uuid(""), None);
    }

    // ── machine-ID fallback format ────────────────────────────────────

    /// Verify the session-unique fallback has the expected "unknown-<uuid>"
    /// format so it is clearly distinguishable from a real machine ID.
    /// We cannot exercise  without a live SSH session,
    /// so we replicate the generation logic here.
    #[test]
    fn machine_id_fallback_starts_with_unknown_dash() {
        let fallback = format!("unknown-{}", uuid::Uuid::now_v7());
        assert!(
            fallback.starts_with("unknown-"),
            "fallback must start with 'unknown-', got: {fallback}"
        );
        let suffix = &fallback["unknown-".len()..];
        assert!(
            !suffix.is_empty(),
            "fallback must have a non-empty UUID suffix, got: {fallback}"
        );
    }

    #[test]
    fn two_machine_id_fallbacks_are_distinct() {
        // UUIDv7 is monotonically increasing — even sub-millisecond, two
        // calls produce different values, ensuring distinct agent identities.
        let a = format!("unknown-{}", uuid::Uuid::now_v7());
        let b = format!("unknown-{}", uuid::Uuid::now_v7());
        assert_ne!(a, b, "two fallback machine-IDs must be distinct");
    }
}
