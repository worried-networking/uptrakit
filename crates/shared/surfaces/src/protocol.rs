use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    Capability, CapabilitySet, DataSourceDescriptor, DataSourceKind, FrameworkGeneration,
    FrameworkGenerationRange, InteractionDescriptor, InteractionId, InteractionKind,
    InteractionTransport, ProviderKind, Scope, SlotValidationError, SurfaceDescriptor, SurfaceId,
    SurfaceNode, Targeting, validate_slot_id, validate_surface_identifier,
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

        let mut surface_ids: HashSet<&str> = HashSet::new();
        let mut single_entry_slots: HashSet<&str> = HashSet::new();
        for surface in &self.surfaces {
            if !surface_ids.insert(surface.descriptor.surface_id.as_str()) {
                return Err(SurfaceRegistrationError::new(
                    SurfaceRegistrationErrorCode::InvalidContract,
                    format!(
                        "duplicate surface_id `{}` within registration batch",
                        surface.descriptor.surface_id
                    ),
                ));
            }

            if surface.descriptor.provider_kind != self.provider.provider_kind {
                return Err(SurfaceRegistrationError::new(
                    SurfaceRegistrationErrorCode::InvalidContract,
                    format!(
                        "surface `{}` provider_kind does not match registration provider_kind",
                        surface.descriptor.surface_id
                    ),
                ));
            }

            let slot_def = validate_slot_id(&surface.descriptor.slot).map_err(|err| {
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

            if !slot_def.multi_entry && !single_entry_slots.insert(slot_def.id) {
                return Err(SurfaceRegistrationError::new(
                    SurfaceRegistrationErrorCode::InvalidContract,
                    format!(
                        "slot `{}` is single-entry and cannot accept multiple surfaces in one registration batch",
                        slot_def.id
                    ),
                ));
            }

            if surface.descriptor.provider_kind != ProviderKind::BuiltIn
                && (surface.descriptor.priority < slot_def.provider_priority_min
                    || surface.descriptor.priority > slot_def.provider_priority_max)
            {
                return Err(SurfaceRegistrationError::new(
                    SurfaceRegistrationErrorCode::InvalidContract,
                    format!(
                        "surface `{}` priority {} is outside slot `{}` provider range {}..={}",
                        surface.descriptor.surface_id,
                        surface.descriptor.priority,
                        slot_def.id,
                        slot_def.provider_priority_min,
                        slot_def.provider_priority_max
                    ),
                ));
            }

            if !self
                .capabilities
                .contains_all(&surface.descriptor.required_capabilities)
            {
                return Err(SurfaceRegistrationError::new(
                    SurfaceRegistrationErrorCode::MissingCapability,
                    format!(
                        "surface `{}` requires capabilities not advertised by registration",
                        surface.descriptor.surface_id
                    ),
                ));
            }

            validate_surface_usage_capabilities(
                &surface.descriptor.surface_id,
                &self.capabilities,
                &surface.descriptor.root_node,
                &surface.descriptor.targeting,
                &surface.interactions,
                &surface.data_sources,
            )?;

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

            validate_root_node_references(
                &surface.descriptor.surface_id,
                &surface.descriptor.root_node,
                &interaction_ids,
                &data_source_ids,
            )?;
        }

        Ok(())
    }
}

