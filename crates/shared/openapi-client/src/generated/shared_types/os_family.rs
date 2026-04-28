use serde::{Deserialize, Serialize};
/// Operating system family. Derived from `host.os_type` at the DB/wire boundary.
///
/// NOT a wire type — the wire carries `os_type: String` as before. This enum is
/// parsed from that string. Unknown values yield `None` at the call site.
///
/// All variants are `Copy`, enabling `&'static [OsFamily]` in role slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum OsFamily {
    Linux,
    MacOs,
    FreeBsd,
    /// Groundwork for future MikroTik/RouterOS support. No runtime implementation yet.
    RouterOs,
    /// Groundwork for future Windows support. No runtime implementation yet.
    Windows,
}
impl OsFamily {
    /// Parse from the existing `host.os_type` string. Returns `None` for unknown values.
    pub fn from_os_type(s: &str) -> Option<Self> {
        match s {
            "linux" => Some(Self::Linux),
            "macos" => Some(Self::MacOs),
            "freebsd" => Some(Self::FreeBsd),
            "routeros" => Some(Self::RouterOs),
            "windows" => Some(Self::Windows),
            _ => None,
        }
    }
}
impl std::fmt::Display for OsFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Linux => "linux",
            Self::MacOs => "macos",
            Self::FreeBsd => "freebsd",
            Self::RouterOs => "routeros",
            Self::Windows => "windows",
        };
        f.write_str(s)
    }
}
