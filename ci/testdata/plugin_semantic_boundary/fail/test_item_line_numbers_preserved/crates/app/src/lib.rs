pub fn production_ok() {}

#[cfg(test)]
mod cfg_tests {
    #[test]
    fn cfg_test_section_is_stripped_without_shifting_lines() {
        let _ = "{";
        // }
        assert_eq!(1, 1);
    }
}

#[test]
fn standalone_test_is_stripped_without_shifting_lines() {
    let _ = "{";
    // }
    assert_eq!(1, 1);
}

pub fn production_violation_keeps_original_line_number() {
    let _ = crate::plugin_type_id::plugin_ids::GENERIC_SHELL;
}
