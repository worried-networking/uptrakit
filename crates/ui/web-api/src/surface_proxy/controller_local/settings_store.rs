use async_trait::async_trait;
use sea_orm::EntityTrait;
use uuid::Uuid;

use super::{AppStateSurfaceActionController, plugin_internal_error};
use uptrakit_plugin_infrastructure_registry::{
    EmailSmtpSettings, EmailSmtpSettingsPatch, EmailSmtpSettingsStore, TelegramGlobalSettingsStore,
};

const SMTP_PASSWORD_AAD: &str = "uptrakit:settings:smtp_password";
const GLOBAL_SMTP_PASSWORD_AAD: &str = "uptrakit:settings:global_smtp_password";

const SMTP_PREFIX: &str = "smtp.";
const GLOBAL_SMTP_PREFIX: &str = "global_smtp.";

const KEY_SMTP_HOST: &str = "smtp.host";
const KEY_SMTP_PORT: &str = "smtp.port";
const KEY_SMTP_USERNAME: &str = "smtp.username";
const KEY_SMTP_PASSWORD: &str = "smtp.password";
const KEY_SMTP_FROM_ADDRESS: &str = "smtp.from_address";
const KEY_SMTP_FROM_NAME: &str = "smtp.from_name";
const KEY_SMTP_TLS_MODE: &str = "smtp.tls_mode";
const KEY_SMTP_HELO_HOST: &str = "smtp.helo_host";

const KEY_GLOBAL_SMTP_HOST: &str = "global_smtp.host";
const KEY_GLOBAL_SMTP_PORT: &str = "global_smtp.port";
const KEY_GLOBAL_SMTP_USERNAME: &str = "global_smtp.username";
const KEY_GLOBAL_SMTP_PASSWORD: &str = "global_smtp.password";
const KEY_GLOBAL_SMTP_FROM_ADDRESS: &str = "global_smtp.from_address";
const KEY_GLOBAL_SMTP_FROM_NAME: &str = "global_smtp.from_name";
const KEY_GLOBAL_SMTP_TLS_MODE: &str = "global_smtp.tls_mode";
const KEY_GLOBAL_SMTP_HELO_HOST: &str = "global_smtp.helo_host";
const GLOBAL_TELEGRAM_PREFIX: &str = "global_telegram.";
const KEY_GLOBAL_TELEGRAM_BOT_TOKEN: &str = "global_telegram.bot_token";

