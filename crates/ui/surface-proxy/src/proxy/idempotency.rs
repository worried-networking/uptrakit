use std::hash::{Hash, Hasher};

use uptrakit_wire::surfaces;

use super::{IdempotencyKey, SurfaceInvokeRequest};

pub(super) fn build_idempotency_key(
    request: &SurfaceInvokeRequest,
    caller_origin: &surfaces::CallerOrigin,
) -> IdempotencyKey {
    IdempotencyKey {
        tenant_id: request.tenant_id,
        surface_id: request.surface_id.clone(),
        interaction_id: request.interaction_id.clone(),
        caller_key: match caller_origin {
            surfaces::CallerOrigin::UserSession {
                user_id,
                session_id,
            } => format!("user:{user_id}:{session_id}"),
            surfaces::CallerOrigin::BuiltInSystem { principal } => {
                format!("system:{principal}")
            }
            surfaces::CallerOrigin::Provider { provider_id } => {
                format!("provider:{provider_id}")
            }
        },
        idempotency_key: request.idempotency_key.clone(),
    }
}

pub(super) fn fingerprint_request(
    params: &serde_json::Map<String, serde_json::Value>,
    encrypted: Option<&surfaces::EncryptedSensitiveParams>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::Value::Object(params.clone())
        .to_string()
        .hash(&mut hasher);
    encrypted
        .map(|value| (&value.key_id, &value.ciphertext_b64))
        .hash(&mut hasher);
    hasher.finish()
}
