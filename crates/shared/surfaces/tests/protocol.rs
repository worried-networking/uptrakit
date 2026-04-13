use serde_json::{Map, Value, json};
use uuid::Uuid;

use uptrakit_surfaces::{
    BuiltInApiOperationId, CallerOrigin, Capability, CapabilitySet, DataSourceDescriptor,
    DataSourceId, DataSourceKind, EffectiveTenantBinding, EncryptedSensitiveParams,
    FrameworkGeneration, FrameworkGenerationRange, InteractionDescriptor, InteractionId,
    InteractionKind, InteractionTransport, MIN_PROVIDER_REFRESH_INTERVAL_SECONDS, ProviderIdentity,
    ProviderKind, RefreshPolicy, RegisteredSurface, SLOT_SETTINGS_TABS, SchemaContract, Scope,
    SurfaceActionRequest, SurfaceDescriptor, SurfaceId, SurfaceNode, SurfaceRegistration,
    SurfaceRegistrationErrorCode, SurfaceRegistrationPolicy, Targeting,
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
            required_permission: None,
            input_schema: Some(SchemaContract::Any),
            result_schema: Some(SchemaContract::Any),
            sensitive_fields: Vec::new(),
            timeout_seconds: Some(30),
            transport: InteractionTransport::ProviderProxied,
            workflow_steps: Vec::new(),
        }],
        data_sources: vec![DataSourceDescriptor {
            data_source_id: DataSourceId::new("surface.data").expect("valid data source id"),
            kind: DataSourceKind::ProviderQuery {
                operation_id: "list".to_string(),
            },
            result_schema: SchemaContract::Object,
            refresh_policy: RefreshPolicy::Interval {
                seconds: MIN_PROVIDER_REFRESH_INTERVAL_SECONDS,
            },
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
        capabilities: CapabilitySet::from_capabilities([Capability::SectionNode]),
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
    let duplicate_interaction = InteractionDescriptor {
        interaction_id: InteractionId::new("surface.refresh").expect("valid id"),
        kind: InteractionKind::MutationAction,
        required_permission: None,
        input_schema: Some(SchemaContract::Object),
        result_schema: Some(SchemaContract::Any),
        sensitive_fields: Vec::new(),
        timeout_seconds: Some(30),
        transport: InteractionTransport::ProviderProxied,
        workflow_steps: Vec::new(),
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
    registration.surfaces[0].data_sources[0].kind = DataSourceKind::ControllerQuery {
        query_id: "controller.hosts".to_string(),
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
        operation_id: BuiltInApiOperationId("settings.save".to_string()),
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
