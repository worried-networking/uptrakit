pub fn demo() {}

#[tokio::test]
async fn namespaced_tests_are_ignored() {
    let _ = crate::plugin_type_id::plugin_ids::GENERIC_SHELL;
}

#[cfg(test)]
#[allow(dead_code)]
mod smoke_suite {
    #[test]
    fn inline_tests_are_ignored() {
        let _ = crate::plugin_type_id::plugin_ids::GENERIC_SHELL;
    }
}
