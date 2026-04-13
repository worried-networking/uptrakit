#[test]
fn platform_types_are_constructible() {
    use uptrakit_service_platform::{ServiceKind, YieldPolicy};

    assert_eq!(ServiceKind::Agent as u8, ServiceKind::Agent as u8);
    assert!(matches!(
        YieldPolicy::SameServiceAnywhere,
        YieldPolicy::SameServiceAnywhere
    ));
}
