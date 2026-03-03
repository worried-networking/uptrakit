use std::fmt;
use std::str::FromStr;

/// Global audit log filter mode, set via CLI flag or per-tenant setting override.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilterMode {
    /// Log all authenticated requests (default).
    #[default]
    All,
    /// Log only mutation requests (POST, PUT, PATCH, DELETE).
    Mutations,
    /// Disable audit logging entirely.
    None,
}

impl FilterMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Mutations => "mutations",
            Self::None => "none",
        }
    }
}

impl fmt::Display for FilterMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for FilterMode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "all" => Ok(Self::All),
            "mutations" => Ok(Self::Mutations),
            "none" => Ok(Self::None),
            other => Err(format!("unknown audit log filter mode: {other}")),
        }
    }
}

/// Audit log filter that checks whether a given HTTP method should be logged.
///
/// Combines a global filter mode with an optional per-tenant override.
#[derive(Clone, Debug)]
pub struct AuditFilter {
    global_mode: FilterMode,
}

impl AuditFilter {
    pub fn new(global_mode: FilterMode) -> Self {
        Self { global_mode }
    }

    /// Returns `true` if the request should be logged.
    ///
    /// `per_tenant_override` allows per-tenant settings to override the global mode.
    /// If `None`, the global mode applies.
    pub fn should_log(&self, method: &str, per_tenant_override: Option<FilterMode>) -> bool {
        let mode = per_tenant_override.unwrap_or(self.global_mode);
        match mode {
            FilterMode::All => true,
            FilterMode::Mutations => is_mutation(method),
            FilterMode::None => false,
        }
    }
}

impl Default for AuditFilter {
    fn default() -> Self {
        Self {
            global_mode: FilterMode::All,
        }
    }
}

/// Returns `true` for HTTP methods that modify server state.
fn is_mutation(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "POST" | "PUT" | "PATCH" | "DELETE"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_mode_from_str() {
        assert_eq!("all".parse::<FilterMode>().unwrap(), FilterMode::All);
        assert_eq!(
            "mutations".parse::<FilterMode>().unwrap(),
            FilterMode::Mutations
        );
        assert_eq!("none".parse::<FilterMode>().unwrap(), FilterMode::None);
        assert_eq!("ALL".parse::<FilterMode>().unwrap(), FilterMode::All);
        assert!("invalid".parse::<FilterMode>().is_err());
    }

    #[test]
    fn filter_mode_display() {
        assert_eq!(FilterMode::All.to_string(), "all");
        assert_eq!(FilterMode::Mutations.to_string(), "mutations");
        assert_eq!(FilterMode::None.to_string(), "none");
    }

    #[test]
    fn filter_all_logs_everything() {
        let filter = AuditFilter::new(FilterMode::All);
        assert!(filter.should_log("GET", None));
        assert!(filter.should_log("POST", None));
        assert!(filter.should_log("DELETE", None));
    }

    #[test]
    fn filter_mutations_only() {
        let filter = AuditFilter::new(FilterMode::Mutations);
        assert!(!filter.should_log("GET", None));
        assert!(!filter.should_log("HEAD", None));
        assert!(!filter.should_log("OPTIONS", None));
        assert!(filter.should_log("POST", None));
        assert!(filter.should_log("PUT", None));
        assert!(filter.should_log("PATCH", None));
        assert!(filter.should_log("DELETE", None));
    }

    #[test]
    fn filter_none_logs_nothing() {
        let filter = AuditFilter::new(FilterMode::None);
        assert!(!filter.should_log("GET", None));
        assert!(!filter.should_log("POST", None));
    }

    #[test]
    fn per_tenant_override_takes_precedence() {
        let filter = AuditFilter::new(FilterMode::All);
        // Global = All, but per-tenant override = None → no logging
        assert!(!filter.should_log("POST", Some(FilterMode::None)));
        // Global = None, but per-tenant override = All → log
        let filter = AuditFilter::new(FilterMode::None);
        assert!(filter.should_log("POST", Some(FilterMode::All)));
    }

    #[test]
    fn per_tenant_mutations_override() {
        let filter = AuditFilter::new(FilterMode::All);
        // Global = All, but tenant says mutations only
        assert!(!filter.should_log("GET", Some(FilterMode::Mutations)));
        assert!(filter.should_log("POST", Some(FilterMode::Mutations)));
    }

    #[test]
    fn default_filter_is_all() {
        let filter = AuditFilter::default();
        assert!(filter.should_log("GET", None));
        assert!(filter.should_log("POST", None));
    }
}
