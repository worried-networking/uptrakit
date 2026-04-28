use std::collections::BTreeSet;

use uptrakit_wire::surfaces::{
    self, CapabilitySet, DataSourceDescriptor, DataSourceKind, InteractionDescriptor,
    InteractionKind, InteractionTransport, SurfaceNode, Targeting,
};

pub(super) fn compute_required_capabilities(
    root_node: &SurfaceNode,
    targeting: &Targeting,
    interactions: &[InteractionDescriptor],
    data_sources: &[DataSourceDescriptor],
) -> CapabilitySet {
    let mut caps = BTreeSet::new();
    collect_node_caps(root_node, &mut caps);
    for interaction in interactions {
        match interaction.kind {
            InteractionKind::MutationAction => {
                caps.insert(surfaces::Capability::MutationAction);
            }
            InteractionKind::FormSubmit => {
                caps.insert(surfaces::Capability::FormSubmit);
            }
            InteractionKind::Workflow => {
                caps.insert(surfaces::Capability::Workflow);
            }
            InteractionKind::Navigate => {
                caps.insert(surfaces::Capability::Navigate);
            }
            InteractionKind::DataLoad => {
                caps.insert(surfaces::Capability::DataLoad);
            }
            InteractionKind::ConfirmableAction => {
                caps.insert(surfaces::Capability::ConfirmableAction);
            }
        }
        if !interaction.sensitive_fields.is_empty() {
            caps.insert(surfaces::Capability::SensitiveFields);
        }
        if matches!(interaction.transport, InteractionTransport::ProviderProxied) {
            caps.insert(surfaces::Capability::ProviderInitiatedActions);
        }
    }
    for data_source in data_sources {
        match data_source.kind {
            DataSourceKind::Static { .. } => {
                caps.insert(surfaces::Capability::StaticDataSource);
            }
            DataSourceKind::ControllerQuery { .. } => {
                caps.insert(surfaces::Capability::ControllerQueryDataSource);
            }
            DataSourceKind::ProviderQuery { .. } => {
                caps.insert(surfaces::Capability::ProviderQueryDataSource);
            }
        }
    }
    match targeting {
        Targeting::Universal => {
            caps.insert(surfaces::Capability::UniversalTargeting);
        }
        Targeting::Targeted => {
            caps.insert(surfaces::Capability::TargetedTargeting);
        }
        _ => {
            tracing::warn!(
                ?targeting,
                "unknown Targeting variant; defaulting to UniversalTargeting capability"
            );
            caps.insert(surfaces::Capability::UniversalTargeting);
        }
    }
    CapabilitySet(caps)
}

