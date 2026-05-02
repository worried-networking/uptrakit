#![expect(
    clippy::assertions_on_result_states,
    reason = "test assertions — is_ok/is_err provides readable failure messages"
)]

use uptrakit_surfaces::{
    BuiltInApiOperationId, CalloutLevel, Capability, ControllerQueryId, ControllerSseTopicId,
    DataSourceId, DataSourceValidationError, FrameworkGeneration, FrameworkGenerationRange,
    InteractionId, SLOT_HOST_DETAIL_TABS, SLOT_SETTINGS_TABS, SLOT_SOFTWARE_ITEM_TABS,
    SLOT_SURFACE_PAGE, SchemaContract, SurfaceId, SurfaceRowCondition, Targeting,
    all_surface_slots, is_valid_surface_identifier, slot_def, validate_surface_identifier,
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
    assert!(all.iter().any(|def| def.id == SLOT_SOFTWARE_ITEM_TABS));

    let settings_tabs = slot_def(SLOT_SETTINGS_TABS).expect("known slot");
    assert!(settings_tabs.multi_entry);

    let surface_page = slot_def(SLOT_SURFACE_PAGE).expect("known slot");
    assert!(!surface_page.multi_entry);

    let host_detail_tabs = slot_def(SLOT_HOST_DETAIL_TABS).expect("known slot");
    assert!(host_detail_tabs.multi_entry);

    let software_item_tabs = slot_def(SLOT_SOFTWARE_ITEM_TABS).expect("known slot");
    assert!(software_item_tabs.multi_entry);
}

#[test]
fn ids_controller_and_builtin_operation_identifiers_follow_lexical_rules() {
    assert!(ControllerQueryId::new("controller.hosts.list").is_ok());
    assert!(BuiltInApiOperationId::new("settings.users.create").is_ok());

    assert!(ControllerQueryId::new("Controller.hosts.list").is_err());
    assert!(BuiltInApiOperationId::new("settings/users/create").is_err());
}

#[test]
fn generated_identifier_new_accepts_valid_value() {
    let id = SurfaceId::new("dashboard.main").expect("valid identifier");
    assert_eq!(id.as_str(), "dashboard.main");
}

#[test]
fn controller_sse_topic_id_round_trips_as_str() {
    let id = ControllerSseTopicId::new("controller.events")
        .expect("valid controller SSE topic identifier");
    assert_eq!(id.as_str(), "controller.events");
}

#[test]
fn framework_generation_range_includes_is_const_friendly() {
    const RANGE: FrameworkGenerationRange = FrameworkGenerationRange {
        min: FrameworkGeneration::new(1, 0),
        max: FrameworkGeneration::new(2, 5),
    };
    const _: () = {
        assert!(RANGE.includes(FrameworkGeneration::new(1, 0)));
        assert!(!RANGE.includes(FrameworkGeneration::new(3, 0)));
    };
}

#[test]
fn small_enums_are_copy() {
    fn assert_copy<T: Copy>() {}

    assert_copy::<Targeting>();
    assert_copy::<SurfaceRowCondition>();
    assert_copy::<CalloutLevel>();
    assert_copy::<Capability>();
    assert_copy::<SchemaContract>();
    assert_copy::<DataSourceValidationError>();
}
