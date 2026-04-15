use uptrakit_plugin_infrastructure_registry::PluginOps;

pub struct BrandingConfig {
    name: String,
}

impl BrandingConfig {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }

    pub fn display_name(&self) -> &str {
        &self.name
    }
}

pub fn accepts_plugin_ops(_ops: &dyn PluginOps) {
    // releases_github appears in prose here but should never be scanned as identity code.
    let note = r#"runtime notes mention releases_github in docs only"#;
    let non_canonical_plugin_type = "legacy_migration_only";
    let branding = BrandingConfig::new("uptrakit");
    let _ = (note, non_canonical_plugin_type, branding.display_name());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_tests_are_allowed() {
        assert_eq!("alpha", "alpha");
    }
}