fn validate_surface_usage_capabilities(
    surface_id: &SurfaceId,
    capabilities: &CapabilitySet,
    root_node: &SurfaceNode,
    targeting: &Targeting,
    interactions: &[InteractionDescriptor],
    data_sources: &[DataSourceDescriptor],
) -> Result<(), SurfaceRegistrationError> {
    validate_node_capabilities(surface_id, capabilities, root_node)?;

    let targeting_capability = match targeting {
        Targeting::Universal => Capability::UniversalTargeting,
        Targeting::Targeted => Capability::TargetedTargeting,
    };
    require_capability(
        capabilities,
        targeting_capability,
        surface_id,
        "targeting mode",
    )?;

    for interaction in interactions {
        let kind_capability = match interaction.kind {
            InteractionKind::MutationAction => Capability::MutationAction,
            InteractionKind::FormSubmit => Capability::FormSubmit,
            InteractionKind::Workflow => Capability::Workflow,
            InteractionKind::Navigate => Capability::Navigate,
            InteractionKind::DataLoad => Capability::DataLoad,
            InteractionKind::ConfirmableAction => Capability::ConfirmableAction,
        };
        require_capability(
            capabilities,
            kind_capability,
            surface_id,
            "interaction kind",
        )?;

        if matches!(
            &interaction.transport,
            InteractionTransport::ProviderProxied
        ) {
            require_capability(
                capabilities,
                Capability::ProviderInitiatedActions,
                surface_id,
                "interaction transport",
            )?;
        }

        if !interaction.sensitive_fields.is_empty() {
            require_capability(
                capabilities,
                Capability::SensitiveFields,
                surface_id,
                "sensitive fields",
            )?;
        }
    }

    for data_source in data_sources {
        let kind_capability = match &data_source.kind {
            DataSourceKind::Static { .. } => Capability::StaticDataSource,
            DataSourceKind::ControllerQuery { .. } => Capability::ControllerQueryDataSource,
            DataSourceKind::ProviderQuery { .. } => Capability::ProviderQueryDataSource,
        };
        require_capability(
            capabilities,
            kind_capability,
            surface_id,
            "data source kind",
        )?;
    }

    Ok(())
}

fn validate_node_capabilities(
    surface_id: &SurfaceId,
    capabilities: &CapabilitySet,
    node: &SurfaceNode,
) -> Result<(), SurfaceRegistrationError> {
    let node_capability = match node {
        SurfaceNode::Section { children, .. } => {
            for child in children {
                validate_node_capabilities(surface_id, capabilities, child)?;
            }
            Capability::SectionNode
        }
        SurfaceNode::TextBlock { .. } => Capability::TextBlockNode,
        SurfaceNode::KeyValue { .. } => Capability::KeyValueNode,
        SurfaceNode::Table { .. } => Capability::TableNode,
        SurfaceNode::Form { .. } => Capability::FormNode,
        SurfaceNode::ActionBar { .. } => Capability::ActionBarNode,
        SurfaceNode::Tabs { tabs } => {
            for tab in tabs {
                validate_node_capabilities(surface_id, capabilities, &tab.root)?;
            }
            Capability::TabsNode
        }
        SurfaceNode::Callout { .. } => Capability::CalloutNode,
        SurfaceNode::EmptyState { .. } => Capability::EmptyStateNode,
        SurfaceNode::ModalTrigger { modal_nodes, .. } => {
            for child in modal_nodes {
                validate_node_capabilities(surface_id, capabilities, child)?;
            }
            Capability::ModalTriggerNode
        }
        SurfaceNode::WorkflowTrigger { step_nodes, .. } => {
            for child in step_nodes {
                validate_node_capabilities(surface_id, capabilities, child)?;
            }
            Capability::WorkflowTriggerNode
        }
    };

    require_capability(capabilities, node_capability, surface_id, "root_node kind")
}

fn require_capability(
    capabilities: &CapabilitySet,
    required: Capability,
    surface_id: &SurfaceId,
    usage: &str,
) -> Result<(), SurfaceRegistrationError> {
    if capabilities.0.contains(&required) {
        return Ok(());
    }

    Err(SurfaceRegistrationError::new(
        SurfaceRegistrationErrorCode::MissingCapability,
        format!(
            "surface `{}` uses {} that requires capability `{}`",
            surface_id,
            usage,
            serde_json::to_string(&required)
                .unwrap_or_else(|_| "\"unknown\"".to_owned())
                .trim_matches('"')
        ),
    ))
}

