// Minimal fixture action_type.rs — only Event actions so emit_sweep passes.
struct RegisteredAuditAction;
enum AuditActionKind {
    Stateful,
    Event,
}
impl RegisteredAuditAction {
    pub const fn new(_v: &'static str, _k: AuditActionKind) -> Self {
        Self
    }
}
struct AuditActionType;
impl AuditActionType {
    pub const TEST_VIEW: RegisteredAuditAction =
        RegisteredAuditAction::new("test.view", AuditActionKind::Event);
}
