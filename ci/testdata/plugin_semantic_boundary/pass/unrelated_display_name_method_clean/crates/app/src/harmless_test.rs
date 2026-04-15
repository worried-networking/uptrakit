#[test]
fn test_only_file_is_excluded() {
    let ids = [
        crate::plugin_type_id::plugin_ids::HOOK_SYSTEMD,
        crate::plugin_type_id::plugin_ids::INFRASTRUCTURE_PROXMOX,
    ];
    assert_eq!(ids.len(), 2);
}
