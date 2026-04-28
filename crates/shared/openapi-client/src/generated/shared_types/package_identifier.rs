//! Shared validation rules for package manager identifier strings.
//!
//! Each package manager plugin has a `validate_identifier()` function with
//! structurally identical logic: empty check → optional minimum length → maximum
//! length → first-character check → allowed character set → optional double-dot
//! check.  `PackageIdentifierRules` captures the varying constants so plugins
//! can delegate the common case without duplicating the branching logic.
/// Validation rules for a package manager's `package_identifier` field.
///
/// Construct a `const` instance in each plugin and call [`PackageIdentifierRules::validate`]
/// from the plugin's own `validate_identifier` function.
///
/// Plugins with non-standard rules (e.g. Homebrew's path-segment checks, npm's
/// `@scope/name` format, Snap's no-consecutive-hyphen rule) should keep any
/// extra validation in their own function and call this for the common subset.
pub struct PackageIdentifierRules {
    /// Minimum acceptable length. Use `1` for no effective minimum.
    pub min_len: usize,
    /// Maximum acceptable length.
    pub max_len: usize,
    /// Returns `true` when the first character of the identifier is valid.
    pub first_char_valid: fn(char) -> bool,
    /// Returns `true` when a character anywhere in the identifier is valid.
    pub char_valid: fn(char) -> bool,
    /// When `true`, identifiers containing `".."` are rejected.
    pub reject_double_dot: bool,
}
impl PackageIdentifierRules {
    /// Validate `value` against this rule set.
    ///
    /// Returns `Ok(())` when every rule passes, or `Err(String)` with a
    /// human-readable message describing the first failure.
    pub fn validate(&self, value: &str) -> Result<(), String> {
        if value.is_empty() {
            return Err("package_identifier must not be empty".to_string());
        }
        if value.len() < self.min_len {
            return Err(format!(
                "package_identifier must be at least {} characters long",
                self.min_len
            ));
        }
        if value.len() > self.max_len {
            return Err(format!(
                "package_identifier must not exceed {} characters",
                self.max_len
            ));
        }
        let first = value.chars().next().unwrap_or('\0');
        if !(self.first_char_valid)(first) {
            return Err(format!(
                "package_identifier starts with an invalid character: '{first}'"
            ));
        }
        for ch in value.chars() {
            if !(self.char_valid)(ch) {
                return Err(format!(
                    "package_identifier contains invalid character: '{ch}'"
                ));
            }
        }
        if self.reject_double_dot && value.contains("..") {
            return Err("package_identifier must not contain '..'".to_string());
        }
        Ok(())
    }
}
