use serde_json::{Map, Value, json};
use uuid::Uuid;

use uptrakit_surfaces::{
    BuiltInApiOperationId, CallerOrigin, Capability, CapabilitySet, ControllerQueryId,
    DataSourceDescriptor, DataSourceEmptyState, DataSourceFiltering, DataSourceId, DataSourceKind,
    DataSourcePagination, DataSourceSorting, EffectiveTenantBinding, EncryptedSensitiveParams,
    FrameworkGeneration, FrameworkGenerationRange, InteractionConfirmation, InteractionDescriptor,
    InteractionId, InteractionKind, InteractionTransport, MIN_PROVIDER_REFRESH_INTERVAL_SECONDS,
    ProviderIdentity, ProviderKind, RefreshPolicy, RegisteredSurface, SLOT_EXTENSION_PAGE,
    SLOT_SETTINGS_TABS, SchemaContract, Scope, SurfaceActionRequest, SurfaceDescriptor, SurfaceId,
    SurfaceNode, SurfaceRegistration, SurfaceRegistrationErrorCode, SurfaceRegistrationPolicy,
    SurfaceTab, SurfaceTabId, Targeting,
};

fn registration_policy(required_capabilities: CapabilitySet) -> SurfaceRegistrationPolicy {
    SurfaceRegistrationPolicy {
        supported_generation: FrameworkGenerationRange {
            min: FrameworkGeneration::new(1, 0),
            max: FrameworkGeneration::new(1, 2),
        },
        required_capabilities,
    }
}

fn minimal_surface(provider_kind: ProviderKind) -> RegisteredSurface {
    RegisteredSurface {
        descriptor: SurfaceDescriptor {
            surface_id: SurfaceId::new("provider.sample.surface").expect("valid surface id"),
            label: "Sample".to_string(),
            priority: 200,
            slot: SLOT_SETTINGS_TABS.to_string(),
            scope: Scope::Tenant,
            targeting: Targeting::Universal,
            required_permission: None,
            provider_kind,
            required_capabilities: CapabilitySet::from_capabilities([Capability::SectionNode]),
            root_node: SurfaceNode::Section {
                title: Some("Sample".to_string()),
                children: Vec::new(),
            },
        },
        interactions: vec![InteractionDescriptor {
            interaction_id: InteractionId::new("surface.refresh").expect("valid interaction id"),
            kind: InteractionKind::DataLoad,
            label: Some("Refresh".to_string()),
            required_permission: None,
            input_schema: Some(SchemaContract::Any),
            result_schema: Some(SchemaContract::Any),
            sensitive_fields: Vec::new(),
            timeout_seconds: Some(30),
            confirmation: None,
            transport: InteractionTransport::ProviderProxied,
            workflow_steps: Vec::new(),
            form_ui: None,
        }],
        data_sources: vec![DataSourceDescriptor {
            data_source_id: DataSourceId::new("surface.data").expect("valid data source id"),
            kind: DataSourceKind::ProviderQuery {
                operation_id: "list".to_string(),
            },
            result_schema: SchemaContract::Object,
            pagination: Some(DataSourcePagination {
                default_page_size: 25,
                max_page_size: 200,
            }),
            sorting: Some(DataSourceSorting {
                sortable_fields: vec!["name".to_string()],
                default_sort_field: Some("name".to_string()),
            }),
            filtering: Some(DataSourceFiltering {
                filter_fields: vec!["status".to_string()],
            }),
            refresh_policy: RefreshPolicy::Interval {
                seconds: MIN_PROVIDER_REFRESH_INTERVAL_SECONDS,
            },
            empty_state: Some(DataSourceEmptyState {
                title: "No rows".to_string(),
                description: Some("Nothing to show yet".to_string()),
            }),
        }],
    }
}

fn minimal_registration(provider_kind: ProviderKind) -> SurfaceRegistration {
    SurfaceRegistration {
        provider: ProviderIdentity {
            provider_id: "provider-1".to_string(),
            provider_kind: provider_kind.clone(),
            provider_namespace: "provider.sample".to_string(),
        },
        framework_generation: FrameworkGeneration::new(1, 1),
        capabilities: CapabilitySet::from_capabilities([
            Capability::SectionNode,
            Capability::UniversalTargeting,
            Capability::DataLoad,
            Capability::ProviderQueryDataSource,
            Capability::ProviderInitiatedActions,
        ]),
        effective_tenant_binding: EffectiveTenantBinding {
            scope: Scope::Tenant,
            tenant_id: Some("tenant-a".to_string()),
        },
        surfaces: vec![minimal_surface(provider_kind)],
        encryption_metadata: None,
    }
}

#[test]
fn protocol_registration_rejects_unsupported_framework_generation() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.framework_generation = FrameworkGeneration::new(2, 0);

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("unsupported framework generation should be rejected");
    assert_eq!(
        err.code,
        SurfaceRegistrationErrorCode::UnsupportedGeneration
    );
}

#[test]
fn protocol_registration_rejects_missing_capability() {
    let registration = minimal_registration(ProviderKind::Plugin);

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::from_capabilities([
            Capability::TargetedTargeting,
        ])))
        .expect_err("missing capability should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::MissingCapability);
}

