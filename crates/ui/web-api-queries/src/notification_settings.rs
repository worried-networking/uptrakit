use sea_orm::DatabaseConnection;
use uuid::Uuid;

use uptrakit_plugin_infrastructure_registry::EmailSmtpSettings;

const SMTP_PREFIX: &str = "smtp.";
const GLOBAL_SMTP_PREFIX: &str = "global_smtp.";
const GLOBAL_TELEGRAM_PREFIX: &str = "global_telegram.";
const SMTP_PASSWORD_AAD: &str = "uptrakit:settings:smtp_password";
const GLOBAL_SMTP_PASSWORD_AAD: &str = "uptrakit:settings:global_smtp_password";

pub async fn build_settings_bag(db: &DatabaseConnection, tenant_id: Uuid) -> serde_json::Value {
    let tenant_smtp = typed_smtp_settings_or_empty(
        {
            let raw = uptrakit_shared_db::raw_settings::load_settings_by_prefix(
                db,
                tenant_id,
                SMTP_PREFIX,
            )
            .await;
            raw.and_then(|r| {
                uptrakit_shared_db::raw_settings::decode_prefixed_settings(SMTP_PREFIX, &r)
            })
        },
        "tenant",
        Some(tenant_id),
        SMTP_PASSWORD_AAD,
    );

    let global_smtp = typed_smtp_settings_or_empty(
        {
            let raw = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
                db,
                GLOBAL_SMTP_PREFIX,
            )
            .await;
            raw.and_then(|r| {
                uptrakit_shared_db::raw_settings::decode_prefixed_settings(GLOBAL_SMTP_PREFIX, &r)
            })
        },
        "global",
        None,
        GLOBAL_SMTP_PASSWORD_AAD,
    );

    let global_telegram = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
        db,
        GLOBAL_TELEGRAM_PREFIX,
    )
    .await
    .unwrap_or_default();

    let mut global = smtp_settings_to_prefixed_map(GLOBAL_SMTP_PREFIX, &global_smtp);
    for (k, v) in &global_telegram {
        global.insert(k.clone(), v.clone());
    }

    let tenant_map = smtp_settings_to_prefixed_map(SMTP_PREFIX, &tenant_smtp);

    serde_json::json!({ "tenant": tenant_map, "global": global })
}

fn typed_smtp_settings_or_empty(
    result: uptrakit_shared_db::raw_settings::Result<EmailSmtpSettings>,
    scope: &'static str,
    tenant_id: Option<Uuid>,
    password_aad: &str,
) -> EmailSmtpSettings {
    match result {
        Ok(settings) => normalize_smtp_settings(settings, password_aad, scope, tenant_id),
        Err(error) => {
            if let Some(tenant_id) = tenant_id {
                tracing::warn!(
                    error = ?error,
                    %tenant_id,
                    scope,
                    "failed to load typed SMTP settings for notification dispatch; using empty fallback"
                );
            } else {
                tracing::warn!(
                    error = ?error,
                    scope,
                    "failed to load typed SMTP settings for notification dispatch; using empty fallback"
                );
            }
            EmailSmtpSettings::default()
        }
    }
}

fn normalize_smtp_settings(
    settings: EmailSmtpSettings,
    password_aad: &str,
    scope: &'static str,
    tenant_id: Option<Uuid>,
) -> EmailSmtpSettings {
    EmailSmtpSettings {
        host: normalize_non_empty_string(settings.host),
        port: settings.port,
        username: normalize_non_empty_string(settings.username),
        password: decode_smtp_password(settings.password, password_aad, scope, tenant_id),
        from_address: normalize_non_empty_string(settings.from_address),
        from_name: normalize_non_empty_string(settings.from_name),
        tls_mode: normalize_non_empty_string(settings.tls_mode),
        helo_host: normalize_non_empty_string(settings.helo_host),
    }
}

fn normalize_non_empty_string(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn decode_smtp_password(
    value: Option<String>,
    aad: &str,
    scope: &'static str,
    tenant_id: Option<Uuid>,
) -> Option<String> {
    let raw = normalize_non_empty_string(value)?;

    if uptrakit_crypto::is_encrypted(&raw) {
        return match uptrakit_crypto::decrypt_str(&raw, aad) {
            Ok(value) => normalize_non_empty_string(Some(value)),
            Err(error) => {
                if let Some(tenant_id) = tenant_id {
                    tracing::warn!(
                        error = ?error,
                        %tenant_id,
                        scope,
                        "failed to decrypt SMTP password for notification dispatch; using empty fallback"
                    );
                } else {
                    tracing::warn!(
                        error = ?error,
                        scope,
                        "failed to decrypt SMTP password for notification dispatch; using empty fallback"
                    );
                }
                None
            }
        };
    }

    Some(raw)
}

fn smtp_settings_to_prefixed_map(
    prefix: &str,
    settings: &EmailSmtpSettings,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();

    insert_prefixed_string(&mut map, prefix, "host", settings.host.as_deref());
    insert_prefixed_u16(&mut map, prefix, "port", settings.port);
    insert_prefixed_string(&mut map, prefix, "username", settings.username.as_deref());
    insert_prefixed_string(&mut map, prefix, "password", settings.password.as_deref());
    insert_prefixed_string(
        &mut map,
        prefix,
        "from_address",
        settings.from_address.as_deref(),
    );
    insert_prefixed_string(&mut map, prefix, "from_name", settings.from_name.as_deref());
    insert_prefixed_string(&mut map, prefix, "tls_mode", settings.tls_mode.as_deref());
    insert_prefixed_string(&mut map, prefix, "helo_host", settings.helo_host.as_deref());

    map
}

fn insert_prefixed_string(
    map: &mut serde_json::Map<String, serde_json::Value>,
    prefix: &str,
    suffix: &str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        map.insert(
            format!("{prefix}{suffix}"),
            serde_json::Value::String(value.to_string()),
        );
    }
}

fn insert_prefixed_u16(
    map: &mut serde_json::Map<String, serde_json::Value>,
    prefix: &str,
    suffix: &str,
    value: Option<u16>,
) {
    if let Some(value) = value {
        map.insert(format!("{prefix}{suffix}"), serde_json::json!(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_smtp_settings_or_empty_returns_default_on_load_error() {
        let tenant_id = Uuid::now_v7();
        let settings = typed_smtp_settings_or_empty(
            Err(rootcause::report!(
                uptrakit_shared_db::raw_settings::RawSettingsError::Decode("boom".into())
            )),
            "tenant",
            Some(tenant_id),
            SMTP_PASSWORD_AAD,
        );

        assert_eq!(settings, EmailSmtpSettings::default());
    }
}
