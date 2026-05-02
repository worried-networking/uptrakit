use serde::{Deserialize, Serialize};

use crate::SurfaceId;

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Targeting {
    Universal,
    Targeted,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Global,
    Tenant,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    BuiltIn,
    Plugin,
    Service,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SurfaceNode {
    Section {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        children: Vec<SurfaceNode>,
    },
    TextBlock {
        text: String,
    },
    KeyValue {
        data_source_id: crate::DataSourceId,
    },
    Table {
        data_source_id: crate::DataSourceId,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        columns: Vec<SurfaceTableColumn>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        row_actions: Vec<SurfaceTableRowAction>,
    },
    Form {
        interaction_id: crate::InteractionId,
    },
    ActionBar {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        action_ids: Vec<crate::InteractionId>,
    },
    Tabs {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tabs: Vec<SurfaceTab>,
    },
    Callout {
        level: CalloutLevel,
        text: String,
    },
    EmptyState {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    ModalTrigger {
        interaction_id: crate::InteractionId,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        modal_nodes: Vec<SurfaceNode>,
    },
    WorkflowTrigger {
        interaction_id: crate::InteractionId,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        step_nodes: Vec<SurfaceNode>,
    },
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceTableColumn {
    pub key: String,
    pub label: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_cell_type"
    )]
    pub cell_type: Option<SurfaceTableCellType>,
}

impl SurfaceTableColumn {
    /// Creates a new column with no cell type (plain text rendering).
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            cell_type: None,
        }
    }
}

fn deserialize_optional_cell_type<'de, D>(
    deserializer: D,
) -> Result<Option<SurfaceTableCellType>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

/// Cell type for a surface table column.
///
/// Forward compatibility: unknown `kind` values deserialize to `None` via
/// [`deserialize_optional_cell_type`] rather than `Other(String)`, because
/// a completely unknown cell type has no meaningful rendering — silently
/// treating it as a plain-text column is safer than propagating an opaque value.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SurfaceTableCellType {
    EntityLink { entity_type: SurfaceEntityType },
}

/// Wire-safe entity type enum.
///
/// Known variants are type-safe; unknown values from newer peers become
/// `Other(String)` for forward compatibility. Uses custom `Serialize`
/// and `Deserialize` so that `Other(String)` emits a bare string on
/// the wire (not `{"other":"..."}`).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SurfaceEntityType {
    Host,
    Other(String),
}

impl SurfaceEntityType {
    /// Returns the snake_case wire string for this entity type.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Host => "host",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl From<String> for SurfaceEntityType {
    fn from(s: String) -> Self {
        match s.as_str() {
            "host" => Self::Host,
            _ => Self::Other(s),
        }
    }
}

impl Serialize for SurfaceEntityType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SurfaceEntityType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(SurfaceEntityType::from)
    }
}

/// Cell value for entity-link columns.
///
/// Plugins construct via [`SurfaceEntityRef::unresolved`] (`entity_id` only).
/// The framework enriches `label` and `found` before sending the wire response.
/// `found: None` is a transient pre-enrichment state — must not appear in the
/// final wire response for cells whose resolver ran.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceEntityRef {
    pub entity_id: uuid::Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<bool>,
}

