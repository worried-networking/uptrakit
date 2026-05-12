use uptrakit_audit_log::{AuditEntry, AuditView};

struct Demo;
impl AuditView for Demo {
    const TARGET_TYPE: &'static str = "demo";
    fn audit_target_id(&self) -> String {
        "x".into()
    }
    fn audit_target_display(&self) -> Option<String> {
        None
    }
    fn audit_view(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

fn main() {
    // Stateful builder needs both .before() and .after() before .build() is callable.
    let partial = AuditEntry::<uptrakit_audit_log::Stateful>::builder_stateful(
        uptrakit_audit_log::AuditActionType::from(
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
        ),
    )
    .before(&Demo);
    let _ = partial.build();
}