#[test]
fn protocol_registration_rejects_duplicate_surface_local_ids() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration
        .capabilities
        .0
        .insert(Capability::MutationAction);
    let duplicate_interaction = InteractionDescriptor {
        interaction_id: InteractionId::new("surface.refresh").expect("valid id"),
        kind: InteractionKind::MutationAction,
        label: Some("Refresh".to_string()),
        required_permission: None,
        input_schema: Some(SchemaContract::Object),
        result_schema: Some(SchemaContract::Any),
        sensitive_fields: Vec::new(),
        timeout_seconds: Some(30),
        confirmation: None,
        transport: InteractionTransport::ProviderProxied,
        workflow_steps: Vec::new(),
        form_ui: None,
    };
    registration.surfaces[0]
        .interactions
        .push(duplicate_interaction);

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("duplicate interaction id must be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn protocol_registration_rejects_service_controller_query() {
    let mut registration = minimal_registration(ProviderKind::Service);
    registration
        .capabilities
        .0
        .insert(Capability::ControllerQueryDataSource);
    registration.surfaces[0].data_sources[0].kind = DataSourceKind::ControllerQuery {
        query_id: ControllerQueryId::new("controller.hosts").expect("valid controller query id"),
    };

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("service provider controller query should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn protocol_registration_rejects_provider_interval_below_minimum() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.surfaces[0].data_sources[0].refresh_policy = RefreshPolicy::Interval {
        seconds: MIN_PROVIDER_REFRESH_INTERVAL_SECONDS - 1,
    };

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("provider query interval below minimum should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn protocol_registration_rejects_provider_direct_builtin_api_transport() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.surfaces[0].interactions[0].transport = InteractionTransport::DirectBuiltInApi {
        operation_id: BuiltInApiOperationId::new("settings.save").expect("valid operation id"),
    };

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("provider direct built-in API transport should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn protocol_action_request_round_trip_preserves_origin_and_encrypted_sensitive_params() {
    let mut params = Map::new();
    params.insert(
        "display_name".to_string(),
        Value::String("Router".to_string()),
    );

    let request = SurfaceActionRequest {
        request_id: Uuid::new_v4(),
        tenant_id: "tenant-a".to_string(),
        surface_id: SurfaceId::new("provider.sample.surface").expect("valid id"),
        interaction_id: InteractionId::new("surface.refresh").expect("valid id"),
        idempotency_key: "idem-123".to_string(),
        target_provider_id: Some("provider-1".to_string()),
        caller_origin: CallerOrigin::UserSession {
            user_id: "user-1".to_string(),
            session_id: "session-1".to_string(),
        },
        params,
        encrypted_sensitive_params: Some(EncryptedSensitiveParams {
            key_id: "key-1".to_string(),
            algorithm: uptrakit_surfaces::ProviderEncryptionAlgorithm::EciesP256,
            ciphertext_b64: "ZmFrZV9jaXBoZXJ0ZXh0".to_string(),
        }),
    };

    let encoded = serde_json::to_value(&request).expect("serialize request");
    assert_eq!(encoded["caller_origin"]["kind"], json!("user_session"));
    assert_eq!(encoded["params"]["display_name"], json!("Router"));

    let decoded: SurfaceActionRequest =
        serde_json::from_value(encoded).expect("deserialize request");
    assert_eq!(
        decoded.caller_origin,
        CallerOrigin::UserSession {
            user_id: "user-1".to_string(),
            session_id: "session-1".to_string(),
        }
    );
    assert!(decoded.encrypted_sensitive_params.is_some());
}

#[test]
fn protocol_data_source_static_supports_embedded_data_and_read_contract_fields() {
    let descriptor = DataSourceDescriptor {
        data_source_id: DataSourceId::new("surface.static.data").expect("valid data source id"),
        kind: DataSourceKind::Static {
            data: json!({
                "rows": [
                    {"id": 1, "name": "router"}
                ]
            }),
        },
        result_schema: SchemaContract::Object,
        pagination: Some(DataSourcePagination {
            default_page_size: 20,
            max_page_size: 100,
        }),
        sorting: Some(DataSourceSorting {
            sortable_fields: vec!["name".to_string()],
            default_sort_field: Some("name".to_string()),
        }),
        filtering: Some(DataSourceFiltering {
            filter_fields: vec!["name".to_string()],
        }),
        refresh_policy: RefreshPolicy::Manual,
        empty_state: Some(DataSourceEmptyState {
            title: "Empty".to_string(),
            description: Some("No static rows".to_string()),
        }),
    };

    let encoded = serde_json::to_value(&descriptor).expect("serialize static data source");
    assert_eq!(
        encoded["kind"],
        json!({"kind": "static", "data": {"rows": [{"id": 1, "name": "router"}]}})
    );
    assert_eq!(encoded["pagination"]["default_page_size"], json!(20));
    assert_eq!(encoded["sorting"]["default_sort_field"], json!("name"));
    assert_eq!(encoded["filtering"]["filter_fields"], json!(["name"]));
    assert_eq!(encoded["empty_state"]["title"], json!("Empty"));
}

#[test]
fn protocol_confirmable_action_carries_confirmation_metadata() {
    let interaction = InteractionDescriptor {
        interaction_id: InteractionId::new("surface.delete").expect("valid id"),
        kind: InteractionKind::ConfirmableAction,
        label: Some("Delete".to_string()),
        required_permission: None,
        input_schema: Some(SchemaContract::Object),
        result_schema: Some(SchemaContract::Any),
        sensitive_fields: Vec::new(),
        timeout_seconds: Some(30),
        confirmation: Some(InteractionConfirmation {
            title: "Delete item?".to_string(),
            message: "This cannot be undone.".to_string(),
            confirm_label: Some("Delete".to_string()),
            cancel_label: Some("Cancel".to_string()),
            severity: uptrakit_surfaces::ConfirmationSeverity::Danger,
        }),
        transport: InteractionTransport::ProviderProxied,
        workflow_steps: Vec::new(),
        form_ui: None,
    };

    let encoded = serde_json::to_value(&interaction).expect("serialize interaction");
    assert_eq!(encoded["confirmation"]["title"], json!("Delete item?"));
    assert_eq!(encoded["confirmation"]["severity"], json!("danger"));
}

#[test]
fn protocol_registration_rejects_dangling_node_references() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.capabilities.0.insert(Capability::FormNode);
    registration.surfaces[0].descriptor.root_node = SurfaceNode::Form {
        interaction_id: InteractionId::new("missing.interaction").expect("valid id"),
    };

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("dangling interaction reference should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn protocol_registration_rejects_duplicate_surface_ids_within_batch() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    let mut duplicate = minimal_surface(ProviderKind::Plugin);
    duplicate.descriptor.surface_id =
        SurfaceId::new("provider.sample.surface").expect("valid surface id");
    registration.surfaces.push(duplicate);

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("duplicate surface_id should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn protocol_registration_rejects_surface_required_capabilities_not_advertised() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.surfaces[0].descriptor.required_capabilities =
        CapabilitySet::from_capabilities([Capability::WorkflowTriggerNode]);

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("surface required capabilities must be subset of registration capabilities");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::MissingCapability);
}

#[test]
fn protocol_registration_rejects_priority_outside_slot_bounds() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.surfaces[0].descriptor.priority = 50;

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("priority outside slot bounds should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn protocol_registration_rejects_multiple_surfaces_in_single_entry_slot() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.surfaces[0].descriptor.slot = SLOT_EXTENSION_PAGE.to_string();

    let mut second = minimal_surface(ProviderKind::Plugin);
    second.descriptor.surface_id = SurfaceId::new("provider.sample.surface2").expect("valid id");
    second.descriptor.slot = SLOT_EXTENSION_PAGE.to_string();
    registration.surfaces.push(second);

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("single-entry slot should reject multiple surfaces in one batch");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn protocol_registration_rejects_missing_node_kind_capability_for_root_node_usage() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.surfaces[0].descriptor.root_node = SurfaceNode::Table {
        data_source_id: DataSourceId::new("surface.data").expect("valid id"),
        columns: vec![],
        row_actions: vec![],
    };

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("table node usage should require table_node capability");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::MissingCapability);
}

