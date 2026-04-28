// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::surfaces::{
    ControllerQueryId, DataSourceId, IdentifierError, ProviderKind, validate_surface_identifier,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
pub const MIN_PROVIDER_REFRESH_INTERVAL_SECONDS: u32 = 10;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DataSourceKind {
    Static { data: Value },
    ControllerQuery { query_id: ControllerQueryId },
    ProviderQuery { operation_id: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum RefreshPolicy {
    Manual,
    Interval { seconds: u32 },
    Sse { topic: ControllerSseTopicId },
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ControllerSseTopicId(String);
impl ControllerSseTopicId {
    /// Constructs a validated SSE topic identifier.
    ///
    /// # Errors
    /// Returns any [`IdentifierError`] from
    /// [`validate_surface_identifier`] when `value` is not a valid
    /// identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_surface_identifier(&value)?;
        Ok(Self(value))
    }
    #[must_use]
    pub const fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
impl<'de> Deserialize<'de> for ControllerSseTopicId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SchemaContract {
    Any,
    Object,
    Array,
    String,
    Integer,
    Number,
    Boolean,
    Null,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSourceDescriptor {
    pub data_source_id: DataSourceId,
    pub kind: DataSourceKind,
    pub result_schema: SchemaContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<DataSourcePagination>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sorting: Option<DataSourceSorting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filtering: Option<DataSourceFiltering>,
    pub refresh_policy: RefreshPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_state: Option<DataSourceEmptyState>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSourcePagination {
    pub default_page_size: u16,
    pub max_page_size: u16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSourceSorting {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sortable_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_sort_field: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSourceFiltering {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filter_fields: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSourceEmptyState {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DataSourceValidationError {
    #[error("service providers cannot declare controller_query data sources")]
    ServiceControllerQueryForbidden,
    #[error(
        "provider_query interval refresh must be at least {MIN_PROVIDER_REFRESH_INTERVAL_SECONDS} seconds"
    )]
    ProviderIntervalTooLow,
}
impl DataSourceDescriptor {
    /// Validates provider-specific data source rules.
    ///
    /// # Errors
    /// Returns
    /// [`DataSourceValidationError::ServiceControllerQueryForbidden`]
    /// when a service provider declares a `controller_query` data source.
    /// Returns [`DataSourceValidationError::ProviderIntervalTooLow`] when
    /// a `provider_query` data source uses interval refresh lower than
    /// [`MIN_PROVIDER_REFRESH_INTERVAL_SECONDS`].
    pub fn validate_for_provider(
        &self,
        provider_kind: ProviderKind,
    ) -> Result<(), DataSourceValidationError> {
        if provider_kind == ProviderKind::Service
            && matches!(self.kind, DataSourceKind::ControllerQuery { .. })
        {
            return Err(DataSourceValidationError::ServiceControllerQueryForbidden);
        }
        if matches!(self.kind, DataSourceKind::ProviderQuery { .. })
            && matches!(
                self.refresh_policy, RefreshPolicy::Interval { seconds } if seconds <
                MIN_PROVIDER_REFRESH_INTERVAL_SECONDS
            )
        {
            return Err(DataSourceValidationError::ProviderIntervalTooLow);
        }
        Ok(())
    }
}
