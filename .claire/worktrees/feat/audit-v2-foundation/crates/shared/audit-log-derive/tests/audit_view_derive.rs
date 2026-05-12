use uptrakit_audit_log::AuditView;

#[derive(uptrakit_audit_log::AuditView)]
#[audit(target_type = "demo")]
struct Demo {
    id: uuid::Uuid,
    name: String,
    count: u32,
    #[audit(skip)]
    internal: i64,
    // auto-skipped by name allowlist:
    created_at: time::OffsetDateTime,
    updated_at: time::OffsetDateTime,
}

#[test]
fn derive_projects_only_audit_relevant_fields() {
    let id = uuid::Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    let demo = Demo {
        id,
        name: "alpha".into(),
        count: 3,
        internal: 99,
        created_at: now,
        updated_at: now,
    };

    assert_eq!(<Demo as AuditView>::TARGET_TYPE, "demo");
    assert_eq!(demo.audit_target_id(), id.to_string());
    assert_eq!(demo.audit_target_display(), Some("alpha".into()));
    let v = demo.audit_view();
    let map = v.as_object().expect("object");
    assert!(map.contains_key("name"));
    assert!(map.contains_key("count"));
    assert!(!map.contains_key("internal"));
    assert!(!map.contains_key("created_at"));
    assert!(!map.contains_key("updated_at"));
    assert!(!map.contains_key("id"));
}

#[test]
fn derive_projection_is_byte_equal_across_invocations() {
    let id = uuid::Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    let demo = Demo {
        id,
        name: "alpha".into(),
        count: 3,
        internal: 99,
        created_at: now,
        updated_at: now,
    };
    let first = serde_json::to_vec(&demo.audit_view()).expect("serialize");
    let second = serde_json::to_vec(&demo.audit_view()).expect("serialize");
    assert_eq!(first, second);
}
