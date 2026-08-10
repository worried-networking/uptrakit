use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use uptrakit_shared_macros::wire_safe_enum;
use uuid::Uuid;

use crate::{
    Capability, CapabilitySet, DataSourceDescriptor, DataSourceKind, FrameworkGeneration,
    FrameworkGenerationRange, InteractionDescriptor, InteractionHttpMethod, InteractionId,
    InteractionKind, InteractionTransport, ProviderKind, RESERVED_PARAM_KEYS, SchemaContract,
    Scope, SlotValidationError, SurfaceDescriptor, SurfaceId, SurfaceNode, SurfaceSlotDef,
    Targeting, validate_slot_id, validate_surface_identifier,
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

        let interaction_methods = validate_surface_interaction_rules(surface)?;
        validate_workflow_step_references(
            &surface.descriptor.surface_id,
            &surface.interactions,
            &interaction_methods,
        )?;

        let data_source_ids = validate_surface_data_source_rules(surface, &interaction_methods)?;
        validate_root_node_references(
            &surface.descriptor.surface_id,
            &surface.descriptor.root_node,
            &interaction_methods,
            &surface.interactions,
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
) -> Result<HashMap<&str, Vec<InteractionHttpMethod>>, SurfaceRegistrationError> {
    let mut interaction_methods: HashMap<&str, Vec<InteractionHttpMethod>> = HashMap::new();
    for interaction in &surface.interactions {
        validate_unique_interaction_id(surface, interaction, &mut interaction_methods)?;
        validate_interaction_method(surface, interaction)?;
        validate_interaction_params(surface, interaction)?;
        validate_interaction_provider_rules(surface, interaction)?;
    }

    Ok(interaction_methods)
}

/// Re-keyed on `(interaction_id, effective_http_method)` (REST method model,
/// spec B1): a single `interaction_id` may legitimately register under
/// multiple HTTP methods within one surface. This map doubles as the
/// reference-resolution index consumed by `resolve_interaction_reference`
/// and `require_pair_reference`.
fn validate_unique_interaction_id<'a>(
    surface: &'a RegisteredSurface,
    interaction: &'a InteractionDescriptor,
    interaction_methods: &mut HashMap<&'a str, Vec<InteractionHttpMethod>>,
) -> Result<(), SurfaceRegistrationError> {
    let method = interaction.effective_http_method();
    let methods = interaction_methods
        .entry(interaction.interaction_id.as_str())
        .or_default();
    if methods.contains(&method) {
        return Err(invalid_contract(format!(
            "duplicate interaction `{}` [{}] within surface `{}`",
            interaction.interaction_id, method, surface.descriptor.surface_id
        )));
    }
    methods.push(method);
    Ok(())
}

/// Kind/method matrix (spec B1): `Other(_)` declared methods are always
/// rejected; `DataLoad` must not declare PUT/DELETE; `Workflow` must declare
/// POST; every other kind must not declare GET.
fn validate_interaction_method(
    surface: &RegisteredSurface,
    interaction: &InteractionDescriptor,
) -> Result<(), SurfaceRegistrationError> {
    let id = &interaction.interaction_id;
    let surface_id = &surface.descriptor.surface_id;
    if matches!(interaction.http_method, InteractionHttpMethod::Other(_)) {
        return Err(invalid_contract(format!(
            "interaction `{id}` in surface `{surface_id}` declares an unknown http_method"
        )));
    }
    match interaction.kind {
        // POST is indistinguishable from an omitted field (serde default) and
        // normalizes to GET; only PUT/DELETE are observably-wrong declarations.
        InteractionKind::DataLoad => {
            if matches!(
                interaction.http_method,
                InteractionHttpMethod::Put | InteractionHttpMethod::Delete
            ) {
                return Err(invalid_contract(format!(
                    "data-load interaction `{id}` in surface `{surface_id}` must use GET"
                )));
            }
        }
        InteractionKind::Workflow => {
            if interaction.http_method != InteractionHttpMethod::Post {
                return Err(invalid_contract(format!(
                    "workflow interaction `{id}` in surface `{surface_id}` must use POST"
                )));
            }
        }
        // Navigate is deliberately in this catch-all: spec B1 assigns it no
        // method (it is never HTTP-dispatched), so it keeps the declared-method
        // rule like FormSubmit/MutationAction/ConfirmableAction.
        _ => {
            if interaction.http_method == InteractionHttpMethod::Get {
                return Err(invalid_contract(format!(
                    "interaction `{id}` in surface `{surface_id}` is not a data-load and cannot use GET"
                )));
            }
        }
    }
    Ok(())
}

