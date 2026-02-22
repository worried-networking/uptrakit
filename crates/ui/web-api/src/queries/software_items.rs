use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, ModelTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set, TransactionTrait,
};
use time::OffsetDateTime;
use uptrakit_provider_registry::ProviderRegistry;
use uptrakit_shared_db::entity::{host, host_software_item, prelude::*, provider_config, software_item};
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, SoftwareItemDetailResponse,
    SoftwareItemHostSummary, SoftwareItemResponse, UpdateSoftwareItemRequest,
};
use uuid::Uuid;

use crate::auth::token::generate_uuid;
use crate::queries::provider_configs::{
    find_raw_active_config, find_raw_active_config_txn, validate_hooks_internal,
};
use crate::tenant_db::TenantDb;

/// Errors returned by software item mutation queries.
#[derive(Debug)]
pub enum SoftwareItemQueryError {
    /// Software item not found or deactivated.
    NotFound,
    /// Name must not be empty (for update).
    EmptyName,
    /// The referenced provider config does not exist or is inactive.
    ProviderConfigNotFound,
    /// A software item with the same (provider_config_id, package_identifier) already exists.
    DuplicateItem,
    /// A host in the request was not found or is deactivated.
    HostNotFound(Uuid),
    /// Package identifier failed validation (e.g. Homebrew naming rules).
    InvalidPackageIdentifier(String),
    /// `config_override` failed provider-level or hook validation.
    InvalidConfigOverride(String),
    /// Inline provider config failed name/config/hook validation.
    InvalidInlineProviderConfig(String),
    /// A database error occurred.
    Db(sea_orm::DbErr),
}

// --- Private helpers ---

fn build_list_response(
    item: &software_item::Model,
    config: &provider_config::Model,
    host_count: u64,
) -> SoftwareItemResponse {
    SoftwareItemResponse {
        id: item.id,
        name: item.name.clone(),
        provider_config_id: item.provider_config_id,
        provider_config_name: config.name.clone(),
        provider_type: config.provider_type.clone(),
        package_identifier: item.package_identifier.clone(),
        config_override: item.config_override.clone(),
        enabled: item.enabled,
        last_checked_at: item.last_checked_at,
        host_count,
        created_at: item.created_at,
        updated_at: item.updated_at,
    }
}

fn build_detail_response(
    item: software_item::Model,
    config: &provider_config::Model,
    host_count: u64,
    hosts: Vec<SoftwareItemHostSummary>,
) -> SoftwareItemDetailResponse {
    SoftwareItemDetailResponse {
        id: item.id,
        name: item.name,
        provider_config_id: item.provider_config_id,
        provider_config_name: config.name.clone(),
        provider_type: config.provider_type.clone(),
        package_identifier: item.package_identifier,
        config_override: item.config_override,
        enabled: item.enabled,
        last_checked_at: item.last_checked_at,
        host_count,
        created_at: item.created_at,
        updated_at: item.updated_at,
        hosts,
    }
}

async fn count_linked_hosts(db: &sea_orm::DatabaseConnection, item_id: Uuid) -> u64 {
    HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .count(db)
        .await
        .unwrap_or(0)
}

async fn load_item_hosts(
    db: &sea_orm::DatabaseConnection,
    item_id: Uuid,
) -> Vec<SoftwareItemHostSummary> {
    let links = match HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .all(db)
        .await
    {
        Ok(links) => links,
        Err(e) => {
            tracing::warn!("Failed to load software item hosts: {e}");
            return Vec::new();
        }
    };

    let mut summaries = Vec::with_capacity(links.len());
    for link in links {
        if let Ok(Some(h)) = Host::find_by_id(link.host_id)
            .filter(host::Column::DeactivatedAt.is_null())
            .one(db)
            .await
        {
            summaries.push(SoftwareItemHostSummary {
                host_id: h.id,
                hostname: h.hostname,
                friendly_name: h.friendly_name,
                installed_version: link.installed_version,
                installed_version_detected_at: link.installed_version_detected_at,
                last_updated_at: link.last_updated_at,
                linked_at: link.linked_at,
            });
        }
    }

    summaries
}

/// Find a non-deactivated software item by ID, scoped to a tenant.
pub(crate) async fn find_active_item(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    id: Uuid,
) -> Option<software_item::Model> {
    SoftwareItem::find_by_id(id)
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .ok()
        .flatten()
}

/// Error returned when `config_override` validation fails.
#[derive(Debug, thiserror::Error)]
enum ConfigOverrideError {
    #[error("config_override must be a JSON object")]
    NotAnObject,
    #[error("provider validation failed: {0}")]
    ProviderValidation(String),
}

