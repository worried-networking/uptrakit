use uptrakit_audit_log::AuditEntry;

fn main() {
    let event = AuditEntry::auth_login()
        .actor_system()
        .build()
        .expect("event");
    // Stub emit_stateful signature — should not accept AuditEntry<Event>.
    fn emit_stateful(_e: AuditEntry<uptrakit_audit_log::Stateful>) {}
    emit_stateful(event);
}