fn validate_root_node_references(
    surface_id: &SurfaceId,
    node: &SurfaceNode,
    interaction_ids: &HashSet<&str>,
    data_source_ids: &HashSet<&str>,
) -> Result<(), SurfaceRegistrationError> {
    match node {
        SurfaceNode::Section { children, .. } => {
            for child in children {
                validate_root_node_references(surface_id, child, interaction_ids, data_source_ids)?;
            }
        }
        SurfaceNode::TextBlock { .. } => {}
        SurfaceNode::KeyValue { data_source_id } | SurfaceNode::Table { data_source_id } => {
            if !data_source_ids.contains(data_source_id.as_str()) {
                return Err(SurfaceRegistrationError::new(
                    SurfaceRegistrationErrorCode::InvalidContract,
                    format!(
                        "surface `{}` root_node references unknown data_source_id `{}`",
                        surface_id, data_source_id
                    ),
                ));
            }
        }
        SurfaceNode::Form { interaction_id } => {
            if !interaction_ids.contains(interaction_id.as_str()) {
                return Err(SurfaceRegistrationError::new(
                    SurfaceRegistrationErrorCode::InvalidContract,
                    format!(
                        "surface `{}` root_node references unknown interaction_id `{}`",
                        surface_id, interaction_id
                    ),
                ));
            }
        }
        SurfaceNode::ActionBar { action_ids } => {
            for action_id in action_ids {
                if !interaction_ids.contains(action_id.as_str()) {
                    return Err(SurfaceRegistrationError::new(
                        SurfaceRegistrationErrorCode::InvalidContract,
                        format!(
                            "surface `{}` root_node references unknown interaction_id `{}`",
                            surface_id, action_id
                        ),
                    ));
                }
            }
        }
        SurfaceNode::Tabs { tabs } => {
            let mut tab_ids: HashSet<&str> = HashSet::new();
            for tab in tabs {
                validate_surface_identifier(tab.id.as_str()).map_err(|err| {
                    SurfaceRegistrationError::new(
                        SurfaceRegistrationErrorCode::InvalidContract,
                        format!(
                            "surface `{}` root_node contains invalid tab id `{}`: {}",
                            surface_id, tab.id, err
                        ),
                    )
                })?;

                if !tab_ids.insert(tab.id.as_str()) {
                    return Err(SurfaceRegistrationError::new(
                        SurfaceRegistrationErrorCode::InvalidContract,
                        format!(
                            "surface `{}` root_node contains duplicate tab id `{}` within one tabs node",
                            surface_id, tab.id
                        ),
                    ));
                }

                validate_root_node_references(
                    surface_id,
                    &tab.root,
                    interaction_ids,
                    data_source_ids,
                )?;
            }
        }
        SurfaceNode::Callout { .. } | SurfaceNode::EmptyState { .. } => {}
        SurfaceNode::ModalTrigger {
            interaction_id,
            modal_nodes,
        } => {
            if !interaction_ids.contains(interaction_id.as_str()) {
                return Err(SurfaceRegistrationError::new(
                    SurfaceRegistrationErrorCode::InvalidContract,
                    format!(
                        "surface `{}` root_node references unknown interaction_id `{}`",
                        surface_id, interaction_id
                    ),
                ));
            }
            for child in modal_nodes {
                validate_root_node_references(surface_id, child, interaction_ids, data_source_ids)?;
            }
        }
        SurfaceNode::WorkflowTrigger {
            interaction_id,
            step_nodes,
        } => {
            if !interaction_ids.contains(interaction_id.as_str()) {
                return Err(SurfaceRegistrationError::new(
                    SurfaceRegistrationErrorCode::InvalidContract,
                    format!(
                        "surface `{}` root_node references unknown interaction_id `{}`",
                        surface_id, interaction_id
                    ),
                ));
            }
            for child in step_nodes {
                validate_root_node_references(surface_id, child, interaction_ids, data_source_ids)?;
            }
        }
    }

    Ok(())
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