/// Validate `config_override` by merging it with the base provider config and running
/// provider-specific validation. The merged document must satisfy the provider's schema.
fn validate_config_override(
    provider_type: &str,
    base_config: &serde_json::Value,
    override_config: &serde_json::Value,
) -> std::result::Result<(), ConfigOverrideError> {
    let mut merged = base_config.clone();
    if let (Some(base_obj), Some(over_obj)) = (merged.as_object_mut(), override_config.as_object())
    {
        for (k, v) in over_obj {
            base_obj.insert(k.clone(), v.clone());
        }
    } else {
        return Err(ConfigOverrideError::NotAnObject);
    }

    ProviderRegistry::validate_config_str(provider_type, &merged)
        .map_err(|e| ConfigOverrideError::ProviderValidation(e.to_string()))
}

/// Validate a Homebrew package identifier.
///
/// Rejects empty values, leading/trailing whitespace, embedded whitespace, path-traversal
/// segments (`..`, `.`), empty path segments (`//`), and any characters outside the
/// allowed set `[A-Za-z0-9\-_.@+/]`.
fn validate_homebrew_package_identifier(value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("package_identifier must not be empty".to_string());
    }
    if value != trimmed {
        return Err(
            "package_identifier must not include leading or trailing whitespace".to_string(),
        );
    }
    if value.chars().any(char::is_whitespace) {
        return Err("package_identifier must not contain whitespace".to_string());
    }
    if value.len() > 200 {
        return Err("package_identifier is too long".to_string());
    }

    for ch in value.chars() {
        let valid = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '@' | '+' | '/');
        if !valid {
            return Err(format!(
                "package_identifier contains invalid character: {ch}"
            ));
        }
    }

    if value.split('/').any(|segment| segment.is_empty()) {
        return Err("package_identifier contains an empty segment".to_string());
    }

    if value
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err("package_identifier contains invalid segment".to_string());
    }

    Ok(())
}

// --- Public query functions ---

