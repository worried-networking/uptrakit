use uptrakit_plugin_infrastructure_registry::PluginOps;

pub fn accepts_plugin_ops(_ops: &dyn PluginOps) {
    // releases_github appears in prose here but should never be scanned as identity code.
    let note = r#"runtime notes mention releases_github in docs only"#;
    let non_canonical_plugin_type = "legacy_migration_only";
    let _ = (note, non_canonical_plugin_type);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_tests_are_allowed() {
        assert_eq!("alpha", "alpha");
    }
}
