use uptrakit_surfaces::{
    BuiltInApiOperationId, ControllerQueryId, DataSourceId, InteractionId, SLOT_HOST_DETAIL_TABS,
    SLOT_SETTINGS_TABS, SLOT_SURFACE_PAGE, SurfaceId, all_surface_slots,
    is_valid_surface_identifier, slot_def, validate_surface_identifier,
};

#[test]
fn ids_accept_valid_values() {
    let valid = "settings.tab_overview-1";

    assert!(validate_surface_identifier(valid).is_ok());
    assert!(is_valid_surface_identifier(valid));
    assert!(SurfaceId::new(valid).is_ok());
    assert!(InteractionId::new("action.submit").is_ok());
    assert!(DataSourceId::new("data.latest").is_ok());
}

#[test]
fn ids_reject_invalid_values() {
    assert!(validate_surface_identifier("").is_err());
    assert!(validate_surface_identifier("1starts.with.digit").is_err());
    assert!(validate_surface_identifier("Uppercase.not.allowed").is_err());
    assert!(validate_surface_identifier("space not allowed").is_err());
}

#[test]
fn slots_registry_exposes_known_slots() {
    let all = all_surface_slots();
    assert!(all.iter().any(|def| def.id == SLOT_SETTINGS_TABS));
    assert!(all.iter().any(|def| def.id == SLOT_SURFACE_PAGE));
    assert!(all.iter().any(|def| def.id == SLOT_HOST_DETAIL_TABS));

    let settings_tabs = slot_def(SLOT_SETTINGS_TABS).expect("known slot");
    assert!(settings_tabs.multi_entry);

    let surface_page = slot_def(SLOT_SURFACE_PAGE).expect("known slot");
    assert!(!surface_page.multi_entry);

    let host_detail_tabs = slot_def(SLOT_HOST_DETAIL_TABS).expect("known slot");
    assert!(host_detail_tabs.multi_entry);
}

#[test]
fn ids_controller_and_builtin_operation_identifiers_follow_lexical_rules() {
    assert!(ControllerQueryId::new("controller.hosts.list").is_ok());
    assert!(BuiltInApiOperationId::new("settings.users.create").is_ok());

    assert!(ControllerQueryId::new("Controller.hosts.list").is_err());
    assert!(BuiltInApiOperationId::new("settings/users/create").is_err());
}
