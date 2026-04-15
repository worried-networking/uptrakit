pub fn demo() {}

#[tokio::test]
async fn namespaced_tests_are_ignored() {
    let _ = crate::plugin_type_id::plugin_ids::GENERIC_SHELL;
}

#[test] // trailing comments should still mark test-only items
fn standalone_tests_with_inline_comments_are_ignored() {
    let _ = crate::plugin_type_id::plugin_ids::HOOK_SYSTEMD;
}

#[cfg(test)] // trailing comments should still mark test-only items
#[allow(dead_code)]
mod smoke_suite {
    #[test]
    fn inline_tests_are_ignored() {
        let _ = crate::plugin_type_id::plugin_ids::GENERIC_SHELL;
    }
}
