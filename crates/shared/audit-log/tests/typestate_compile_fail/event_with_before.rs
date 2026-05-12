use uptrakit_audit_log::AuditEntry;

struct Demo;
impl uptrakit_audit_log::AuditView for Demo {
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
    let builder = AuditEntry::auth_login();
    // Event builder has no .before() method.
    let _ = builder.before(&Demo);
}