/// Params rules (spec §4 rule 1): reserved key collisions, duplicate keys,
/// non-scalar `DataLoad` param schemas (GET query strings carry scalars
/// only), and non-empty `sensitive_fields` on `DataLoad` (GET params travel
/// in query strings, never a request body).
fn validate_interaction_params(
    surface: &RegisteredSurface,
    interaction: &InteractionDescriptor,
) -> Result<(), SurfaceRegistrationError> {
    let id = &interaction.interaction_id;
    let surface_id = &surface.descriptor.surface_id;
    let mut seen = HashSet::new();
    for field in &interaction.params {
        if RESERVED_PARAM_KEYS.contains(&field.key.as_str()) {
            return Err(invalid_contract(format!(
                "interaction `{id}` in surface `{surface_id}` declares reserved param key `{}`",
                field.key
            )));
        }
        if !seen.insert(field.key.as_str()) {
            return Err(invalid_contract(format!(
                "interaction `{id}` in surface `{surface_id}` declares duplicate param key `{}`",
                field.key
            )));
        }
        if interaction.kind == InteractionKind::DataLoad
            && !matches!(
                field.schema,
                SchemaContract::String
                    | SchemaContract::Integer
                    | SchemaContract::Number
                    | SchemaContract::Boolean
            )
        {
            return Err(invalid_contract(format!(
                "data-load interaction `{id}` in surface `{surface_id}` param `{}` must be a scalar schema",
                field.key
            )));
        }
    }
    if interaction.kind == InteractionKind::DataLoad && !interaction.sensitive_fields.is_empty() {
        return Err(invalid_contract(format!(
            "data-load interaction `{id}` in surface `{surface_id}` must not declare sensitive_fields \
             (GET params travel in query strings)"
        )));
    }
    Ok(())
}

fn validate_interaction_provider_rules(
    surface: &RegisteredSurface,
    interaction: &InteractionDescriptor,
) -> Result<(), SurfaceRegistrationError> {
    interaction
        .validate_for_provider(surface.descriptor.provider_kind)
        .map_err(|err| invalid_contract(err.to_string()))?;

    // Presence-only read of the wire string `descriptor.required_action` is
    // sound at admission: the registry parses it to a typed `Action` right
    // after contract validation and rejects unparseable values, so a
    // present-but-invalid gate never reaches the runtime path. The
    // `provider_kind` consulted here is trustworthy only via a two-hop pin:
    // `validate_surface_provider_kind` pins descriptor.provider_kind to the
    // registration's provider_kind, and the registry's
    // `validate_registration_basics` pins that to the trusted connection
    // source_kind. Do not simplify either hop away.
    if interaction.provider_invocable
        && surface.descriptor.provider_kind == ProviderKind::Service
        && surface.descriptor.required_action.is_some()
    {
        return Err(invalid_contract(format!(
            "interaction `{}` in surface `{}` sets provider_invocable under an \
             action-gated surface descriptor — not allowed for service-registered surfaces",
            interaction.interaction_id, surface.descriptor.surface_id
        )));
    }
    Ok(())
}

fn validate_surface_data_source_rules<'a>(
    surface: &'a RegisteredSurface,
    interaction_methods: &HashMap<&str, Vec<InteractionHttpMethod>>,
) -> Result<HashSet<&'a str>, SurfaceRegistrationError> {
    let mut data_source_ids: HashSet<&str> = HashSet::new();
    for data_source in &surface.data_sources {
        validate_unique_data_source_id(surface, data_source, &mut data_source_ids)?;
        validate_data_source_provider_rules(surface, data_source)?;
        validate_data_source_reference_rules(surface, data_source, interaction_methods)?;
    }

    Ok(data_source_ids)
}