fn collect_node_caps(node: &SurfaceNode, caps: &mut BTreeSet<surfaces::Capability>) {
    match node {
        SurfaceNode::Section { children, .. } => {
            caps.insert(surfaces::Capability::SectionNode);
            for child in children {
                collect_node_caps(child, caps);
            }
        }
        SurfaceNode::TextBlock { .. } => {
            caps.insert(surfaces::Capability::TextBlockNode);
        }
        SurfaceNode::KeyValue { .. } => {
            caps.insert(surfaces::Capability::KeyValueNode);
        }
        SurfaceNode::Table { .. } => {
            caps.insert(surfaces::Capability::TableNode);
        }
        SurfaceNode::Form { .. } => {
            caps.insert(surfaces::Capability::FormNode);
        }
        SurfaceNode::ActionBar { .. } => {
            caps.insert(surfaces::Capability::ActionBarNode);
        }
        SurfaceNode::Tabs { tabs } => {
            caps.insert(surfaces::Capability::TabsNode);
            for tab in tabs {
                collect_node_caps(&tab.root, caps);
            }
        }
        SurfaceNode::Callout { .. } => {
            caps.insert(surfaces::Capability::CalloutNode);
        }
        SurfaceNode::EmptyState { .. } => {
            caps.insert(surfaces::Capability::EmptyStateNode);
        }
        SurfaceNode::ModalTrigger { modal_nodes, .. } => {
            caps.insert(surfaces::Capability::ModalTriggerNode);
            for modal in modal_nodes {
                collect_node_caps(modal, caps);
            }
        }
        SurfaceNode::WorkflowTrigger { step_nodes, .. } => {
            caps.insert(surfaces::Capability::WorkflowTriggerNode);
            for step in step_nodes {
                collect_node_caps(step, caps);
            }
        }
        _ => {
            tracing::warn!(?node, "unknown SurfaceNode variant; no capability inserted");
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uptrakit_wire::surfaces::{
        self, ControllerQueryId, DataSourceDescriptor, DataSourceId, DataSourceKind,
        InteractionDescriptor, InteractionId, InteractionKind, InteractionTransport, RefreshPolicy,
        SchemaContract, SurfaceNode, Targeting,
    };

    use super::compute_required_capabilities;

    fn make_interaction(
        interaction_id: &str,
        kind: InteractionKind,
        transport: InteractionTransport,
        sensitive_fields: Vec<String>,
    ) -> InteractionDescriptor {
        InteractionDescriptor {
            interaction_id: InteractionId::new(interaction_id).expect("valid interaction id"),
            kind,
            label: None,
            required_permission: None,
            input_schema: Some(SchemaContract::Object),
            result_schema: Some(SchemaContract::Any),
            sensitive_fields,
            timeout_seconds: None,
            confirmation: None,
            transport,
            workflow_steps: vec![],
            form_ui: None,
        }
    }

    fn make_data_source(kind: DataSourceKind) -> DataSourceDescriptor {
        DataSourceDescriptor {
            data_source_id: DataSourceId::new("data.test").expect("valid data source id"),
            kind,
            result_schema: SchemaContract::Any,
            pagination: None,
            sorting: None,
            filtering: None,
            refresh_policy: RefreshPolicy::Manual,
            empty_state: None,
        }
    }

    #[test]
    fn section_table_action_bar_tree_adds_node_capabilities() {
        let data_source_id = DataSourceId::new("data.primary").expect("valid data source id");
        let action_id = InteractionId::new("action.primary").expect("valid interaction id");
        let root = SurfaceNode::Section {
            title: None,
            children: vec![
                SurfaceNode::Table {
                    data_source_id,
                    columns: vec![],
                    row_actions: vec![],
                },
                SurfaceNode::ActionBar {
                    action_ids: vec![action_id],
                },
            ],
        };

        let caps = compute_required_capabilities(&root, &Targeting::Universal, &[], &[]);

        assert!(caps.0.contains(&surfaces::Capability::SectionNode));
        assert!(caps.0.contains(&surfaces::Capability::TableNode));
        assert!(caps.0.contains(&surfaces::Capability::ActionBarNode));
    }

    #[test]
    fn interactions_and_provider_proxied_transport_add_capabilities() {
        let root = SurfaceNode::Section {
            title: None,
            children: vec![],
        };
        let interactions = vec![
            make_interaction(
                "interaction.mutation",
                InteractionKind::MutationAction,
                InteractionTransport::ProviderProxied,
                vec!["secret".to_string()],
            ),
            make_interaction(
                "interaction.form",
                InteractionKind::FormSubmit,
                InteractionTransport::ControllerLocal,
                vec![],
            ),
            make_interaction(
                "interaction.workflow",
                InteractionKind::Workflow,
                InteractionTransport::ControllerLocal,
                vec![],
            ),
            make_interaction(
                "interaction.navigate",
                InteractionKind::Navigate,
                InteractionTransport::ControllerLocal,
                vec![],
            ),
            make_interaction(
                "interaction.load",
                InteractionKind::DataLoad,
                InteractionTransport::ControllerLocal,
                vec![],
            ),
            make_interaction(
                "interaction.confirmable",
                InteractionKind::ConfirmableAction,
                InteractionTransport::ControllerLocal,
                vec![],
            ),
        ];

        let caps = compute_required_capabilities(&root, &Targeting::Universal, &interactions, &[]);

        assert!(caps.0.contains(&surfaces::Capability::MutationAction));
        assert!(caps.0.contains(&surfaces::Capability::FormSubmit));
        assert!(caps.0.contains(&surfaces::Capability::Workflow));
        assert!(caps.0.contains(&surfaces::Capability::Navigate));
        assert!(caps.0.contains(&surfaces::Capability::DataLoad));
        assert!(caps.0.contains(&surfaces::Capability::ConfirmableAction));
        assert!(caps.0.contains(&surfaces::Capability::SensitiveFields));
        assert!(
            caps.0
                .contains(&surfaces::Capability::ProviderInitiatedActions)
        );
    }

    #[test]
    fn data_source_kinds_add_expected_data_source_capabilities() {
        let root = SurfaceNode::Section {
            title: None,
            children: vec![],
        };
        let data_sources = vec![
            make_data_source(DataSourceKind::Static { data: json!({}) }),
            make_data_source(DataSourceKind::ControllerQuery {
                query_id: ControllerQueryId::new("controller.query")
                    .expect("valid controller query id"),
            }),
            make_data_source(DataSourceKind::ProviderQuery {
                operation_id: "load-data".to_string(),
            }),
        ];

        let caps = compute_required_capabilities(&root, &Targeting::Universal, &[], &data_sources);

        assert!(caps.0.contains(&surfaces::Capability::StaticDataSource));
        assert!(
            caps.0
                .contains(&surfaces::Capability::ControllerQueryDataSource)
        );
        assert!(
            caps.0
                .contains(&surfaces::Capability::ProviderQueryDataSource)
        );
    }

    #[test]
    fn targeting_adds_matching_targeting_capability() {
        let root = SurfaceNode::Section {
            title: None,
            children: vec![],
        };

        let universal_caps = compute_required_capabilities(&root, &Targeting::Universal, &[], &[]);
        assert!(
            universal_caps
                .0
                .contains(&surfaces::Capability::UniversalTargeting)
        );
        assert!(
            !universal_caps
                .0
                .contains(&surfaces::Capability::TargetedTargeting)
        );

        let targeted_caps = compute_required_capabilities(&root, &Targeting::Targeted, &[], &[]);
        assert!(
            targeted_caps
                .0
                .contains(&surfaces::Capability::TargetedTargeting)
        );
        assert!(
            !targeted_caps
                .0
                .contains(&surfaces::Capability::UniversalTargeting)
        );
    }
}
