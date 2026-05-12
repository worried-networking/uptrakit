pub mod scope;

pub use scope::Scope;

// Section modules and RuntimeConfig added in Tasks 4-6.
/// Placeholder — expanded in Task 6.
pub struct RuntimeConfig;

impl RuntimeConfig {
    /// Placeholder validator.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn validate(&self) -> Result<(), rootcause::Report> {
        Ok(())
    }

    /// Placeholder extras warning collector.
    #[must_use]
    pub fn warn_about_extras(&self) -> Vec<String> {
        Vec::new()
    }
}