impl SurfaceEntityRef {
    /// Constructs an unresolved ref for use by plugin handlers.
    /// The framework enriches `label` and `found` in the enrichment step.
    pub fn unresolved(entity_id: uuid::Uuid) -> Self {
        Self {
            entity_id,
            label: None,
            found: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceTableRowAction {
    pub interaction_id: crate::InteractionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<SurfaceRowVisibleWhen>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceRowVisibleWhen {
    pub field: String,
    pub condition: SurfaceRowCondition,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceRowCondition {
    Present,
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceTab {
    pub id: crate::SurfaceTabId,
    pub label: String,
    pub root: SurfaceNode,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalloutLevel {
    Info,
    Warning,
    Danger,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceDescriptor {
    pub surface_id: SurfaceId,
    pub label: String,
    pub priority: i32,
    pub slot: String,
    pub scope: Scope,
    pub targeting: Targeting,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_permission: Option<String>,
    pub provider_kind: ProviderKind,
    pub required_capabilities: CapabilitySet,
    pub root_node: SurfaceNode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_selector: Option<SurfaceContextSelectorDescriptor>,
}

impl SurfaceDescriptor {
    /// Returns a zero-arg [`SurfaceDescriptorBuilder`] for constructing a [`SurfaceDescriptor`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use uptrakit_surfaces::{
    ///     CapabilitySet, ProviderKind, Scope, SurfaceDescriptor, SurfaceId, SurfaceNode, Targeting,
    /// };
    ///
    /// let descriptor = SurfaceDescriptor::builder()
    ///     .surface_id(SurfaceId::new("provider.sample.surface").unwrap())
    ///     .label("Sample")
    ///     .priority(200)
    ///     .slot("surface.page")
    ///     .scope(Scope::Tenant)
    ///     .targeting(Targeting::Universal)
    ///     .provider_kind(ProviderKind::Plugin)
    ///     .required_capabilities(CapabilitySet::default())
    ///     .root_node(SurfaceNode::Section { title: None, children: vec![] })
    ///     .build();
    /// ```
    #[must_use]
    pub fn builder() -> SurfaceDescriptorBuilder {
        SurfaceDescriptorBuilder::default()
    }
}

/// Builder for [`SurfaceDescriptor`].
///
/// Obtain an instance via [`SurfaceDescriptor::builder`] and call [`build`](Self::build) to
/// finalise the descriptor. Optional fields ([`required_permission`](Self::required_permission)
/// and [`context_selector`](Self::context_selector)) default to `None`.
///
/// [`build`](Self::build) panics if any required field has not been set.
#[derive(Debug, Clone, Default)]
pub struct SurfaceDescriptorBuilder {
    surface_id: Option<SurfaceId>,
    label: Option<String>,
    priority: Option<i32>,
    slot: Option<String>,
    scope: Option<Scope>,
    targeting: Option<Targeting>,
    required_permission: Option<String>,
    provider_kind: Option<ProviderKind>,
    required_capabilities: Option<CapabilitySet>,
    root_node: Option<SurfaceNode>,
    context_selector: Option<SurfaceContextSelectorDescriptor>,
}

impl SurfaceDescriptorBuilder {
    /// Sets the surface identifier.
    #[must_use]
    pub fn surface_id(mut self, surface_id: SurfaceId) -> Self {
        self.surface_id = Some(surface_id);
        self
    }

    /// Sets the human-readable label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Sets the display priority within the slot.
    #[must_use]
    pub fn priority(mut self, priority: i32) -> Self {
        self.priority = Some(priority);
        self
    }

    /// Sets the slot identifier (e.g. `"surface.page"`).
    #[must_use]
    pub fn slot(mut self, slot: impl Into<String>) -> Self {
        self.slot = Some(slot.into());
        self
    }

    /// Sets the scope (global or tenant).
    #[must_use]
    pub fn scope(mut self, scope: Scope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Sets the targeting mode.
    #[must_use]
    pub fn targeting(mut self, targeting: Targeting) -> Self {
        self.targeting = Some(targeting);
        self
    }

    /// Sets the permission string required to view this surface (optional).
    #[must_use]
    pub fn required_permission(mut self, permission: impl Into<String>) -> Self {
        self.required_permission = Some(permission.into());
        self
    }

    /// Sets the provider kind.
    #[must_use]
    pub fn provider_kind(mut self, provider_kind: ProviderKind) -> Self {
        self.provider_kind = Some(provider_kind);
        self
    }

    /// Sets the set of capabilities this surface requires from the framework.
    #[must_use]
    pub fn required_capabilities(mut self, required_capabilities: CapabilitySet) -> Self {
        self.required_capabilities = Some(required_capabilities);
        self
    }

    /// Sets the root [`SurfaceNode`] of the surface layout.
    #[must_use]
    pub fn root_node(mut self, root_node: SurfaceNode) -> Self {
        self.root_node = Some(root_node);
        self
    }

    /// Attaches a context-selector dropdown descriptor to the surface (optional).
    #[must_use]
    pub fn context_selector(mut self, context_selector: SurfaceContextSelectorDescriptor) -> Self {
        self.context_selector = Some(context_selector);
        self
    }

    /// Consumes the builder and returns the [`SurfaceDescriptor`].
    ///
    /// # Panics
    ///
    /// Panics if any required field (`surface_id`, `label`, `priority`, `slot`, `scope`,
    /// `targeting`, `provider_kind`, `required_capabilities`, `root_node`) has not been set.
    #[must_use]
    pub fn build(self) -> SurfaceDescriptor {
        SurfaceDescriptor {
            surface_id: self
                .surface_id
                .expect("SurfaceDescriptorBuilder: surface_id not set"),
            label: self.label.expect("SurfaceDescriptorBuilder: label not set"),
            priority: self
                .priority
                .expect("SurfaceDescriptorBuilder: priority not set"),
            slot: self.slot.expect("SurfaceDescriptorBuilder: slot not set"),
            scope: self.scope.expect("SurfaceDescriptorBuilder: scope not set"),
            targeting: self
                .targeting
                .expect("SurfaceDescriptorBuilder: targeting not set"),
            required_permission: self.required_permission,
            provider_kind: self
                .provider_kind
                .expect("SurfaceDescriptorBuilder: provider_kind not set"),
            required_capabilities: self
                .required_capabilities
                .expect("SurfaceDescriptorBuilder: required_capabilities not set"),
            root_node: self
                .root_node
                .expect("SurfaceDescriptorBuilder: root_node not set"),
            context_selector: self.context_selector,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FrameworkGeneration {
    pub major: u16,
    pub minor: u16,
}

impl FrameworkGeneration {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkGenerationRange {
    pub min: FrameworkGeneration,
    pub max: FrameworkGeneration,
}

impl FrameworkGenerationRange {
    #[must_use]
    pub const fn includes(&self, value: FrameworkGeneration) -> bool {
        is_generation_le(self.min, value) && is_generation_le(value, self.max)
    }
}

const fn is_generation_le(left: FrameworkGeneration, right: FrameworkGeneration) -> bool {
    left.major < right.major || (left.major == right.major && left.minor <= right.minor)
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    SectionNode,
    TextBlockNode,
    KeyValueNode,
    TableNode,
    FormNode,
    ActionBarNode,
    TabsNode,
    CalloutNode,
    EmptyStateNode,
    ModalTriggerNode,
    WorkflowTriggerNode,
    MutationAction,
    FormSubmit,
    Workflow,
    Navigate,
    DataLoad,
    ConfirmableAction,
    StaticDataSource,
    ControllerQueryDataSource,
    ProviderQueryDataSource,
    UniversalTargeting,
    TargetedTargeting,
    SensitiveFields,
    ProviderInitiatedActions,
    ContextSelector,
    EntityLinkColumn,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilitySet(pub std::collections::BTreeSet<Capability>);

impl CapabilitySet {
    #[must_use]
    pub fn from_capabilities(caps: impl IntoIterator<Item = Capability>) -> Self {
        Self(caps.into_iter().collect())
    }

    #[must_use]
    pub fn contains_all(&self, other: &Self) -> bool {
        other.0.iter().all(|cap| self.0.contains(cap))
    }
}

/// Describes a context-selector dropdown rendered above a surface's content.
///
/// When present on a `SurfaceDescriptor`, `SurfaceReadPanel` fetches the
/// options from `rest_api_path` and renders a `ProviderSelector` above the
/// surface content. The selected value is merged into `baseParams` under
/// `param_key`, driving both the table data load and optional interaction gates.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceContextSelectorDescriptor {
    /// Param key injected into `baseParams` when a specific option is selected.
    pub param_key: String,
    /// Label shown above the selector dropdown.
    pub label: String,
    /// Label for the "show all" option (no param injected).
    pub all_option_label: String,
    /// REST API path returning a JSON array or paginated `items` list.
    pub rest_api_path: String,
    /// Field in each item used as the option value.
    pub value_field: String,
    /// Field in each item used as the option label.
    pub label_field: String,
    /// Interaction IDs disabled (with tooltip) when no specific option is selected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_for_interactions: Vec<crate::InteractionId>,
}

impl SurfaceContextSelectorDescriptor {
    /// Constructs a new [`SurfaceContextSelectorDescriptor`].
    ///
    /// Required because the struct is `#[non_exhaustive]` — external crates cannot use
    /// struct literal syntax and must call this constructor instead.
    #[must_use]
    pub fn new(
        param_key: impl Into<String>,
        label: impl Into<String>,
        all_option_label: impl Into<String>,
        rest_api_path: impl Into<String>,
        value_field: impl Into<String>,
        label_field: impl Into<String>,
        required_for_interactions: Vec<crate::InteractionId>,
    ) -> Self {
        Self {
            param_key: param_key.into(),
            label: label.into(),
            all_option_label: all_option_label.into(),
            rest_api_path: rest_api_path.into(),
            value_field: value_field.into(),
            label_field: label_field.into(),
            required_for_interactions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_selector_capability_serializes_to_snake_case() {
        let cap = Capability::ContextSelector;
        let serialized = serde_json::to_string(&cap).expect("serialize");
        assert_eq!(serialized, r#""context_selector""#);
    }

    #[test]
    fn surface_descriptor_context_selector_round_trips() {
        let descriptor = SurfaceDescriptor {
            surface_id: SurfaceId::new("test.surface").unwrap(),
            label: "Test".to_string(),
            priority: 100,
            slot: "surface.page".to_string(),
            scope: Scope::Global,
            targeting: Targeting::Universal,
            required_permission: None,
            provider_kind: ProviderKind::Plugin,
            required_capabilities: CapabilitySet::from_capabilities([Capability::ContextSelector]),
            root_node: SurfaceNode::Section {
                title: None,
                children: vec![],
            },
            context_selector: Some(SurfaceContextSelectorDescriptor {
                param_key: "plugin_config_id".to_string(),
                label: "Configuration".to_string(),
                all_option_label: "All Configurations".to_string(),
                rest_api_path: "/api/v1/plugin-configs".to_string(),
                value_field: "id".to_string(),
                label_field: "name".to_string(),
                required_for_interactions: vec![crate::InteractionId::new("discover").unwrap()],
            }),
        };

        let json = serde_json::to_string(&descriptor).expect("serialize");
        let deserialized: SurfaceDescriptor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(descriptor, deserialized);

        let context_selector = deserialized.context_selector.unwrap();
        assert_eq!(context_selector.param_key, "plugin_config_id");
        assert_eq!(
            context_selector.required_for_interactions,
            vec![crate::InteractionId::new("discover").unwrap()]
        );
    }

    #[test]
    fn surface_descriptor_without_context_selector_omits_field_in_json() {
        let descriptor = SurfaceDescriptor {
            surface_id: SurfaceId::new("test.surface").unwrap(),
            label: "Test".to_string(),
            priority: 100,
            slot: "surface.page".to_string(),
            scope: Scope::Global,
            targeting: Targeting::Universal,
            required_permission: None,
            provider_kind: ProviderKind::Plugin,
            required_capabilities: CapabilitySet::default(),
            root_node: SurfaceNode::Section {
                title: None,
                children: vec![],
            },
            context_selector: None,
        };

        let json = serde_json::to_string(&descriptor).expect("serialize");
        assert!(
            !json.contains("context_selector"),
            "absent context_selector must be omitted from JSON"
        );
    }

    #[test]
    fn surface_table_cell_type_entity_link_serializes_correctly() {
        let mut col = SurfaceTableColumn::new("host", "Host");
        col.cell_type = Some(SurfaceTableCellType::EntityLink {
            entity_type: SurfaceEntityType::Host,
        });
        let json = serde_json::to_string(&col).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["cell_type"]["kind"], "entity_link");
        assert_eq!(parsed["cell_type"]["entity_type"], "host");
    }

    #[test]
    fn surface_table_column_without_cell_type_omits_field() {
        let col = SurfaceTableColumn::new("name", "Name");
        let json = serde_json::to_string(&col).expect("serialize");
        assert!(!json.contains("cell_type"));
    }

    #[test]
    fn unknown_cell_type_deserializes_to_none() {
        let json =
            r#"{"key":"host","label":"Host","cell_type":{"kind":"future_type","extra":"data"}}"#;
        let col: SurfaceTableColumn = serde_json::from_str(json).expect("deserialize");
        assert!(col.cell_type.is_none());
    }

    #[test]
    fn surface_entity_type_host_serializes_to_bare_string() {
        let t = SurfaceEntityType::Host;
        let s = serde_json::to_string(&t).expect("serialize");
        assert_eq!(s, r#""host""#);
    }

    #[test]
    fn surface_entity_type_other_serializes_to_bare_string() {
        let t = SurfaceEntityType::Other("my_future_type".to_string());
        let s = serde_json::to_string(&t).expect("serialize");
        assert_eq!(s, r#""my_future_type""#);
    }

    #[test]
    fn surface_entity_type_unknown_string_deserializes_to_other() {
        let t: SurfaceEntityType = serde_json::from_str(r#""unknown_type""#).expect("deserialize");
        assert_eq!(t, SurfaceEntityType::Other("unknown_type".to_string()));
    }

    #[test]
    fn surface_entity_ref_unresolved_serializes_without_label_or_found() {
        let entity_id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let r = SurfaceEntityRef::unresolved(entity_id);
        let json = serde_json::to_string(&r).expect("serialize");
        let val: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(val["entity_id"], entity_id.to_string());
        assert!(val.get("label").is_none());
        assert!(val.get("found").is_none());
    }

    #[test]
    fn entity_link_column_capability_serializes_to_snake_case() {
        let cap = Capability::EntityLinkColumn;
        let s = serde_json::to_string(&cap).expect("serialize");
        assert_eq!(s, r#""entity_link_column""#);
    }
}
