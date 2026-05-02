#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: dead_code fires only when not all test feature flags are active"
)]
#[allow(dead_code)]
pub(crate) mod db_providers;
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: dead_code fires only when not all test feature flags are active"
)]
#[allow(dead_code)]
pub(crate) mod fixtures;
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: dead_code fires only when not all test feature flags are active"
)]
#[allow(dead_code)]
pub(crate) mod harness;
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: dead_code fires only when not all test feature flags are active"
)]
#[allow(dead_code)]
pub(crate) mod http_client;
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: dead_code fires only when not all test feature flags are active"
)]
#[allow(dead_code)]
pub(crate) mod macros;
