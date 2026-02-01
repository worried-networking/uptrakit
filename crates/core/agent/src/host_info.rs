use uptrakit_internal_wire::HostInfo;

/// Collect host information for the current machine.
pub fn collect_host_info() -> HostInfo {
    HostInfo {
        machine_id: read_machine_id(),
        os_type: Some(std::env::consts::OS.to_string()),
        os_version: read_os_version(),
        architecture: Some(std::env::consts::ARCH.to_string()),
    }
}

/// Read the persistent machine identifier.
///
/// - Linux: `/etc/machine-id`
/// - macOS: `IOPlatformUUID` via `ioreg`
/// - Fallback: `"unknown"`
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

    "unknown".to_string()
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
    }
}
