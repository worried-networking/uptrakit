#![expect(
    clippy::expect_used,
    reason = "integration test helpers — expect() on compile-time-valid IDs and schemas is the idiomatic way to construct test values in non-#[test] helper functions"
)]

use serde_json::{Map, Value, json};
use uuid::Uuid;

use uptrakit_surfaces::{
    ActionRef, CallerOrigin, Capability, CapabilitySet, ControllerQueryId, DataSourceDescriptor,
    DataSourceEmptyState, DataSourceFiltering, DataSourceId, DataSourceKind, DataSourcePagination,
    DataSourceSorting, EffectiveTenantBinding, EncryptedSensitiveParams, FormFieldDescriptor,
    FormUiDescriptor, FrameworkGeneration, FrameworkGenerationRange, InteractionConfirmation,
    InteractionDescriptor, InteractionHttpMethod, InteractionId, InteractionKind,
    InteractionTransport, MIN_PROVIDER_REFRESH_INTERVAL_SECONDS, ParamFieldDescriptor,
    ProviderEncryptionAlgorithm, ProviderIdentity, ProviderKind, RefreshPolicy, RegisteredSurface,
    SLOT_SETTINGS_TABS, SLOT_SURFACE_PAGE, SchemaContract, Scope, SurfaceActionRequest,
    SurfaceDescriptor, SurfaceId, SurfaceNode, SurfaceRegistration, SurfaceRegistrationErrorCode,
    SurfaceRegistrationPolicy, SurfaceTab, SurfaceTabId, Targeting, WorkflowStepDescriptor,
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
        descriptor: SurfaceDescriptor::builder()
            .surface_id(SurfaceId::new("provider.sample.surface").expect("valid surface id"))
            .label("Sample")
            .priority(200)
            .slot(SLOT_SETTINGS_TABS)
            .scope(Scope::Tenant)
            .targeting(Targeting::Universal)
            .provider_kind(provider_kind)
            .required_capabilities(CapabilitySet::from_capabilities([Capability::SectionNode]))
            .root_node(SurfaceNode::section(Some("Sample"), Vec::new()))
            .build(),
        interactions: {
            let mut i = InteractionDescriptor::new(
                InteractionId::new("surface.refresh").expect("valid interaction id"),
                InteractionKind::DataLoad,
                "Refresh",
                InteractionTransport::ProviderProxied,
            );
            i.input_schema = Some(SchemaContract::Any);
            i.result_schema = Some(SchemaContract::Any);
            i.timeout_seconds = Some(30);
            vec![i]
        },
        data_sources: vec![DataSourceDescriptor {
            data_source_id: DataSourceId::new("surface.data").expect("valid data source id"),
            kind: DataSourceKind::ProviderQuery {
                operation_id: "surface.refresh".to_string(),
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
            provider_kind,
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
    // REST method model (spec B1): `interaction_id` is unique per
    // `(id, effective_http_method)` pair, not per bare id — so the duplicate
    // must collide on the *same* effective method (GET, matching the
    // existing DataLoad "surface.refresh") to still be rejected. The
    // different-method case is covered by `same_id_different_methods_accepted`.
    let mut registration = minimal_registration(ProviderKind::Plugin);
    let duplicate_interaction = {
        let mut i = InteractionDescriptor::new(
            InteractionId::new("surface.refresh").expect("valid id"),
            InteractionKind::DataLoad,
            "Refresh",
            InteractionTransport::ProviderProxied,
        );
        i.input_schema = Some(SchemaContract::Any);
        i.result_schema = Some(SchemaContract::Any);
        i.timeout_seconds = Some(30);
        i
    };
    registration.surfaces[0]
        .interactions
        .push(duplicate_interaction);

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("duplicate (id, method) pair must be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn protocol_registration_rejects_missing_interaction_label_during_deserialization() {
    let registration = minimal_registration(ProviderKind::Plugin);
    let mut encoded = serde_json::to_value(&registration).expect("serialize registration");
    encoded["surfaces"][0]["interactions"][0]
        .as_object_mut()
        .expect("interaction object")
        .remove("label");

    let err = serde_json::from_value::<SurfaceRegistration>(encoded)
        .expect_err("missing interaction label must fail deserialization");
    assert!(err.to_string().contains("label"));
}

#[test]
fn protocol_registration_rejects_blank_interaction_label() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.surfaces[0].interactions[0].label = "   ".to_string();

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("blank interaction label must be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
    assert!(err.message.contains("label"));
}

#[test]
fn protocol_registration_rejects_missing_workflow_step_label_during_deserialization() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.capabilities.0.insert(Capability::Workflow);
    registration.surfaces[0].interactions[0] = {
        let mut i = InteractionDescriptor::new(
            InteractionId::new("surface.bootstrap").expect("valid interaction id"),
            InteractionKind::Workflow,
            "Bootstrap",
            InteractionTransport::ProviderProxied,
        );
        i.input_schema = Some(SchemaContract::Object);
        i.result_schema = Some(SchemaContract::Any);
        i.timeout_seconds = Some(30);
        i.workflow_steps = vec![WorkflowStepDescriptor {
            step_id: "connect".to_string(),
            label: "Connect".to_string(),
            form_ui: None,
            submit_interaction_id: None,
            render_previous_response: false,
            input_schema: SchemaContract::Object,
            result_schema: SchemaContract::Any,
        }];
        i
    };

    let mut encoded = serde_json::to_value(&registration).expect("serialize registration");
    encoded["surfaces"][0]["interactions"][0]["workflow_steps"][0]
        .as_object_mut()
        .expect("workflow step object")
        .remove("label");

    let err = serde_json::from_value::<SurfaceRegistration>(encoded)
        .expect_err("missing workflow step label must fail deserialization");
    assert!(err.to_string().contains("label"));
}

#[test]
fn protocol_registration_rejects_blank_workflow_step_label() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.capabilities.0.insert(Capability::Workflow);
    registration.surfaces[0].interactions[0] = {
        let mut i = InteractionDescriptor::new(
            InteractionId::new("surface.bootstrap").expect("valid interaction id"),
            InteractionKind::Workflow,
            "Bootstrap",
            InteractionTransport::ProviderProxied,
        );
        i.input_schema = Some(SchemaContract::Object);
        i.result_schema = Some(SchemaContract::Any);
        i.timeout_seconds = Some(30);
        i.workflow_steps = vec![WorkflowStepDescriptor {
            step_id: "connect".to_string(),
            label: "   ".to_string(),
            form_ui: None,
            submit_interaction_id: None,
            render_previous_response: false,
            input_schema: SchemaContract::Object,
            result_schema: SchemaContract::Any,
        }];
        i
    };

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("blank workflow step label must be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
    assert!(err.message.contains("label"));
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
    let interaction = {
        let mut i = InteractionDescriptor::new(
            InteractionId::new("surface.delete").expect("valid id"),
            InteractionKind::ConfirmableAction,
            "Delete",
            InteractionTransport::ProviderProxied,
        );
        i.input_schema = Some(SchemaContract::Object);
        i.result_schema = Some(SchemaContract::Any);
        i.timeout_seconds = Some(30);
        i.confirmation = Some(InteractionConfirmation {
            title: "Delete item?".to_string(),
            message: "This cannot be undone.".to_string(),
            confirm_label: Some("Delete".to_string()),
            cancel_label: Some("Cancel".to_string()),
            severity: uptrakit_surfaces::ConfirmationSeverity::Danger,
        });
        i
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
        http_method: None,
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
    registration.surfaces[0].descriptor.slot = SLOT_SURFACE_PAGE.to_string();

    let mut second = minimal_surface(ProviderKind::Plugin);
    second.descriptor.surface_id = SurfaceId::new("provider.sample.surface2").expect("valid id");
    second.descriptor.slot = SLOT_SURFACE_PAGE.to_string();
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

#[test]
fn protocol_workflow_step_round_trip_preserves_wizard_metadata() {
    let interaction = {
        let mut i = InteractionDescriptor::new(
            InteractionId::new("surface.bootstrap").expect("valid id"),
            InteractionKind::Workflow,
            "Bootstrap Host",
            InteractionTransport::ProviderProxied,
        );
        i.input_schema = Some(SchemaContract::Object);
        i.result_schema = Some(SchemaContract::Any);
        i.timeout_seconds = Some(120);
        i.workflow_steps = vec![WorkflowStepDescriptor {
            step_id: "connect".to_string(),
            label: "Connection & Authentication".to_string(),
            form_ui: Some(FormUiDescriptor {
                fields: vec![FormFieldDescriptor {
                    key: "target".to_string(),
                    label: "SSH Target".to_string(),
                    field_type: "text".to_string(),
                    required: true,
                    placeholder: None,
                    help_text: None,
                    default_value: None,
                    options: vec![],
                    select_source: None,
                    sensitive: false,
                    list: false,
                    visible_when: None,
                }],
                pre_load_interaction_id: Some(
                    InteractionId::new("surface.bootstrap.preload").expect("valid id"),
                ),
            }),
            submit_interaction_id: Some(
                InteractionId::new("surface.bootstrap.connect").expect("valid id"),
            ),
            render_previous_response: false,
            input_schema: SchemaContract::Object,
            result_schema: SchemaContract::Any,
        }];
        i
    };

    let encoded = serde_json::to_value(&interaction).expect("serialize interaction");
    assert_eq!(
        encoded["workflow_steps"][0]["label"],
        json!("Connection & Authentication")
    );
    assert_eq!(
        encoded["workflow_steps"][0]["submit_interaction_id"],
        json!("surface.bootstrap.connect")
    );
    assert_eq!(
        encoded["workflow_steps"][0]["form_ui"]["fields"][0]["key"],
        json!("target")
    );

    let decoded: InteractionDescriptor =
        serde_json::from_value(encoded).expect("deserialize interaction");
    assert_eq!(decoded.kind, InteractionKind::Workflow);
    assert_eq!(decoded.workflow_steps.len(), 1);
    assert_eq!(decoded.workflow_steps[0].step_id, "connect");
    assert_eq!(
        decoded.workflow_steps[0]
            .submit_interaction_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("surface.bootstrap.connect")
    );
}

#[test]
fn protocol_registration_rejects_workflow_step_unknown_submit_interaction() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.capabilities.0.insert(Capability::Workflow);
    registration.surfaces[0].interactions.push({
        let mut i = InteractionDescriptor::new(
            InteractionId::new("surface.workflow").expect("valid id"),
            InteractionKind::Workflow,
            "Workflow",
            InteractionTransport::ProviderProxied,
        );
        i.input_schema = Some(SchemaContract::Object);
        i.result_schema = Some(SchemaContract::Any);
        i.timeout_seconds = Some(30);
        i.workflow_steps = vec![WorkflowStepDescriptor {
            step_id: "step-1".to_string(),
            label: "Step 1".to_string(),
            form_ui: None,
            submit_interaction_id: Some(
                InteractionId::new("surface.workflow.missing").expect("valid id"),
            ),
            render_previous_response: false,
            input_schema: SchemaContract::Object,
            result_schema: SchemaContract::Any,
        }];
        i
    });

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("workflow step submit interaction id should be validated");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn provider_encryption_algorithm_ecies_p256_serializes_correctly() {
    let val = ProviderEncryptionAlgorithm::EciesP256;
    let json = serde_json::to_string(&val).expect("serialize");
    assert_eq!(json, "\"ecies_p256\"");
}

#[test]
fn provider_encryption_algorithm_ecies_p256_deserializes_correctly() {
    let val: ProviderEncryptionAlgorithm =
        serde_json::from_str("\"ecies_p256\"").expect("deserialize");
    assert_eq!(val, ProviderEncryptionAlgorithm::EciesP256);
}

#[test]
fn provider_encryption_algorithm_unknown_deserializes_to_other() {
    let val: ProviderEncryptionAlgorithm =
        serde_json::from_str("\"ecies_p384\"").expect("deserialize unknown");
    assert_eq!(
        val,
        ProviderEncryptionAlgorithm::Other("ecies_p384".to_string())
    );
}

#[test]
fn provider_encryption_algorithm_other_round_trips() {
    let original = ProviderEncryptionAlgorithm::Other("ecies_p384".to_string());
    let json = serde_json::to_string(&original).expect("serialize");
    assert_eq!(json, "\"ecies_p384\"");
    let back: ProviderEncryptionAlgorithm = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, original);
}

#[test]
fn provider_encryption_algorithm_known_variants_round_trip() {
    for v in ProviderEncryptionAlgorithm::KNOWN_VARIANTS {
        let json = serde_json::to_string(v).expect("serialize");
        let back: ProviderEncryptionAlgorithm = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(*v, back);
    }
}

// REST method model — (id, method) uniqueness, kind/method matrix, params
// rules, method-aware reference resolution (spec B1, B5, §2a, §4 rule 1).

#[test]
fn duplicate_id_method_pair_rejected() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    let duplicate = {
        let mut i = InteractionDescriptor::new(
            InteractionId::new("surface.refresh").expect("valid id"),
            InteractionKind::DataLoad,
            "Refresh",
            InteractionTransport::ProviderProxied,
        );
        i.input_schema = Some(SchemaContract::Any);
        i.result_schema = Some(SchemaContract::Any);
        i.timeout_seconds = Some(30);
        i
    };
    registration.surfaces[0].interactions.push(duplicate);

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("duplicate (id, method) pair should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
    assert!(err.message.contains("duplicate interaction"));
}

#[test]
fn same_id_different_methods_accepted() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration
        .capabilities
        .0
        .insert(Capability::MutationAction);
    let mutation = {
        let mut i = InteractionDescriptor::new(
            InteractionId::new("surface.refresh").expect("valid id"),
            InteractionKind::MutationAction,
            "Refresh (mutate)",
            InteractionTransport::ProviderProxied,
        );
        i.input_schema = Some(SchemaContract::Object);
        i.result_schema = Some(SchemaContract::Any);
        i.timeout_seconds = Some(30);
        i
    };
    registration.surfaces[0].interactions.push(mutation);

    registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect("same id registered under different methods should be accepted");
}

#[test]
fn dataload_put_rejected() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.surfaces[0].interactions[0].http_method = InteractionHttpMethod::Put;

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("data-load interaction declaring PUT should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn dataload_delete_rejected() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.surfaces[0].interactions[0].http_method = InteractionHttpMethod::Delete;

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("data-load interaction declaring DELETE should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn other_method_rejected() {
    // Built from raw JSON (not direct field assignment) to prove the
    // end-to-end wire path: a peer sending `"http_method": "patch"`
    // deserializes into `InteractionHttpMethod::Other(_)` (wire_safe_enum
    // catch-all), and that value is rejected at admission.
    let registration = minimal_registration(ProviderKind::Plugin);
    let mut encoded = serde_json::to_value(&registration).expect("serialize registration");
    encoded["surfaces"][0]["interactions"][0]["http_method"] = json!("patch");

    let decoded: SurfaceRegistration =
        serde_json::from_value(encoded).expect("deserialize registration with unknown http_method");
    assert_eq!(
        decoded.surfaces[0].interactions[0].http_method,
        InteractionHttpMethod::Other("patch".to_string())
    );

    let err = decoded
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("an unknown declared http_method should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn workflow_non_post_rejected() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.capabilities.0.insert(Capability::Workflow);
    let interaction = &mut registration.surfaces[0].interactions[0];
    interaction.kind = InteractionKind::Workflow;
    interaction.http_method = InteractionHttpMethod::Get;
    interaction.workflow_steps = vec![WorkflowStepDescriptor {
        step_id: "connect".to_string(),
        label: "Connect".to_string(),
        form_ui: None,
        submit_interaction_id: None,
        render_previous_response: false,
        input_schema: SchemaContract::Object,
        result_schema: SchemaContract::Any,
    }];

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("workflow interaction not declaring POST should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn non_dataload_get_rejected() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration
        .capabilities
        .0
        .insert(Capability::MutationAction);
    let interaction = &mut registration.surfaces[0].interactions[0];
    interaction.kind = InteractionKind::MutationAction;
    interaction.http_method = InteractionHttpMethod::Get;

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("a non data-load interaction declaring GET should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn reserved_param_key_rejected() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.surfaces[0].interactions[0].params =
        vec![ParamFieldDescriptor::new("id", SchemaContract::String)];

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("a param key colliding with the reserved `id` key should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);

    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.surfaces[0].interactions[0].params =
        vec![ParamFieldDescriptor::new("page", SchemaContract::String)];

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("a param key colliding with the reserved `page` key should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn dataload_array_param_rejected() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.surfaces[0].interactions[0].params =
        vec![ParamFieldDescriptor::new("tags", SchemaContract::Array)];

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err(
            "a data-load interaction declaring a non-scalar param schema should be rejected",
        );
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn dataload_sensitive_fields_rejected() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration
        .capabilities
        .0
        .insert(Capability::SensitiveFields);
    registration.surfaces[0].interactions[0]
        .sensitive_fields
        .push("token".to_string());

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("a data-load interaction declaring sensitive_fields should be rejected");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn bare_reference_to_multi_method_id_rejected() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration
        .capabilities
        .0
        .insert(Capability::ActionBarNode);
    registration
        .capabilities
        .0
        .insert(Capability::MutationAction);
    let mutation = {
        let mut i = InteractionDescriptor::new(
            InteractionId::new("surface.refresh").expect("valid id"),
            InteractionKind::MutationAction,
            "Refresh (mutate)",
            InteractionTransport::ProviderProxied,
        );
        i.input_schema = Some(SchemaContract::Object);
        i.result_schema = Some(SchemaContract::Any);
        i.timeout_seconds = Some(30);
        i
    };
    registration.surfaces[0].interactions.push(mutation);
    registration.surfaces[0].descriptor.root_node = SurfaceNode::ActionBar {
        action_ids: vec![ActionRef::from(
            InteractionId::new("surface.refresh").expect("valid id"),
        )],
    };

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("a bare reference to an id registered under multiple methods must fail closed");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
    assert!(err.message.contains("ambiguous"));
}

#[test]
fn explicit_reference_to_unregistered_pair_rejected() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration
        .capabilities
        .0
        .insert(Capability::ActionBarNode);
    registration.surfaces[0].descriptor.root_node = SurfaceNode::ActionBar {
        action_ids: vec![ActionRef::WithMethod {
            interaction_id: InteractionId::new("surface.refresh").expect("valid id"),
            http_method: Some(InteractionHttpMethod::Delete),
        }],
    };

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err(
            "an explicit (id, method) reference to a pair that was never registered must be rejected",
        );
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn pre_load_reference_must_be_get() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration.capabilities.0.insert(Capability::FormNode);
    registration.capabilities.0.insert(Capability::FormSubmit);
    registration
        .capabilities
        .0
        .insert(Capability::MutationAction);

    let preload = {
        let mut i = InteractionDescriptor::new(
            InteractionId::new("preload").expect("valid id"),
            InteractionKind::MutationAction,
            "Preload",
            InteractionTransport::ProviderProxied,
        );
        i.input_schema = Some(SchemaContract::Object);
        i.result_schema = Some(SchemaContract::Any);
        i.timeout_seconds = Some(30);
        i
    };
    let create = {
        let mut i = InteractionDescriptor::new(
            InteractionId::new("create").expect("valid id"),
            InteractionKind::FormSubmit,
            "Create",
            InteractionTransport::ProviderProxied,
        );
        i.input_schema = Some(SchemaContract::Object);
        i.result_schema = Some(SchemaContract::Any);
        i.timeout_seconds = Some(30);
        i.form_ui = Some(FormUiDescriptor {
            fields: vec![],
            pre_load_interaction_id: Some(InteractionId::new("preload").expect("valid id")),
        });
        i
    };
    registration.surfaces[0].interactions.push(preload);
    registration.surfaces[0].interactions.push(create);
    registration.surfaces[0].descriptor.root_node = SurfaceNode::Form {
        interaction_id: InteractionId::new("create").expect("valid id"),
        http_method: None,
    };

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err(
            "a root-level form's pre_load_interaction_id must resolve to a GET-registered interaction",
        );
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn provider_query_operation_id_must_resolve_get() {
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration
        .capabilities
        .0
        .insert(Capability::MutationAction);
    let mutation = {
        let mut i = InteractionDescriptor::new(
            InteractionId::new("mutate-op").expect("valid id"),
            InteractionKind::MutationAction,
            "Mutate",
            InteractionTransport::ProviderProxied,
        );
        i.input_schema = Some(SchemaContract::Object);
        i.result_schema = Some(SchemaContract::Any);
        i.timeout_seconds = Some(30);
        i
    };
    registration.surfaces[0].interactions.push(mutation);
    registration.surfaces[0].data_sources[0].kind = DataSourceKind::ProviderQuery {
        operation_id: "mutate-op".to_string(),
    };

    let err = registration
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect_err("a provider_query operation_id must resolve to a GET-registered interaction");
    assert_eq!(err.code, SurfaceRegistrationErrorCode::InvalidContract);
}

#[test]
fn legacy_single_method_bare_references_still_admit() {
    // Simulates an old peer's wire payload: no interaction registers under
    // more than one method, and every reference is a bare id string (no
    // `http_method` anywhere). Must still admit under the new resolver.
    let mut registration = minimal_registration(ProviderKind::Plugin);
    registration
        .capabilities
        .0
        .insert(Capability::ActionBarNode);
    registration
        .capabilities
        .0
        .insert(Capability::MutationAction);
    let create = {
        let mut i = InteractionDescriptor::new(
            InteractionId::new("create").expect("valid id"),
            InteractionKind::MutationAction,
            "Create",
            InteractionTransport::ProviderProxied,
        );
        i.input_schema = Some(SchemaContract::Object);
        i.result_schema = Some(SchemaContract::Any);
        i.timeout_seconds = Some(30);
        i
    };
    registration.surfaces[0].interactions.push(create);
    registration.surfaces[0].descriptor.root_node = SurfaceNode::ActionBar {
        action_ids: vec![
            ActionRef::from(InteractionId::new("surface.refresh").expect("valid id")),
            ActionRef::from(InteractionId::new("create").expect("valid id")),
        ],
    };

    let encoded = serde_json::to_value(&registration).expect("serialize registration");
    assert_eq!(
        encoded["surfaces"][0]["descriptor"]["root_node"]["action_ids"],
        json!(["surface.refresh", "create"]),
        "legacy bare references must serialize as plain id strings, not method-tagged objects"
    );

    let decoded: SurfaceRegistration =
        serde_json::from_value(encoded).expect("deserialize legacy-shaped registration");

    decoded
        .validate_against(&registration_policy(CapabilitySet::default()))
        .expect("legacy single-method bare-reference payload should still admit");
}