#[async_trait]
impl EmailSmtpSettingsStore for AppStateSurfaceActionController<'_> {
    async fn load_tenant_smtp_settings(
        &self,
        tenant_id: Uuid,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<EmailSmtpSettings> {
        let settings = uptrakit_web_api_auth::settings_store::load_typed_settings_by_prefix(
            self.db(),
            tenant_id,
            SMTP_PREFIX,
        )
        .await
        .map_err(plugin_internal_error)?;
        Ok(normalize_smtp_settings(
            settings,
            SMTP_PASSWORD_AAD,
            "tenant",
            Some(tenant_id),
        ))
    }

    async fn save_tenant_smtp_settings(
        &self,
        tenant_id: Uuid,
        patch: EmailSmtpSettingsPatch,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<EmailSmtpSettings> {
        apply_tenant_smtp_patch(self.db(), tenant_id, patch).await?;
        self.load_tenant_smtp_settings(tenant_id).await
    }

    async fn load_global_smtp_settings(
        &self,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<EmailSmtpSettings> {
        let settings = uptrakit_web_api_auth::settings_store::load_typed_global_settings_by_prefix(
            self.db(),
            GLOBAL_SMTP_PREFIX,
        )
        .await
        .map_err(plugin_internal_error)?;
        Ok(normalize_smtp_settings(
            settings,
            GLOBAL_SMTP_PASSWORD_AAD,
            "global",
            None,
        ))
    }

    async fn save_global_smtp_settings(
        &self,
        patch: EmailSmtpSettingsPatch,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<EmailSmtpSettings> {
        apply_global_smtp_patch(self.db(), patch).await?;
        self.load_global_smtp_settings().await
    }

    async fn load_user_email(
        &self,
        user_id: Uuid,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<Option<String>> {
        let model = uptrakit_shared_db::entity::prelude::User::find_by_id(user_id)
            .one(self.db())
            .await
            .map_err(plugin_internal_error)?;
        Ok(model.map(|user| user.email.expose_email().to_string()))
    }
}

#[async_trait]
impl TelegramGlobalSettingsStore for AppStateSurfaceActionController<'_> {
    async fn load_global_bot_token(
        &self,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<String> {
        let map = uptrakit_web_api_auth::settings_store::load_global_settings_by_prefix(
            self.db(),
            GLOBAL_TELEGRAM_PREFIX,
        )
        .await
        .map_err(plugin_internal_error)?;

        Ok(map
            .get(KEY_GLOBAL_TELEGRAM_BOT_TOKEN)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    async fn save_global_bot_token(
        &self,
        bot_token: String,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<String> {
        uptrakit_web_api_auth::settings_store::upsert_global_setting_raw(
            self.db(),
            KEY_GLOBAL_TELEGRAM_BOT_TOKEN,
            serde_json::json!(bot_token),
        )
        .await
        .map_err(plugin_internal_error)?;
        self.load_global_bot_token().await
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
                        "failed to decrypt SMTP password while loading typed SMTP settings; using empty fallback"
                    );
                } else {
                    tracing::warn!(
                        error = ?error,
                        scope,
                        "failed to decrypt SMTP password while loading typed SMTP settings; using empty fallback"
                    );
                }
                None
            }
        };
    }

    Some(raw)
}

async fn apply_tenant_smtp_patch(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    patch: EmailSmtpSettingsPatch,
) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
    if let Some(value) = patch.host {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_HOST,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.port {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_PORT,
            value.map(|port| serde_json::json!(port)),
        )
        .await?;
    }
    if let Some(value) = patch.username {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_USERNAME,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.password {
        let password_value = match value {
            Some(password) => Some(serde_json::Value::String(
                uptrakit_crypto::encrypt_str(password.as_str(), SMTP_PASSWORD_AAD)
                    .map_err(plugin_internal_error)?,
            )),
            None => None,
        };
        upsert_tenant_setting_raw(db, tenant_id, KEY_SMTP_PASSWORD, password_value).await?;
    }
    if let Some(value) = patch.from_address {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_FROM_ADDRESS,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.from_name {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_FROM_NAME,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.tls_mode {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_TLS_MODE,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.helo_host {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_HELO_HOST,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    Ok(())
}

async fn apply_global_smtp_patch(
    db: &sea_orm::DatabaseConnection,
    patch: EmailSmtpSettingsPatch,
) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
    if let Some(value) = patch.host {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_HOST,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.port {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_PORT,
            value.map(|port| serde_json::json!(port)),
        )
        .await?;
    }
    if let Some(value) = patch.username {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_USERNAME,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.password {
        let password_value = match value {
            Some(password) => Some(serde_json::Value::String(
                uptrakit_crypto::encrypt_str(password.as_str(), GLOBAL_SMTP_PASSWORD_AAD)
                    .map_err(plugin_internal_error)?,
            )),
            None => None,
        };
        upsert_global_setting_raw(db, KEY_GLOBAL_SMTP_PASSWORD, password_value).await?;
    }
    if let Some(value) = patch.from_address {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_FROM_ADDRESS,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.from_name {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_FROM_NAME,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.tls_mode {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_TLS_MODE,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.helo_host {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_HELO_HOST,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    Ok(())
}

async fn upsert_tenant_setting_raw(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    key: &str,
    value: Option<serde_json::Value>,
) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
    let payload = value.unwrap_or(serde_json::Value::Null);
    uptrakit_web_api_auth::settings_store::upsert_setting_raw(db, tenant_id, key, payload)
        .await
        .map_err(plugin_internal_error)
}

async fn upsert_global_setting_raw(
    db: &sea_orm::DatabaseConnection,
    key: &str,
    value: Option<serde_json::Value>,
) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
    let payload = value.unwrap_or(serde_json::Value::Null);
    uptrakit_web_api_auth::settings_store::upsert_global_setting_raw(db, key, payload)
        .await
        .map_err(plugin_internal_error)
}

#[cfg(test)]
mod tests {
    use super::{decode_smtp_password, normalize_smtp_settings};
    use uptrakit_plugin_infrastructure_registry::EmailSmtpSettings;

    #[test]
    fn normalize_smtp_settings_converts_empty_strings_to_none() {
        let normalized = normalize_smtp_settings(
            EmailSmtpSettings {
                host: Some(String::new()),
                port: Some(587),
                username: Some(String::new()),
                password: Some(String::new()),
                from_address: Some(String::new()),
                from_name: Some(String::new()),
                tls_mode: Some(String::new()),
                helo_host: Some(String::new()),
            },
            "unused",
            "tenant",
            None,
        );

        assert_eq!(
            normalized,
            EmailSmtpSettings {
                host: None,
                port: Some(587),
                username: None,
                password: None,
                from_address: None,
                from_name: None,
                tls_mode: None,
                helo_host: None,
            }
        );
    }

    #[test]
    fn decode_smtp_password_returns_plaintext_for_non_encrypted_values() {
        assert_eq!(
            decode_smtp_password(Some("plain-secret".to_string()), "unused", "tenant", None),
            Some("plain-secret".to_string())
        );
    }
}
