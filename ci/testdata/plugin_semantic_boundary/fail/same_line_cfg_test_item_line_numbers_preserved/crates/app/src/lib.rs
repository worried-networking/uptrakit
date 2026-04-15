pub fn production_ok() {}

#[cfg(test)] mod tests_inline {
    pub fn inline_cfg_helpers_are_ignored() {
        let _ = crate::plugin_type_id::plugin_ids::GENERIC_SHELL;
    }
}

pub fn production_violation_keeps_original_line_number() {
    let _ = crate::plugin_type_id::plugin_ids::HOOK_SYSTEMD;
}