/// `DataSourceKind::ProviderQuery.operation_id` must resolve to a same-surface
/// interaction registered under GET (NEW rule — previously unvalidated).
fn validate_data_source_reference_rules(
    surface: &RegisteredSurface,
    data_source: &DataSourceDescriptor,
    interaction_methods: &HashMap<&str, Vec<InteractionHttpMethod>>,
) -> Result<(), SurfaceRegistrationError> {
    if let DataSourceKind::ProviderQuery { operation_id } = &data_source.kind {
        require_pair_reference(
            interaction_methods,
            operation_id,
            InteractionHttpMethod::Get,
            || {
                format!(
                    "surface `{}` data source `{}` references unknown provider_query operation_id `{}`",
                    surface.descriptor.surface_id, data_source.data_source_id, operation_id
                )
            },
        )?;
    }
    Ok(())
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
    interaction_methods: &HashMap<&str, Vec<InteractionHttpMethod>>,
    interactions: &[InteractionDescriptor],
    data_source_ids: &HashSet<&str>,
) -> Result<(), SurfaceRegistrationError> {
    RootNodeReferenceValidator::new(
        surface_id,
        interaction_methods,
        interactions,
        data_source_ids,
    )
    .validate(node)
}

struct RootNodeReferenceValidator<'a> {
    surface_id: &'a SurfaceId,
    interaction_methods: &'a HashMap<&'a str, Vec<InteractionHttpMethod>>,
    interactions: &'a [InteractionDescriptor],
    data_source_ids: &'a HashSet<&'a str>,
}

impl<'a> RootNodeReferenceValidator<'a> {
    fn new(
        surface_id: &'a SurfaceId,
        interaction_methods: &'a HashMap<&'a str, Vec<InteractionHttpMethod>>,
        interactions: &'a [InteractionDescriptor],
        data_source_ids: &'a HashSet<&'a str>,
    ) -> Self {
        Self {
            surface_id,
            interaction_methods,
            interactions,
            data_source_ids,
        }
    }

