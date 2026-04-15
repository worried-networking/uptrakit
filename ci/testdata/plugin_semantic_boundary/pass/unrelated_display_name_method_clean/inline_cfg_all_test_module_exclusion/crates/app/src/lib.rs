pub fn demo() {}

#[cfg(all(test, feature = "smoke-suite"))]
mod smoke_suite {
    fn helper_referencing_plugin_ids() {
        let _ = crate::plugin_type_id::plugin_ids::GENERIC_SHELL;
    }

    #[test]
    fn cfg_all_test_module_is_ignored() {
        helper_referencing_plugin_ids();
    }
}