/// Create a new software item, validating the request (inline provider config, package
/// identifier, and config_override) before persisting.
pub async fn create_software_item(
    tenant_db: &TenantDb,
    req: CreateSoftwareItemRequest,
) -> Result<SoftwareItemResponse, SoftwareItemQueryError> {
    let txn = tenant_db
        .db()
        .begin()
        .await
        .map_err(SoftwareItemQueryError::Db)?;

    let (provider_config_id, config) =
        match (req.provider_config_id, req.provider_config.as_ref()) {
            (Some(pcid), None) => {
                let c = find_raw_active_config_txn(&txn, tenant_db.tenant_id, pcid)
                    .await
                    .ok_or(SoftwareItemQueryError::ProviderConfigNotFound)?;
                (pcid, c)
            }
            (None, Some(inline)) => {
                if inline.name.is_empty() {
                    return Err(SoftwareItemQueryError::InvalidInlineProviderConfig(
                        "name must not be empty".to_string(),
                    ));
                }
                if let Err(e) =
                    ProviderRegistry::validate_config_str(inline.provider_type.as_str(), &inline.config)
                {
                    return Err(SoftwareItemQueryError::InvalidInlineProviderConfig(
                        e.to_string(),
                    ));
                }
                if let Err(e) = validate_hooks_internal(&inline.config) {
                    return Err(SoftwareItemQueryError::InvalidInlineProviderConfig(
                        e.to_string(),
                    ));
                }
                let now = OffsetDateTime::now_utc();
                let pcid = generate_uuid();
                let model = provider_config::ActiveModel {
                    id: Set(pcid),
                    tenant_id: Set(tenant_db.tenant_id),
                    name: Set(inline.name.clone()),
                    provider_type: Set(inline.provider_type.to_string()),
                    config: Set(inline.config.clone()),
                    enabled: Set(inline.enabled),
                    created_at: Set(now),
                    updated_at: Set(now),
                    deactivated_at: Set(None),
                };
                let inserted = model.insert(&txn).await.map_err(SoftwareItemQueryError::Db)?;
                (pcid, inserted)
            }
            _ => {
                return Err(SoftwareItemQueryError::ProviderConfigNotFound);
            }
        };

    let package_identifier = req.package_identifier.unwrap_or_default();

    if config.provider_type == "homebrew"
        && let Err(e) = validate_homebrew_package_identifier(&package_identifier)
    {
        return Err(SoftwareItemQueryError::InvalidPackageIdentifier(e));
    }

    if let Some(ref override_val) = req.config_override {
        if let Err(e) = validate_config_override(&config.provider_type, &config.config, override_val)
        {
            return Err(SoftwareItemQueryError::InvalidConfigOverride(e.to_string()));
        }
        if let Err(e) = validate_hooks_internal(override_val) {
            return Err(SoftwareItemQueryError::InvalidConfigOverride(e.to_string()));
        }
    }

    // Check uniqueness: (provider_config_id, package_identifier) among active items.
    let duplicate = SoftwareItem::find()
        .filter(software_item::Column::ProviderConfigId.eq(provider_config_id))
        .filter(software_item::Column::PackageIdentifier.eq(&package_identifier))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(&txn)
        .await
        .map_err(SoftwareItemQueryError::Db)?;

    if duplicate.is_some() {
        return Err(SoftwareItemQueryError::DuplicateItem);
    }

    let now = OffsetDateTime::now_utc();
    let model = software_item::ActiveModel {
        id: Set(generate_uuid()),
        tenant_id: Set(tenant_db.tenant_id),
        name: Set(req.name),
        provider_config_id: Set(provider_config_id),
        package_identifier: Set(package_identifier),
        config_override: Set(req.config_override),
        enabled: Set(req.enabled),
        last_checked_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    let inserted = model.insert(&txn).await.map_err(SoftwareItemQueryError::Db)?;

    txn.commit().await.map_err(SoftwareItemQueryError::Db)?;

    Ok(build_list_response(&inserted, &config, 0))
}

pub async fn list_software_items(
    tenant_db: &TenantDb,
    params: &PaginationParams,
) -> Result<PaginatedResponse<SoftwareItemResponse>, sea_orm::DbErr> {
    let pagination = params.resolve();

    let base_query = SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .order_by_asc(software_item::Column::Name);

    let total = base_query.clone().count(tenant_db.db()).await?;

    let items = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await?;

    let mut response = Vec::with_capacity(items.len());
    for item in items {
        let config = match find_raw_active_config(tenant_db, item.provider_config_id).await {
            Some(c) => c,
            None => {
                tracing::warn!(
                    "Software item {} references missing provider config {}",
                    item.id,
                    item.provider_config_id
                );
                continue;
            }
        };
        let host_count = count_linked_hosts(tenant_db.db(), item.id).await;
        response.push(build_list_response(&item, &config, host_count));
    }

    Ok(PaginatedResponse::new(response, total, pagination))
}

/// Returns `None` if not found.
pub async fn get_software_item(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<SoftwareItemDetailResponse>, sea_orm::DbErr> {
    let Some(item) = find_active_item(tenant_db.db(), tenant_db.tenant_id, id).await else {
        return Ok(None);
    };

    let Some(config) = find_raw_active_config(tenant_db, item.provider_config_id).await else {
        tracing::error!(
            "Software item {} references missing provider config {}",
            item.id,
            item.provider_config_id
        );
        return Err(sea_orm::DbErr::Custom(
            "referenced provider config not found".to_string(),
        ));
    };

    let hosts = load_item_hosts(tenant_db.db(), id).await;
    let host_count = hosts.len() as u64;
    Ok(Some(build_detail_response(item, &config, host_count, hosts)))
}

/// Partial update, validating name, package identifier, and config_override.
/// Returns `Err(NotFound)` if the item does not exist or is deactivated.
pub async fn update_software_item(
    tenant_db: &TenantDb,
    id: Uuid,
    req: UpdateSoftwareItemRequest,
) -> Result<SoftwareItemResponse, SoftwareItemQueryError> {
    let existing = find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or(SoftwareItemQueryError::NotFound)?;

    let config = find_raw_active_config(tenant_db, existing.provider_config_id)
        .await
        .ok_or(SoftwareItemQueryError::ProviderConfigNotFound)?;

    if let Some(ref name) = req.name
        && name.is_empty()
    {
        return Err(SoftwareItemQueryError::EmptyName);
    }

    if let Some(ref new_pid) = req.package_identifier
        && config.provider_type == "homebrew"
        && let Err(e) = validate_homebrew_package_identifier(new_pid)
    {
        return Err(SoftwareItemQueryError::InvalidPackageIdentifier(e));
    }

    let new_package_id = req
        .package_identifier
        .as_deref()
        .unwrap_or(&existing.package_identifier);

    if new_package_id != existing.package_identifier {
        let duplicate = SoftwareItem::find()
            .filter(software_item::Column::ProviderConfigId.eq(existing.provider_config_id))
            .filter(software_item::Column::PackageIdentifier.eq(new_package_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .filter(software_item::Column::Id.ne(id))
            .one(tenant_db.db())
            .await
            .map_err(SoftwareItemQueryError::Db)?;

        if duplicate.is_some() {
            return Err(SoftwareItemQueryError::DuplicateItem);
        }
    }

    if let Some(ref override_val) = req.config_override
        && !override_val.is_null()
    {
        if let Err(e) =
            validate_config_override(&config.provider_type, &config.config, override_val)
        {
            return Err(SoftwareItemQueryError::InvalidConfigOverride(e.to_string()));
        }
        if let Err(e) = validate_hooks_internal(override_val) {
            return Err(SoftwareItemQueryError::InvalidConfigOverride(e.to_string()));
        }
    }

    let now = OffsetDateTime::now_utc();
    let mut model: software_item::ActiveModel = existing.into();

    if let Some(name) = req.name {
        model.name = Set(name);
    }
    if let Some(package_identifier) = req.package_identifier {
        model.package_identifier = Set(package_identifier);
    }
    if let Some(config_override) = req.config_override {
        if config_override.is_null() {
            model.config_override = Set(None);
        } else {
            model.config_override = Set(Some(config_override));
        }
    }
    if let Some(enabled) = req.enabled {
        model.enabled = Set(enabled);
    }
    model.updated_at = Set(now);

    let updated = model.update(tenant_db.db()).await.map_err(SoftwareItemQueryError::Db)?;
    let host_count = count_linked_hosts(tenant_db.db(), id).await;
    Ok(build_list_response(&updated, &config, host_count))
}

/// Soft-delete a software item. Returns `true` if deleted, `false` if not found.
pub async fn delete_software_item(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<bool, sea_orm::DbErr> {
    let Some(item) = find_active_item(tenant_db.db(), tenant_db.tenant_id, id).await else {
        return Ok(false);
    };

    let now = OffsetDateTime::now_utc();
    let mut model: software_item::ActiveModel = item.into();
    model.deactivated_at = Set(Some(now));
    model.enabled = Set(false);
    model.updated_at = Set(now);
    model.update(tenant_db.db()).await?;
    Ok(true)
}

/// Assign hosts to a software item.
/// Returns the updated detail response, or an error if the item or a host is not found.
pub async fn assign_hosts(
    tenant_db: &TenantDb,
    id: Uuid,
    req: AssignHostsRequest,
) -> Result<SoftwareItemDetailResponse, SoftwareItemQueryError> {
    let item = find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or(SoftwareItemQueryError::NotFound)?;

    let now = OffsetDateTime::now_utc();

    for &host_id in &req.host_ids {
        let host_exists = Host::find_by_id(host_id)
            .filter(host::Column::DeactivatedAt.is_null())
            .one(tenant_db.db())
            .await
            .map_err(SoftwareItemQueryError::Db)?;

        if host_exists.is_none() {
            return Err(SoftwareItemQueryError::HostNotFound(host_id));
        }

        let existing_link = HostSoftwareItem::find_by_id((host_id, id))
            .one(tenant_db.db())
            .await
            .map_err(SoftwareItemQueryError::Db)?;

        if existing_link.is_none() {
            let link = host_software_item::ActiveModel {
                host_id: Set(host_id),
                software_item_id: Set(id),
                installed_version: Set(None),
                installed_version_detected_at: Set(None),
                last_updated_at: Set(None),
                linked_at: Set(now),
            };
            link.insert(tenant_db.db())
                .await
                .map_err(SoftwareItemQueryError::Db)?;
        }
    }

    let config = find_raw_active_config(tenant_db, item.provider_config_id)
        .await
        .ok_or(SoftwareItemQueryError::ProviderConfigNotFound)?;

    let hosts = load_item_hosts(tenant_db.db(), id).await;
    let host_count = hosts.len() as u64;
    Ok(build_detail_response(item, &config, host_count, hosts))
}

/// Unassign a host from a software item.
/// Returns `true` if removed, `false` if the software item or link was not found.
pub async fn unassign_host(
    tenant_db: &TenantDb,
    id: Uuid,
    host_id: Uuid,
) -> Result<bool, sea_orm::DbErr> {
    if find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .is_none()
    {
        return Ok(false);
    }

    let link = HostSoftwareItem::find_by_id((host_id, id))
        .one(tenant_db.db())
        .await?;

    match link {
        Some(l) => {
            l.delete(tenant_db.db()).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    #[test]
    fn build_list_response_formats_timestamps() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Node.js".to_string(),
            provider_config_id: uuid::Uuid::now_v7(),
            package_identifier: String::new(),
            config_override: None,
            enabled: true,
            last_checked_at: Some(now),
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };
        let config = provider_config::Model {
            id: item.provider_config_id,
            tenant_id: uuid::Uuid::nil(),
            name: "My GitHub Config".to_string(),
            provider_type: "github_releases".to_string(),
            config: serde_json::json!({}),
            enabled: true,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(&item, &config, 3);

        assert_eq!(resp.name, "Node.js");
        assert_eq!(resp.provider_config_name, "My GitHub Config");
        assert_eq!(resp.provider_type, "github_releases");
        assert_eq!(resp.host_count, 3);
        assert!(resp.last_checked_at.is_some());
        assert!(resp.config_override.is_none());
    }

    #[test]
    fn build_detail_response_includes_hosts() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Redis".to_string(),
            provider_config_id: uuid::Uuid::now_v7(),
            package_identifier: "redis-server".to_string(),
            config_override: Some(serde_json::json!({"asset_patterns": ["redis.*linux"]})),
            enabled: true,
            last_checked_at: None,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };
        let config = provider_config::Model {
            id: item.provider_config_id,
            tenant_id: uuid::Uuid::nil(),
            name: "Redis GitHub".to_string(),
            provider_type: "github_releases".to_string(),
            config: serde_json::json!({}),
            enabled: true,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };
        let hosts = vec![SoftwareItemHostSummary {
            host_id: uuid::Uuid::now_v7(),
            hostname: "web-01".to_string(),
            friendly_name: "Web Server 1".to_string(),
            installed_version: Some("7.2.4".to_string()),
            installed_version_detected_at: Some(now),
            last_updated_at: None,
            linked_at: now,
        }];

        let resp = build_detail_response(item, &config, 1, hosts);

        assert_eq!(resp.name, "Redis");
        assert_eq!(resp.package_identifier, "redis-server");
        assert!(resp.config_override.is_some());
        assert_eq!(resp.hosts.len(), 1);
        assert_eq!(resp.hosts[0].hostname, "web-01");
        assert_eq!(resp.hosts[0].installed_version, Some("7.2.4".to_string()));
    }

    #[test]
    fn build_list_response_null_last_checked_at() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Nginx".to_string(),
            provider_config_id: uuid::Uuid::now_v7(),
            package_identifier: String::new(),
            config_override: None,
            enabled: false,
            last_checked_at: None,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };
        let config = provider_config::Model {
            id: item.provider_config_id,
            tenant_id: uuid::Uuid::nil(),
            name: "Config".to_string(),
            provider_type: "github_releases".to_string(),
            config: serde_json::json!({}),
            enabled: true,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(&item, &config, 0);

        assert!(!resp.enabled);
        assert!(resp.last_checked_at.is_none());
        assert_eq!(resp.host_count, 0);
    }

    #[test]
    fn validate_config_override_valid_merge() {
        let base = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let override_val = serde_json::json!({
            "tag_strip_prefix": "release-"
        });

        let result = validate_config_override("github_releases", &base, &override_val);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_config_override_invalid_merge() {
        let base = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        // Override that clears a required field.
        let override_val = serde_json::json!({
            "owner": ""
        });

        let result = validate_config_override("github_releases", &base, &override_val);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_override_non_object_rejected() {
        let base = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let override_val = serde_json::json!("not an object");

        let result = validate_config_override("github_releases", &base, &override_val);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigOverrideError::NotAnObject
        ));
    }

    #[test]
    fn validate_homebrew_package_identifier_accepts_valid() {
        let cases = [
            "wget",
            "node@18",
            "homebrew/cask/firefox",
            "custom-tap/tool",
            "pkg.name",
            "pkg_name",
            "pkg+name",
        ];

        for case in cases {
            assert!(
                validate_homebrew_package_identifier(case).is_ok(),
                "expected valid: {case}"
            );
        }
    }

    #[test]
    fn validate_homebrew_package_identifier_rejects_invalid() {
        let cases = [
            "",
            " ",
            " leading",
            "trailing ",
            "has space",
            "tap//pkg",
            "tap/../pkg",
            "tap/./pkg",
            "pkg$",
        ];

        for case in cases {
            assert!(
                validate_homebrew_package_identifier(case).is_err(),
                "expected invalid: {case}"
            );
        }
    }
}
