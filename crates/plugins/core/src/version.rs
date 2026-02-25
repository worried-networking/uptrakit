use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

/// A version identifier that wraps a raw string and optionally parses it as semver.
///
/// When both sides parse as semver, comparison uses semver ordering.
/// Otherwise, comparison falls back to raw string ordering.
#[derive(Clone, Debug)]
pub struct Version {
    raw: String,
    parsed: Option<semver::Version>,
}

impl Version {
    /// Create a new `Version` from a raw string.
    ///
    /// Attempts to parse the string as a semver version. If parsing fails,
    /// the version is stored as a raw string and comparisons use string ordering.
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let parsed = semver::Version::parse(&raw).ok();
        Self { raw, parsed }
    }

    /// Returns the raw version string.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Returns the parsed semver version, if available.
    pub fn semver(&self) -> Option<&semver::Version> {
        self.parsed.as_ref()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        match (&self.parsed, &other.parsed) {
            (Some(a), Some(b)) => a == b,
            _ => self.raw == other.raw,
        }
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.parsed, &other.parsed) {
            (Some(a), Some(b)) => a.cmp(b),
            _ => self.raw.cmp(&other.raw),
        }
    }
}

impl Hash for Version {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.parsed {
            Some(v) => v.hash(state),
            None => self.raw.hash(state),
        }
    }
}

impl Serialize for Version {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::new(raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parsing_succeeds() {
        let v = Version::new("1.2.3");
        assert!(v.semver().is_some());
        assert_eq!(v.as_str(), "1.2.3");
    }

    #[test]
    fn non_semver_parsing_stores_raw() {
        let v = Version::new("not-a-version");
        assert!(v.semver().is_none());
        assert_eq!(v.as_str(), "not-a-version");
    }

    #[test]
    fn semver_comparison() {
        let v1 = Version::new("1.0.0");
        let v2 = Version::new("2.0.0");
        let v3 = Version::new("1.10.0");
        assert!(v1 < v2);
        assert!(v1 < v3);
        assert!(v3 < v2);
    }

    #[test]
    fn string_fallback_comparison() {
        let v1 = Version::new("abc");
        let v2 = Version::new("def");
        assert!(v1 < v2);
    }

    #[test]
    fn mixed_comparison_uses_string() {
        // When one side is semver and the other is not, use string comparison
        let v1 = Version::new("1.0.0");
        let v2 = Version::new("not-semver");
        // String comparison: "1.0.0" < "not-semver"
        assert!(v1 < v2);
    }

    #[test]
    fn sorting() {
        let mut versions = [
            Version::new("3.0.0"),
            Version::new("1.0.0"),
            Version::new("2.0.0"),
            Version::new("1.10.0"),
        ];
        versions.sort();
        let sorted: Vec<&str> = versions.iter().map(|v| v.as_str()).collect();
        assert_eq!(sorted, ["1.0.0", "1.10.0", "2.0.0", "3.0.0"]);
    }

    #[test]
    fn serialization_roundtrip() {
        let v = Version::new("1.2.3");
        let json = serde_json::to_string(&v).expect("serialize");
        assert_eq!(json, r#""1.2.3""#);

        let deserialized: Version = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, v);
        assert!(deserialized.semver().is_some());
    }

    #[test]
    fn display_shows_raw() {
        let v = Version::new("1.2.3-beta.1");
        assert_eq!(format!("{v}"), "1.2.3-beta.1");
    }

    #[test]
    fn equality_uses_semver_when_available() {
        let v1 = Version::new("1.0.0");
        let v2 = Version::new("1.0.0");
        assert_eq!(v1, v2);
    }

    #[test]
    fn hash_consistency_with_eq() {
        use std::collections::HashSet;

        let v1 = Version::new("1.0.0");
        let v2 = Version::new("1.0.0");
        let mut set = HashSet::new();
        set.insert(v1);
        // Should not insert a duplicate
        assert!(!set.insert(v2));
    }

    #[test]
    fn prerelease_ordering() {
        let stable = Version::new("1.0.0");
        let pre = Version::new("1.0.0-alpha.1");
        // semver: pre-release versions have lower precedence than the associated normal version
        assert!(pre < stable);
    }
}
