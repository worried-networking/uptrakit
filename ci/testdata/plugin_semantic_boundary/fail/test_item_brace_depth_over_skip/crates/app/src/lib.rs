pub fn production_ok() {}

#[cfg(test)]
mod tests {
    #[test]
    fn braces_inside_literals_comments_and_chars_do_not_extend_cfg_skip_range() {
        let _ = "{";
        let _ = '{';
        let _ = '\\';
        let _ = '\'';
        // {
        assert_eq!(1, 1);
    }
}

#[test]
fn braces_inside_chars_do_not_extend_test_skip_range() {
    let _ = '{';
    let _ = '\\';
    let _ = '\'';
    assert_eq!(1, 1);
}

pub fn production_violation_after_cfg_test_is_still_seen() {
    let _ = crate::plugin_type_id::plugin_ids::GENERIC_SHELL;
}

pub fn production_violation_after_test_function_is_still_seen() {
    let _ = crate::plugin_type_id::plugin_ids::HOOK_SYSTEMD;
}
