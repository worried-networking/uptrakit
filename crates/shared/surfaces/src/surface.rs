use serde::{Deserialize, Serialize};

use crate::SurfaceId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Targeting {
    Universal,
    Targeted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Global,
    Tenant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    BuiltIn,
    Plugin,
    Service,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceTab {
    pub id: String,
    pub label: String,
    pub root: SurfaceNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalloutLevel {
    Info,
    Warning,
    Danger,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkGenerationRange {
    pub min: FrameworkGeneration,
    pub max: FrameworkGeneration,
}

impl FrameworkGenerationRange {
    pub fn includes(&self, value: FrameworkGeneration) -> bool {
        value >= self.min && value <= self.max
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilitySet(pub std::collections::BTreeSet<Capability>);

impl CapabilitySet {
    pub fn from_capabilities(caps: impl IntoIterator<Item = Capability>) -> Self {
        Self(caps.into_iter().collect())
    }

    pub fn contains_all(&self, other: &Self) -> bool {
        other.0.iter().all(|cap| self.0.contains(cap))
    }
}
