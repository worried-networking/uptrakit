// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::surfaces::{
    Capability, CapabilitySet, DataSourceDescriptor, DataSourceKind, FrameworkGeneration,
    FrameworkGenerationRange, InteractionDescriptor, InteractionId, InteractionKind,
    InteractionTransport, ProviderKind, Scope, SlotValidationError, SurfaceDescriptor, SurfaceId,
    SurfaceNode, SurfaceSlotDef, Targeting, validate_slot_id, validate_surface_identifier,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use thiserror::Error;
use uuid::Uuid;
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
    /// Validates a registration payload against policy and contract rules.
    ///
    /// # Errors
    /// Returns [`SurfaceRegistrationError`] when any validation check fails,
    /// including unsupported framework generation, missing required
    /// capabilities, invalid or duplicate identifiers, slot violations,
    /// provider-kind mismatches, invalid interaction/data-source declarations,
    /// or broken cross-reference links in the surface graph.
    pub fn validate_against(
        &self,
        policy: &SurfaceRegistrationPolicy,
    ) -> Result<(), SurfaceRegistrationError> {
        validate_supported_generation(self, policy)?;
        validate_required_registration_capabilities(self, policy)?;
        validate_registered_surfaces(self)?;
        Ok(())
    }
}
fn validate_supported_generation(
    registration: &SurfaceRegistration,
    policy: &SurfaceRegistrationPolicy,
) -> Result<(), SurfaceRegistrationError> {
    if policy
        .supported_generation
        .includes(registration.framework_generation)
    {
        return Ok(());
    }
    Err(registration_error(
        SurfaceRegistrationErrorCode::UnsupportedGeneration,
        format!(
            "framework generation {}.{} is outside supported range {}.{}..={}.{}",
            registration.framework_generation.major,
            registration.framework_generation.minor,
            policy.supported_generation.min.major,
            policy.supported_generation.min.minor,
            policy.supported_generation.max.major,
            policy.supported_generation.max.minor,
        ),
    ))
}
fn validate_required_registration_capabilities(
    registration: &SurfaceRegistration,
    policy: &SurfaceRegistrationPolicy,
) -> Result<(), SurfaceRegistrationError> {
    if registration
        .capabilities
        .contains_all(&policy.required_capabilities)
    {
        return Ok(());
    }
    Err(missing_capability(
        "registration is missing one or more required capabilities",
    ))
}
fn validate_registered_surfaces(
    registration: &SurfaceRegistration,
) -> Result<(), SurfaceRegistrationError> {
    RegisteredSurfacesValidator::new(registration).validate()
}
struct RegisteredSurfacesValidator<'a> {
    provider_kind: ProviderKind,
    capabilities: &'a CapabilitySet,
    surfaces: &'a [RegisteredSurface],
    surface_ids: HashSet<&'a str>,
    single_entry_slots: HashSet<&'static str>,
}
impl<'a> RegisteredSurfacesValidator<'a> {
    fn new(registration: &'a SurfaceRegistration) -> Self {
        Self {
            provider_kind: registration.provider.provider_kind,
            capabilities: &registration.capabilities,
            surfaces: &registration.surfaces,
            surface_ids: HashSet::new(),
            single_entry_slots: HashSet::new(),
        }
    }
    fn validate(mut self) -> Result<(), SurfaceRegistrationError> {
        for surface in self.surfaces {
            self.validate_surface(surface)?;
        }
        Ok(())
    }
    fn validate_surface(
        &mut self,
        surface: &'a RegisteredSurface,
    ) -> Result<(), SurfaceRegistrationError> {
        validate_surface_descriptor_rules(
            surface,
            self.provider_kind,
            self.capabilities,
            &mut self.surface_ids,
            &mut self.single_entry_slots,
        )?;
        let interaction_ids = validate_surface_interaction_rules(surface)?;
        validate_workflow_step_references(
            &surface.descriptor.surface_id,
            &surface.interactions,
            &interaction_ids,
        )?;
        let data_source_ids = validate_surface_data_source_rules(surface)?;
        validate_root_node_references(
            &surface.descriptor.surface_id,
            &surface.descriptor.root_node,
            &interaction_ids,
            &data_source_ids,
        )
    }
}
fn validate_surface_descriptor_rules<'a>(
    surface: &'a RegisteredSurface,
    registration_provider_kind: ProviderKind,
    capabilities: &CapabilitySet,
    surface_ids: &mut HashSet<&'a str>,
    single_entry_slots: &mut HashSet<&'static str>,
) -> Result<(), SurfaceRegistrationError> {
    validate_unique_surface_id(surface, surface_ids)?;
    validate_surface_provider_kind(surface, registration_provider_kind)?;
    let slot_def = validate_surface_slot(surface)?;
    validate_single_entry_slot_occupancy(slot_def, single_entry_slots)?;
    validate_surface_priority_range(surface, slot_def)?;
    validate_surface_required_capabilities(surface, capabilities)?;
    validate_surface_usage_capabilities(
        &surface.descriptor.surface_id,
        capabilities,
        &surface.descriptor.root_node,
        &surface.descriptor.targeting,
        &surface.interactions,
        &surface.data_sources,
    )
}
fn validate_unique_surface_id<'a>(
    surface: &'a RegisteredSurface,
    surface_ids: &mut HashSet<&'a str>,
) -> Result<(), SurfaceRegistrationError> {
    if surface_ids.insert(surface.descriptor.surface_id.as_str()) {
        return Ok(());
    }
    Err(invalid_contract(format!(
        "duplicate surface_id `{}` within registration batch",
        surface.descriptor.surface_id
    )))
}
fn validate_surface_provider_kind(
    surface: &RegisteredSurface,
    registration_provider_kind: ProviderKind,
) -> Result<(), SurfaceRegistrationError> {
    if surface.descriptor.provider_kind == registration_provider_kind {
        return Ok(());
    }
    Err(invalid_contract(format!(
        "surface `{}` provider_kind does not match registration provider_kind",
        surface.descriptor.surface_id
    )))
}
fn validate_surface_slot(
    surface: &RegisteredSurface,
) -> Result<&'static SurfaceSlotDef, SurfaceRegistrationError> {
    validate_slot_id(&surface.descriptor.slot).map_err(map_slot_validation_error)
}
fn validate_single_entry_slot_occupancy(
    slot_def: &'static SurfaceSlotDef,
    single_entry_slots: &mut HashSet<&'static str>,
) -> Result<(), SurfaceRegistrationError> {
    if slot_def.multi_entry || single_entry_slots.insert(slot_def.id) {
        return Ok(());
    }
    Err(invalid_contract(format!(
        "slot `{}` is single-entry and cannot accept multiple surfaces in one registration batch",
        slot_def.id
    )))
}
fn validate_surface_priority_range(
    surface: &RegisteredSurface,
    slot_def: &'static SurfaceSlotDef,
) -> Result<(), SurfaceRegistrationError> {
    if surface.descriptor.provider_kind == ProviderKind::BuiltIn {
        return Ok(());
    }
    if surface.descriptor.priority >= slot_def.provider_priority_min
        && surface.descriptor.priority <= slot_def.provider_priority_max
    {
        return Ok(());
    }
    Err(invalid_contract(format!(
        "surface `{}` priority {} is outside slot `{}` provider range {}..={}",
        surface.descriptor.surface_id,
        surface.descriptor.priority,
        slot_def.id,
        slot_def.provider_priority_min,
        slot_def.provider_priority_max
    )))
}
fn validate_surface_required_capabilities(
    surface: &RegisteredSurface,
    capabilities: &CapabilitySet,
) -> Result<(), SurfaceRegistrationError> {
    if capabilities.contains_all(&surface.descriptor.required_capabilities) {
        return Ok(());
    }
    Err(missing_capability(format!(
        "surface `{}` requires capabilities not advertised by registration",
        surface.descriptor.surface_id
    )))
}
fn validate_surface_interaction_rules(
    surface: &RegisteredSurface,
) -> Result<HashSet<&str>, SurfaceRegistrationError> {
    let mut interaction_ids: HashSet<&str> = HashSet::new();
    for interaction in &surface.interactions {
        validate_unique_interaction_id(surface, interaction, &mut interaction_ids)?;
        validate_interaction_provider_rules(surface, interaction)?;
    }
    Ok(interaction_ids)
}
fn validate_unique_interaction_id<'a>(
    surface: &'a RegisteredSurface,
    interaction: &'a InteractionDescriptor,
    interaction_ids: &mut HashSet<&'a str>,
) -> Result<(), SurfaceRegistrationError> {
    if interaction_ids.insert(interaction.interaction_id.as_str()) {
        return Ok(());
    }
    Err(invalid_contract(format!(
        "duplicate interaction_id `{}` within surface `{}`",
        interaction.interaction_id, surface.descriptor.surface_id
    )))
}
fn validate_interaction_provider_rules(
    surface: &RegisteredSurface,
    interaction: &InteractionDescriptor,
) -> Result<(), SurfaceRegistrationError> {
    interaction
        .validate_for_provider(surface.descriptor.provider_kind)
        .map_err(|err| invalid_contract(err.to_string()))
}
fn validate_surface_data_source_rules(
    surface: &RegisteredSurface,
) -> Result<HashSet<&str>, SurfaceRegistrationError> {
    let mut data_source_ids: HashSet<&str> = HashSet::new();
    for data_source in &surface.data_sources {
        validate_unique_data_source_id(surface, data_source, &mut data_source_ids)?;
        validate_data_source_provider_rules(surface, data_source)?;
    }
    Ok(data_source_ids)
}
fn validate_unique_data_source_id<'a>(
    surface: &'a RegisteredSurface,
    data_source: &'a DataSourceDescriptor,
    data_source_ids: &mut HashSet<&'a str>,
) -> Result<(), SurfaceRegistrationError> {
    if data_source_ids.insert(data_source.data_source_id.as_str()) {
        return Ok(());
    }
    Err(invalid_contract(format!(
        "duplicate data_source_id `{}` within surface `{}`",
        data_source.data_source_id, surface.descriptor.surface_id
    )))
}
fn validate_data_source_provider_rules(
    surface: &RegisteredSurface,
    data_source: &DataSourceDescriptor,
) -> Result<(), SurfaceRegistrationError> {
    data_source
        .validate_for_provider(surface.descriptor.provider_kind)
        .map_err(|err| invalid_contract(err.to_string()))
}
fn map_slot_validation_error(err: SlotValidationError) -> SurfaceRegistrationError {
    let code = match err {
        SlotValidationError::UnknownSlot(_) => SurfaceRegistrationErrorCode::InvalidSlot,
        SlotValidationError::InvalidIdentifier(_) => SurfaceRegistrationErrorCode::InvalidContract,
    };
    registration_error(code, err.to_string())
}
fn registration_error(
    code: SurfaceRegistrationErrorCode,
    message: impl Into<String>,
) -> SurfaceRegistrationError {
    SurfaceRegistrationError::new(code, message.into())
}
fn invalid_contract(message: impl Into<String>) -> SurfaceRegistrationError {
    registration_error(SurfaceRegistrationErrorCode::InvalidContract, message)
}
fn missing_capability(message: impl Into<String>) -> SurfaceRegistrationError {
    registration_error(SurfaceRegistrationErrorCode::MissingCapability, message)
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
    Err(missing_capability(format!(
        "surface `{}` uses {} that requires capability `{}`",
        surface_id,
        usage,
        serde_json::to_string(&required)
            .unwrap_or_else(|_| "\"unknown\"".to_owned())
            .trim_matches('"')
    )))
}
fn validate_root_node_references(
    surface_id: &SurfaceId,
    node: &SurfaceNode,
    interaction_ids: &HashSet<&str>,
    data_source_ids: &HashSet<&str>,
) -> Result<(), SurfaceRegistrationError> {
    RootNodeReferenceValidator::new(surface_id, interaction_ids, data_source_ids).validate(node)
}
struct RootNodeReferenceValidator<'a> {
    surface_id: &'a SurfaceId,
    interaction_ids: &'a HashSet<&'a str>,
    data_source_ids: &'a HashSet<&'a str>,
}
impl<'a> RootNodeReferenceValidator<'a> {
    fn new(
        surface_id: &'a SurfaceId,
        interaction_ids: &'a HashSet<&'a str>,
        data_source_ids: &'a HashSet<&'a str>,
    ) -> Self {
        Self {
            surface_id,
            interaction_ids,
            data_source_ids,
        }
    }
    fn validate(&self, node: &SurfaceNode) -> Result<(), SurfaceRegistrationError> {
        match node {
            SurfaceNode::Section { children, .. } => self.validate_children(children),
            SurfaceNode::TextBlock { .. } => Ok(()),
            SurfaceNode::KeyValue { data_source_id } => {
                self.require_data_source_reference(data_source_id.as_str())
            }
            SurfaceNode::Table {
                data_source_id,
                row_actions,
                ..
            } => {
                self.require_data_source_reference(data_source_id.as_str())?;
                self.validate_table_row_actions(row_actions)
            }
            SurfaceNode::Form { interaction_id } => {
                self.require_root_interaction_reference(interaction_id.as_str())
            }
            SurfaceNode::ActionBar { action_ids } => self.validate_action_bar(action_ids),
            SurfaceNode::Tabs { tabs } => self.validate_tabs(tabs),
            SurfaceNode::Callout { .. } | SurfaceNode::EmptyState { .. } => Ok(()),
            SurfaceNode::ModalTrigger {
                interaction_id,
                modal_nodes,
            } => {
                self.require_root_interaction_reference(interaction_id.as_str())?;
                self.validate_children(modal_nodes)
            }
            SurfaceNode::WorkflowTrigger {
                interaction_id,
                step_nodes,
            } => {
                self.require_root_interaction_reference(interaction_id.as_str())?;
                self.validate_children(step_nodes)
            }
        }
    }
    fn validate_children(&self, nodes: &[SurfaceNode]) -> Result<(), SurfaceRegistrationError> {
        for child in nodes {
            self.validate(child)?;
        }
        Ok(())
    }
    fn validate_table_row_actions(
        &self,
        row_actions: &[crate::generated::surfaces::SurfaceTableRowAction],
    ) -> Result<(), SurfaceRegistrationError> {
        for row_action in row_actions {
            self.require_interaction_reference(row_action.interaction_id.as_str(), || {
                format!(
                    "surface `{}` table references unknown row-action interaction_id `{}`",
                    self.surface_id, row_action.interaction_id
                )
            })?;
        }
        Ok(())
    }
    fn validate_action_bar(
        &self,
        action_ids: &[InteractionId],
    ) -> Result<(), SurfaceRegistrationError> {
        for action_id in action_ids {
            self.require_root_interaction_reference(action_id.as_str())?;
        }
        Ok(())
    }
    fn validate_tabs(
        &self,
        tabs: &[crate::generated::surfaces::SurfaceTab],
    ) -> Result<(), SurfaceRegistrationError> {
        let mut tab_ids: HashSet<&str> = HashSet::new();
        for tab in tabs {
            validate_surface_identifier(tab.id.as_str()).map_err(|err| {
                invalid_contract(format!(
                    "surface `{}` root_node contains invalid tab id `{}`: {}",
                    self.surface_id, tab.id, err
                ))
            })?;
            if !tab_ids.insert(tab.id.as_str()) {
                return Err(invalid_contract(format!(
                    "surface `{}` root_node contains duplicate tab id `{}` within one tabs node",
                    self.surface_id, tab.id
                )));
            }
            self.validate(&tab.root)?;
        }
        Ok(())
    }
    fn require_root_interaction_reference(
        &self,
        interaction_id: &str,
    ) -> Result<(), SurfaceRegistrationError> {
        self.require_interaction_reference(interaction_id, || {
            format!(
                "surface `{}` root_node references unknown interaction_id `{}`",
                self.surface_id, interaction_id
            )
        })
    }
    fn require_data_source_reference(
        &self,
        data_source_id: &str,
    ) -> Result<(), SurfaceRegistrationError> {
        ensure_known_reference(self.data_source_ids, data_source_id, || {
            format!(
                "surface `{}` root_node references unknown data_source_id `{}`",
                self.surface_id, data_source_id
            )
        })
    }
    fn require_interaction_reference(
        &self,
        interaction_id: &str,
        error_message: impl FnOnce() -> String,
    ) -> Result<(), SurfaceRegistrationError> {
        ensure_known_reference(self.interaction_ids, interaction_id, error_message)
    }
}
fn validate_workflow_step_references(
    surface_id: &SurfaceId,
    interactions: &[InteractionDescriptor],
    interaction_ids: &HashSet<&str>,
) -> Result<(), SurfaceRegistrationError> {
    WorkflowStepReferenceValidator::new(surface_id, interaction_ids).validate(interactions)
}
struct WorkflowStepReferenceValidator<'a> {
    surface_id: &'a SurfaceId,
    interaction_ids: &'a HashSet<&'a str>,
}
impl<'a> WorkflowStepReferenceValidator<'a> {
    fn new(surface_id: &'a SurfaceId, interaction_ids: &'a HashSet<&'a str>) -> Self {
        Self {
            surface_id,
            interaction_ids,
        }
    }
    fn validate(
        &self,
        interactions: &[InteractionDescriptor],
    ) -> Result<(), SurfaceRegistrationError> {
        for interaction in interactions {
            if interaction.kind != InteractionKind::Workflow {
                continue;
            }
            self.validate_workflow_interaction_steps(interaction)?;
        }
        Ok(())
    }
    fn validate_workflow_interaction_steps(
        &self,
        interaction: &InteractionDescriptor,
    ) -> Result<(), SurfaceRegistrationError> {
        for step in &interaction.workflow_steps {
            if let Some(submit_interaction_id) = &step.submit_interaction_id {
                ensure_known_reference(
                    self.interaction_ids,
                    submit_interaction_id.as_str(),
                    || {
                        format!(
                            "surface `{}` workflow interaction `{}` references unknown submit_interaction_id `{}` in step `{}`",
                            self.surface_id,
                            interaction.interaction_id,
                            submit_interaction_id,
                            step.step_id
                        )
                    },
                )?;
            }
            if let Some(form_ui) = &step.form_ui
                && let Some(pre_load_interaction_id) = &form_ui.pre_load_interaction_id
            {
                ensure_known_reference(
                    self.interaction_ids,
                    pre_load_interaction_id.as_str(),
                    || {
                        format!(
                            "surface `{}` workflow interaction `{}` references unknown pre_load_interaction_id `{}` in step `{}`",
                            self.surface_id,
                            interaction.interaction_id,
                            pre_load_interaction_id,
                            step.step_id
                        )
                    },
                )?;
            }
        }
        Ok(())
    }
}
fn ensure_known_reference(
    known_ids: &HashSet<&str>,
    reference_id: &str,
    error_message: impl FnOnce() -> String,
) -> Result<(), SurfaceRegistrationError> {
    if known_ids.contains(reference_id) {
        return Ok(());
    }
    Err(invalid_contract(error_message()))
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