    fn validate(&self, node: &SurfaceNode) -> Result<(), SurfaceRegistrationError> {
        match node {
            // header_action_ids are kind-gated in registry.rs, not resolved here (out of scope).
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
            SurfaceNode::Form {
                interaction_id,
                http_method,
            } => {
                self.require_root_interaction_reference(
                    interaction_id.as_str(),
                    http_method.as_ref(),
                )?;
                self.require_form_pre_load_reference(interaction_id.as_str(), http_method.as_ref())
            }
            SurfaceNode::ActionBar { action_ids } => self.validate_action_bar(action_ids),
            SurfaceNode::Tabs { tabs } => self.validate_tabs(tabs),
            SurfaceNode::Callout { .. } | SurfaceNode::EmptyState { .. } => Ok(()),
            SurfaceNode::ModalTrigger {
                interaction_id,
                http_method,
                modal_nodes,
            } => {
                self.require_root_interaction_reference(
                    interaction_id.as_str(),
                    http_method.as_ref(),
                )?;
                self.validate_children(modal_nodes)
            }
            SurfaceNode::WorkflowTrigger {
                interaction_id,
                step_nodes,
            } => {
                self.require_root_workflow_trigger_reference(interaction_id.as_str())?;
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
        row_actions: &[crate::SurfaceTableRowAction],
    ) -> Result<(), SurfaceRegistrationError> {
        for row_action in row_actions {
            resolve_interaction_reference(
                self.interaction_methods,
                row_action.interaction_id.as_str(),
                row_action.http_method.as_ref(),
                || {
                    format!(
                        "surface `{}` table references unknown row-action interaction_id `{}`",
                        self.surface_id, row_action.interaction_id
                    )
                },
            )?;
        }
        Ok(())
    }

    fn validate_action_bar(
        &self,
        action_ids: &[crate::ActionRef],
    ) -> Result<(), SurfaceRegistrationError> {
        for action_ref in action_ids {
            resolve_interaction_reference(
                self.interaction_methods,
                action_ref.interaction_id().as_str(),
                action_ref.http_method(),
                || {
                    format!(
                        "surface `{}` root_node references unknown interaction_id `{}`",
                        self.surface_id,
                        action_ref.interaction_id()
                    )
                },
            )?;
        }
        Ok(())
    }

    fn validate_tabs(&self, tabs: &[crate::SurfaceTab]) -> Result<(), SurfaceRegistrationError> {
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
        declared_method: Option<&InteractionHttpMethod>,
    ) -> Result<(), SurfaceRegistrationError> {
        resolve_interaction_reference(
            self.interaction_methods,
            interaction_id,
            declared_method,
            || {
                format!(
                    "surface `{}` root_node references unknown interaction_id `{}`",
                    self.surface_id, interaction_id
                )
            },
        )
    }

    fn require_root_workflow_trigger_reference(
        &self,
        interaction_id: &str,
    ) -> Result<(), SurfaceRegistrationError> {
        require_pair_reference(
            self.interaction_methods,
            interaction_id,
            InteractionHttpMethod::Post,
            || {
                format!(
                    "surface `{}` root_node references unknown interaction_id `{}`",
                    self.surface_id, interaction_id
                )
            },
        )
    }

    /// Root-level `Form` `form_ui.pre_load_interaction_id` (NEW — previously
    /// unvalidated; workflow-step forms already had this check). Looks up the
    /// target interaction by `(id, declared_method)` to find its `form_ui`
    /// since `SurfaceNode::Form` itself carries no `form_ui` field.
    fn require_form_pre_load_reference(
        &self,
        interaction_id: &str,
        declared_method: Option<&InteractionHttpMethod>,
    ) -> Result<(), SurfaceRegistrationError> {
        // `declared_method: None` matches the first interaction with this id
        // regardless of method, i.e. first-match-wins. That's only safe
        // because `require_root_interaction_reference` runs first (in the
        // caller chain) and already rejects an ambiguous reference — an id
        // with `declared_method: None` that resolves to more than one
        // registered method. Do not reorder validation so this lookup runs
        // before that check: it would silently pick an arbitrary method's
        // `form_ui` instead of erroring on the ambiguity.
        let Some(target) = self.interactions.iter().find(|interaction| {
            interaction.interaction_id.as_str() == interaction_id
                && declared_method
                    .map(|method| interaction.effective_http_method() == *method)
                    .unwrap_or(true)
        }) else {
            // Unknown/ambiguous id already reported by require_root_interaction_reference.
            return Ok(());
        };
        let Some(pre_load_interaction_id) = target
            .form_ui
            .as_ref()
            .and_then(|form_ui| form_ui.pre_load_interaction_id.as_ref())
        else {
            return Ok(());
        };

        require_pair_reference(
            self.interaction_methods,
            pre_load_interaction_id.as_str(),
            InteractionHttpMethod::Get,
            || {
                format!(
                    "surface `{}` root_node form `{}` references unknown pre_load_interaction_id `{}`",
                    self.surface_id, interaction_id, pre_load_interaction_id
                )
            },
        )
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
}

fn validate_workflow_step_references(
    surface_id: &SurfaceId,
    interactions: &[InteractionDescriptor],
    interaction_methods: &HashMap<&str, Vec<InteractionHttpMethod>>,
) -> Result<(), SurfaceRegistrationError> {
    WorkflowStepReferenceValidator::new(surface_id, interaction_methods).validate(interactions)
}

struct WorkflowStepReferenceValidator<'a> {
    surface_id: &'a SurfaceId,
    interaction_methods: &'a HashMap<&'a str, Vec<InteractionHttpMethod>>,
}

impl<'a> WorkflowStepReferenceValidator<'a> {
    fn new(
        surface_id: &'a SurfaceId,
        interaction_methods: &'a HashMap<&'a str, Vec<InteractionHttpMethod>>,
    ) -> Self {
        Self {
            surface_id,
            interaction_methods,
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
                require_pair_reference(
                    self.interaction_methods,
                    submit_interaction_id.as_str(),
                    InteractionHttpMethod::Post,
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
                require_pair_reference(
                    self.interaction_methods,
                    pre_load_interaction_id.as_str(),
                    InteractionHttpMethod::Get,
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

/// Resolves a bare/explicit interaction reference against the `(id, method)`
/// index (REST method model, spec §2a). A `declared_method` (explicit
/// `(id, method)` reference) must be registered exactly; a bare reference
/// (`declared_method: None`) resolves only when the id registers under
/// exactly one method — no POST fallback, fail-closed on ambiguity.
fn resolve_interaction_reference(
    interaction_methods: &HashMap<&str, Vec<InteractionHttpMethod>>,
    reference_id: &str,
    declared_method: Option<&InteractionHttpMethod>,
    error_context: impl FnOnce() -> String,
) -> Result<(), SurfaceRegistrationError> {
    let Some(methods) = interaction_methods.get(reference_id) else {
        return Err(invalid_contract(error_context()));
    };
    match declared_method {
        Some(method) if !methods.contains(method) => Err(invalid_contract(format!(
            "{} — `{reference_id}` is not registered under method `{method}`",
            error_context()
        ))),
        // No Post fallback: a bare reference to a multi-method ID must fail
        // closed (a default would silently resolve a delete-intent reference
        // to a registered create sibling).
        None if methods.len() > 1 => Err(invalid_contract(format!(
            "{} — `{reference_id}` is registered under multiple methods; declare http_method — ambiguous",
            error_context()
        ))),
        _ => Ok(()),
    }
}

/// Resolves a reference that must land under one exact required method
/// (workflow triggers/submits are POST-fixed; pre-load/`ProviderQuery`
/// sources are GET-fixed) — used where the node/field carries no explicit
/// `http_method` of its own to disambiguate with.
fn require_pair_reference(
    interaction_methods: &HashMap<&str, Vec<InteractionHttpMethod>>,
    reference_id: &str,
    required: InteractionHttpMethod,
    error_context: impl FnOnce() -> String,
) -> Result<(), SurfaceRegistrationError> {
    match interaction_methods.get(reference_id) {
        Some(methods) if methods.contains(&required) => Ok(()),
        _ => Err(invalid_contract(format!(
            "{} — `{reference_id}` must be registered under method `{required}`",
            error_context()
        ))),
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

wire_safe_enum! {
    /// Encryption algorithm used for ECIES sealed-box parameter encryption.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum ProviderEncryptionAlgorithm {
        EciesP256 => "ecies_p256",
    }
    parse_error = ParseProviderEncryptionAlgorithmError("invalid provider encryption algorithm");
}

impl ProviderEncryptionAlgorithm {
    /// All known (non-`Other`) variants. Used in tests for exhaustive iteration
    /// (`strum::EnumIter` is incompatible with the `Other(String)` tuple variant).
    pub const KNOWN_VARIANTS: &'static [ProviderEncryptionAlgorithm] =
        &[ProviderEncryptionAlgorithm::EciesP256];
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
    /// Dispatch method (REST method model). Old controllers never set it;
    /// old services drop it — default POST both ways.
    #[serde(default)]
    pub method: InteractionHttpMethod,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_action_request_without_method_deserializes_to_post() {
        // Old-peer wire shape (no `method` key).
        let json = serde_json::json!({
            "request_id": "018f0000-0000-7000-8000-000000000000",
            "tenant_id": "018f0000-0000-7000-8000-000000000001",
            "surface_id": "test.surface",
            "interaction_id": "save",
            "idempotency_key": "k1",
            "caller_origin": { "kind": "built_in_system", "principal": "test" }
        });
        let request: SurfaceActionRequest = serde_json::from_value(json).expect("old-peer shape");
        assert_eq!(request.method, InteractionHttpMethod::Post);
    }
}
