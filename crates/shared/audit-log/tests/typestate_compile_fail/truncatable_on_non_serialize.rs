use uptrakit_audit_log::AuditView;

/// A field type without Serialize.
struct NotSerialize;

#[derive(uptrakit_audit_log::AuditView)]
#[audit(target_type = "demo")]
struct Demo {
    id: uuid::Uuid,
    #[audit(truncatable)]
    blob: NotSerialize,
}

fn main() {}
