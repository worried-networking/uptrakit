use uptrakit_internal_wire::HostInfo;

/// Collect host information for the current machine.
pub fn collect_host_info() -> HostInfo {
    HostInfo {
        machine_id: read_machine_id(),
        os_type: Some(std::env::consts::OS.to_string()),
        os_version: read_os_version(),
        architecture: Some(std::env::consts::ARCH.to_string()),
        hostname: read_hostname(),
        ip_address: None, // Controller knows the connection IP from the service record.
    }
}

/// Read the system hostname.
///
/// Tries FQDN first (`hostname -f`), falls back to short hostname.
fn read_hostname() -> Option<String> {
    // Try FQDN first.
    if let Ok(output) = std::process::Command::new("hostname").arg("-f").output()
        && output.status.success()
    {
        let fqdn = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !fqdn.is_empty() {
            return Some(fqdn);
        }
    }

    // Fall back to short hostname.
    if let Ok(output) = std::process::Command::new("hostname").output()
        && output.status.success()
    {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }

    None
}

/// Read the persistent machine identifier.
///
/// - Linux: `/etc/machine-id`
/// - macOS: `IOPlatformUUID` via `ioreg`
/// - Fallback: `"unknown-<random-uuid>"` (see note below)
///
/// # Fallback behaviour
///
/// When no persistent machine identifier can be determined (containers without
/// `/etc/machine-id`, exotic operating systems, permission errors), a
/// session-unique fallback of the form `"unknown-<uuidv7>"` is generated and a
/// `WARN`-level log line is emitted.  The fallback is unique within a process
/// lifetime but does **not** persist across restarts, so the agent will appear
/// as a different host after each restart.  Operators should provision
/// `/etc/machine-id` in containerised environments to avoid this.
///
/// See [`docs/security/security-architecture.md`] for security implications.
fn read_machine_id() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(contents) = std::fs::read_to_string("/etc/machine-id") {
            let trimmed = contents.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.contains("IOPlatformUUID") {
                    // Line format: "IOPlatformUUID" = "XXXXXXXX-XXXX-..."
                    if let Some(uuid) = line.split('"').nth(3) {
                        return uuid.to_string();
                    }
                }
            }
        }
    }

    let fallback = format!("unknown-{}", uuid::Uuid::now_v7());
    tracing::warn!(
        fallback,
        "machine-ID could not be determined; using session-unique fallback. \
         Host identity will not persist across restarts. \
         Provision /etc/machine-id (Linux) or ensure ioreg access (macOS) to fix this. \
         See docs/security/security-architecture.md."
    );
    fallback
}

/// Read a human-readable OS version string.
///
/// - Linux: `PRETTY_NAME` from `/etc/os-release`
/// - macOS: `sw_vers` output (e.g. "macOS 15.2")
fn read_os_version() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(contents) = std::fs::read_to_string("/etc/os-release") {
            for line in contents.lines() {
                if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
                    return Some(value.trim_matches('"').to_string());
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("sw_vers").output()
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut product_name = None;
            let mut product_version = None;
            for line in stdout.lines() {
                if let Some(val) = line.strip_prefix("ProductName:") {
                    product_name = Some(val.trim().to_string());
                }
                if let Some(val) = line.strip_prefix("ProductVersion:") {
                    product_version = Some(val.trim().to_string());
                }
            }
            if let (Some(name), Some(version)) = (product_name, product_version) {
                return Some(format!("{name} {version}"));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_host_info_returns_valid_data() {
        let info = collect_host_info();
        // machine_id should never be empty
        assert!(!info.machine_id.is_empty());
        // os_type should match the current OS
        assert_eq!(info.os_type.as_deref(), Some(std::env::consts::OS));
        // architecture should match the current arch
        assert_eq!(info.architecture.as_deref(), Some(std::env::consts::ARCH));
        // hostname should be present on most systems
        assert!(info.hostname.is_some(), "hostname should be detected");
        // ip_address is intentionally None for the regular agent
        assert_eq!(info.ip_address, None);
    }

    /// `read_machine_id` must never return an empty string, even on platforms
    /// where neither `/etc/machine-id` nor `ioreg` are available.
    #[test]
    fn machine_id_is_never_empty() {
        let id = read_machine_id();
        assert!(!id.is_empty(), "machine_id must not be empty");
    }

    /// When a persistent machine ID is unavailable the fallback starts with
    /// `"unknown-"` and is long enough to contain a UUID suffix.
    ///
    /// Note: this test only exercises the fallback branch when the current
    /// platform cannot determine a real machine ID (e.g. containers without
    /// `/etc/machine-id` on Linux CI). On macOS the test verifies the ID is
    /// non-empty; the `"unknown-"` prefix is tested indirectly via `starts_with`.
    #[test]
    fn machine_id_fallback_has_unknown_prefix_or_real_value() {
        let id = read_machine_id();
        // Either a real machine ID (non-empty) or the session-unique fallback.
        // The fallback always starts with "unknown-".
        assert!(!id.is_empty());
        if let Some(suffix) = id.strip_prefix("unknown-") {
            // Verify it has the UUID suffix (format: "unknown-<uuidv7>").
            assert!(
                !suffix.is_empty(),
                "fallback machine-ID must have a non-empty UUID suffix, got: {id}"
            );
        }
    }
}
