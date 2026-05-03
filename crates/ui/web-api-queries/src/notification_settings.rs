use sea_orm::DatabaseConnection;
use uuid::Uuid;

const SMTP_PREFIX: &str = "smtp.";
const GLOBAL_SMTP_PREFIX: &str = "global_smtp.";
const GLOBAL_TELEGRAM_PREFIX: &str = "global_telegram.";
const SMTP_PASSWORD_AAD: &str = "uptrakit:settings:smtp_password";
const GLOBAL_SMTP_PASSWORD_AAD: &str = "uptrakit:settings:global_smtp_password";

/// Build the settings bag consumed by notification plugin `deliver()` calls.
///
/// Returns `{ "tenant": { "smtp.*" -> val, ... }, "global": { ... } }`.
pub async fn build_settings_bag(db: &DatabaseConnection, tenant_id: Uuid) -> serde_json::Value {
    let tenant_map = load_smtp_map(db, tenant_id, SMTP_PREFIX, SMTP_PASSWORD_AAD).await;
    let mut global_map =
        load_global_smtp_map(db, GLOBAL_SMTP_PREFIX, GLOBAL_SMTP_PASSWORD_AAD).await;

    match uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
        db,
        GLOBAL_TELEGRAM_PREFIX,
    )
    .await
    {
        Ok(r) => {
            for (k, v) in r {
                global_map.insert(k, v);
            }
        }
        Err(e) => tracing::warn!(error = ?e, "failed to load global Telegram settings"),
    }

    serde_json::json!({ "tenant": tenant_map, "global": global_map })
}

async fn load_smtp_map(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    prefix: &str,
    password_aad: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let raw = match uptrakit_shared_db::raw_settings::load_settings_by_prefix(db, tenant_id, prefix)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                error = ?e, %tenant_id,
                "failed to load tenant SMTP settings; using empty"
            );
            return serde_json::Map::new();
        }
    };
    smtp_raw_to_json_map(raw, password_aad, "tenant", Some(tenant_id))
}

async fn load_global_smtp_map(
    db: &DatabaseConnection,
    prefix: &str,
    password_aad: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let raw =
        match uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(db, prefix).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = ?e, "failed to load global SMTP settings; using empty");
                return serde_json::Map::new();
            }
        };
    smtp_raw_to_json_map(raw, password_aad, "global", None)
}

fn smtp_raw_to_json_map(
    raw: std::collections::HashMap<String, serde_json::Value>,
    password_aad: &str,
    scope: &'static str,
    tenant_id: Option<Uuid>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (k, v) in raw {
        if let serde_json::Value::String(s) = &v
            && s.is_empty()
        {
            continue;
        }
        let value = if k.ends_with(".password") {
            let raw_str = match &v {
                serde_json::Value::String(s) => s.clone(),
                _ => continue,
            };
            decrypt_password_value(raw_str, password_aad, scope, tenant_id)
                .map(serde_json::Value::String)
        } else {
            Some(v)
        };
        if let Some(value) = value {
            map.insert(k, value);
        }
    }
    map
}

fn decrypt_password_value(
    raw: String,
    aad: &str,
    scope: &'static str,
    tenant_id: Option<Uuid>,
) -> Option<String> {
    if !uptrakit_crypto::is_encrypted(&raw) {
        return if raw.is_empty() { None } else { Some(raw) };
    }
    match uptrakit_crypto::decrypt_str(&raw, aad) {
        Ok(v) if v.is_empty() => None,
        Ok(v) => Some(v),
        Err(e) => {
            if let Some(tid) = tenant_id {
                tracing::warn!(
                    error = ?e, %tid, scope,
                    "failed to decrypt SMTP password; using empty"
                );
            } else {
                tracing::warn!(error = ?e, scope, "failed to decrypt SMTP password; using empty");
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypt_password_value_returns_plaintext_for_non_encrypted_values() {
        assert_eq!(
            decrypt_password_value("plain-secret".to_string(), "unused", "tenant", None),
            Some("plain-secret".to_string())
        );
    }

    #[test]
    fn decrypt_password_value_returns_none_for_empty_plaintext() {
        assert_eq!(
            decrypt_password_value(String::new(), "unused", "tenant", None),
            None
        );
    }

    #[test]
    fn smtp_raw_to_json_map_skips_empty_strings() {
        let mut raw = std::collections::HashMap::new();
        raw.insert(
            "smtp.host".to_string(),
            serde_json::Value::String(String::new()),
        );
        raw.insert("smtp.port".to_string(), serde_json::json!(587_u64));
        let map = smtp_raw_to_json_map(raw, "unused", "tenant", None);
        assert!(
            !map.contains_key("smtp.host"),
            "empty host should be skipped"
        );
        assert!(
            map.contains_key("smtp.port"),
            "non-empty port should be kept"
        );
    }
}
