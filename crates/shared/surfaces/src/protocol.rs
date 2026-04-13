use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    CapabilitySet, DataSourceDescriptor, FrameworkGeneration, FrameworkGenerationRange,
    InteractionDescriptor, InteractionId, ProviderKind, Scope, SlotValidationError,
    SurfaceDescriptor, SurfaceId, validate_slot_id,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceRegistration {
    pub provider: ProviderIdentity,
    pub framework_generation: FrameworkGeneration,
    pub capabilities: CapabilitySet,
    pub effective_tenant_binding: EffectiveTenantBinding,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<RegisteredSurface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_metadata: Option<ProviderEncryptionMetadata>,
}

impl SurfaceRegistration {
    pub fn validate_against(
        &self,
        policy: &SurfaceRegistrationPolicy,
    ) -> Result<(), SurfaceRegistrationError> {
        if !policy
            .supported_generation
            .includes(self.framework_generation)
        {
            return Err(SurfaceRegistrationError::new(
                SurfaceRegistrationErrorCode::UnsupportedGeneration,
                format!(
                    "framework generation {}.{} is outside supported range {}.{}..={}.{}",
                    self.framework_generation.major,
                    self.framework_generation.minor,
                    policy.supported_generation.min.major,
                    policy.supported_generation.min.minor,
                    policy.supported_generation.max.major,
                    policy.supported_generation.max.minor,
                ),
            ));
        }

        if !self
            .capabilities
            .contains_all(&policy.required_capabilities)
        {
            return Err(SurfaceRegistrationError::new(
                SurfaceRegistrationErrorCode::MissingCapability,
                "registration is missing one or more required capabilities".to_owned(),
            ));
        }

        for surface in &self.surfaces {
            if surface.descriptor.provider_kind != self.provider.provider_kind {
                return Err(SurfaceRegistrationError::new(
                    SurfaceRegistrationErrorCode::InvalidContract,
                    format!(
                        "surface `{}` provider_kind does not match registration provider_kind",
                        surface.descriptor.surface_id
                    ),
                ));
            }

            validate_slot_id(&surface.descriptor.slot).map_err(|err| {
                let code = match err {
                    SlotValidationError::UnknownSlot(_) => {
                        SurfaceRegistrationErrorCode::InvalidSlot
                    }
                    SlotValidationError::InvalidIdentifier(_) => {
                        SurfaceRegistrationErrorCode::InvalidContract
                    }
                };
                SurfaceRegistrationError::new(code, err.to_string())
            })?;

            let mut interaction_ids: HashSet<&str> = HashSet::new();
            for interaction in &surface.interactions {
                if !interaction_ids.insert(interaction.interaction_id.as_str()) {
                    return Err(SurfaceRegistrationError::new(
                        SurfaceRegistrationErrorCode::InvalidContract,
                        format!(
                            "duplicate interaction_id `{}` within surface `{}`",
                            interaction.interaction_id, surface.descriptor.surface_id
                        ),
                    ));
                }

                interaction
                    .validate_for_provider(surface.descriptor.provider_kind)
                    .map_err(|err| {
                        SurfaceRegistrationError::new(
                            SurfaceRegistrationErrorCode::InvalidContract,
                            err.to_string(),
                        )
                    })?;
            }

            let mut data_source_ids: HashSet<&str> = HashSet::new();
            for data_source in &surface.data_sources {
                if !data_source_ids.insert(data_source.data_source_id.as_str()) {
                    return Err(SurfaceRegistrationError::new(
                        SurfaceRegistrationErrorCode::InvalidContract,
                        format!(
                            "duplicate data_source_id `{}` within surface `{}`",
                            data_source.data_source_id, surface.descriptor.surface_id
                        ),
                    ));
                }

                data_source
                    .validate_for_provider(surface.descriptor.provider_kind)
                    .map_err(|err| {
                        SurfaceRegistrationError::new(
                            SurfaceRegistrationErrorCode::InvalidContract,
                            err.to_string(),
                        )
                    })?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderIdentity {
    pub provider_id: String,
    pub provider_kind: ProviderKind,
    pub provider_namespace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveTenantBinding {
    pub scope: Scope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEncryptionMetadata {
    pub key_id: String,
    pub algorithm: ProviderEncryptionAlgorithm,
    pub public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderEncryptionAlgorithm {
    EciesP256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredSurface {
    pub descriptor: SurfaceDescriptor,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interactions: Vec<InteractionDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data_sources: Vec<DataSourceDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceRegistrationPolicy {
    pub supported_generation: FrameworkGenerationRange,
    pub required_capabilities: CapabilitySet,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code:?}: {message}")]
pub struct SurfaceRegistrationError {
    pub code: SurfaceRegistrationErrorCode,
    pub message: String,
}

impl SurfaceRegistrationError {
    pub fn new(code: SurfaceRegistrationErrorCode, message: String) -> Self {
        Self { code, message }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceRegistrationErrorCode {
    UnsupportedGeneration,
    MissingCapability,
    InvalidSlot,
    InvalidContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceActionRequest {
    pub request_id: Uuid,
    pub tenant_id: String,
    pub surface_id: SurfaceId,
    pub interaction_id: InteractionId,
    pub idempotency_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_provider_id: Option<String>,
    pub caller_origin: CallerOrigin,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub params: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_sensitive_params: Option<EncryptedSensitiveParams>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CallerOrigin {
    UserSession { user_id: String, session_id: String },
    BuiltInSystem { principal: String },
    Provider { provider_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedSensitiveParams {
    pub key_id: String,
    pub algorithm: ProviderEncryptionAlgorithm,
    pub ciphertext_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceActionCancel {
    pub request_id: Uuid,
    pub target_provider_id: String,
    pub reason: SurfaceActionCancelReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceActionCancelReason {
    Timeout,
    RequestCancelled,
    ProviderDisconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceActionResponse {
    pub request_id: Uuid,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<SurfaceActionError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceActionError {
    pub code: SurfaceActionErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceActionErrorCode {
    PermissionDenied,
    InvalidRequest,
    SchemaValidationFailed,
    UnsupportedCapability,
    ProviderUnavailable,
    Timeout,
    DuplicateRequest,
    InternalError,
}