#[test]
fn protocol_registration_rejects_missing_targeting_capability_for_targeted_surface() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.surfaces[0].descriptor.targeting = Targeting::Targeted;

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("targeted surface should require targeted_targeting capability");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::MissingCapability);
}

#[test]
fn protocol_registration_rejects_missing_interaction_transport_and_sensitive_capabilities() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration
        .capabilities
        .0
        .remove(&Capability::ProviderInitiatedActions);
    registration.surfaces[0].interactions[0]
        .sensitive_fields
        .push("token".to_string());

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("provider transport and sensitive fields should require capabilities");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::MissingCapability);
}

#[test]
fn protocol_registration_rejects_missing_data_source_kind_capability_for_usage() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration
        .capabilities
        .0
        .remove(&Capability::ProviderQueryDataSource);

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err(
            "provider query data source should require provider_query_data_source capability",
        );
    assert_eq!(err.code, SurfaceRegistrationErrorCode::MissingCapability);
}

#[test]
fn protocol_registration_rejects_duplicate_tab_ids_within_tabs_node() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.capabilities.0.insert(Capability::TabsNode);
    registration
        .capabilities
        .0
        .insert(Capability::TextBlockNode);
    registration.surfaces[0].descriptor.root_node = SurfaceNode::Tabs {
        tabs: vec![
            SurfaceTab {
                id: SurfaceTabId::new("status").expect("valid tab id"),
                label: "Status".to_string(),
                root: SurfaceNode::TextBlock {
                    text: "ok".to_string(),
                },
            },
            SurfaceTab {
                id: SurfaceTabId::new("status").expect("valid tab id"),
                label: "Details".to_string(),
                root: SurfaceNode::TextBlock {
                    text: "duplicate".to_string(),
                },
            },
        ],
    };

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("duplicate tab ids should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}
