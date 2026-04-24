//! Shared-surface registration and interaction helpers for the MQTT service.
//!
//! The MQTT service exposes a settings tab that lets users create, edit, list,
//! and delete MQTT client configurations stored in the service config store.
//!
//! Sensitive fields (`password`, `ca_pem`) are preserved as provider-proxied
//! interaction fields so the frontend can submit encrypted sensitive payloads.

use uptrakit_internal_wire::{
    ServiceMessage,
    surfaces::{
        self, Capability, CapabilitySet, DataSourceDescriptor, DataSourceId, DataSourceKind,
        DataSourcePagination, FrameworkGeneration, InteractionConfirmation, InteractionDescriptor,
        InteractionId, InteractionKind, InteractionTransport, ProviderEncryptionAlgorithm,
        ProviderEncryptionMetadata, RefreshPolicy, SurfaceActionError, SurfaceActionErrorCode,
        SurfaceActionRequest, SurfaceActionResponse, SurfaceDescriptor, SurfaceId, SurfaceNode,
        SurfaceRegistration, SurfaceRowCondition, SurfaceRowVisibleWhen, SurfaceTableColumn,
        SurfaceTableRowAction,
    },
};

/// Surface and interaction IDs — kept as constants to avoid magic strings.
pub(crate) const EXT_ID: &str = "mqtt.clients";
pub(crate) const ACTION_LIST: &str = "mqtt.list-clients";
pub(crate) const ACTION_CREATE: &str = "mqtt.create-client";
pub(crate) const ACTION_EDIT: &str = "mqtt.edit-client";
pub(crate) const ACTION_GET: &str = "mqtt.get-client";
pub(crate) const ACTION_DELETE: &str = "mqtt.delete-client";
const DATA_SOURCE_PRIMARY: &str = "mqtt.clients.primary";
const LIST_DEFAULT_PAGE: u64 = 1;
const LIST_DEFAULT_PER_PAGE: u64 = 50;
const LIST_MAX_PER_PAGE: u64 = 200;

pub(crate) fn build_surface_registration_with_ids(
    encryption_public_key: Option<String>,
    service_id: Option<uuid::Uuid>,
    service_tenant_id: Option<uuid::Uuid>,
) -> Option<SurfaceRegistration> {
    let tenant_id = service_tenant_id?;
    let provider_id = service_id
        .map(|id| format!("service.uptrakit-mqtt.{id}"))
        .unwrap_or_else(|| "service.uptrakit-mqtt".to_string());
    let scope = surfaces::Scope::Tenant;
    let targeting = surfaces::Targeting::Targeted;
    let binding_scope = surfaces::Scope::Tenant;
    let binding_tenant_id = Some(tenant_id.to_string());

    let required_capabilities = CapabilitySet::from_capabilities([
        Capability::SectionNode,
        Capability::ActionBarNode,
        Capability::TableNode,
        Capability::DataLoad,
        Capability::FormSubmit,
        Capability::ConfirmableAction,
        Capability::ProviderQueryDataSource,
        Capability::ProviderInitiatedActions,
        Capability::SensitiveFields,
        match targeting {
            surfaces::Targeting::Targeted => Capability::TargetedTargeting,
            surfaces::Targeting::Universal => Capability::UniversalTargeting,
            _ => {
                tracing::warn!(
                    ?targeting,
                    "unknown Targeting variant; defaulting to UniversalTargeting capability"
                );
                Capability::UniversalTargeting
            }
        },
    ]);

    let data_source_id = DataSourceId::new(DATA_SOURCE_PRIMARY).expect("data source id is valid");
    let registered_surface = surfaces::RegisteredSurface {
        descriptor: SurfaceDescriptor::builder()
            .surface_id(SurfaceId::new(EXT_ID).expect("surface id is valid"))
            .label("MQTT Clients")
            .priority(100)
            .slot(surfaces::SLOT_SETTINGS_TABS)
            .scope(scope)
            .targeting(targeting)
            .required_permission("update_system_services")
            .provider_kind(surfaces::ProviderKind::Service)
            .required_capabilities(required_capabilities.clone())
            .root_node(SurfaceNode::Section {
                title: None,
                children: vec![
                    SurfaceNode::ActionBar {
                        action_ids: vec![
                            InteractionId::new(ACTION_CREATE).expect("interaction id is valid"),
                        ],
                    },
                    SurfaceNode::Table {
                        data_source_id: data_source_id.clone(),
                        columns: vec![
                            SurfaceTableColumn {
                                key: "client_id".to_string(),
                                label: "Client ID".to_string(),
                            },
                            SurfaceTableColumn {
                                key: "host".to_string(),
                                label: "Broker Host".to_string(),
                            },
                            SurfaceTableColumn {
                                key: "transport".to_string(),
                                label: "Transport".to_string(),
                            },
                            SurfaceTableColumn {
                                key: "enabled".to_string(),
                                label: "Enabled".to_string(),
                            },
                            SurfaceTableColumn {
                                key: "ha_discovery".to_string(),
                                label: "HA Discovery".to_string(),
                            },
                        ],
                        row_actions: vec![
                            SurfaceTableRowAction {
                                interaction_id: InteractionId::new(ACTION_EDIT)
                                    .expect("interaction id is valid"),
                                visible_when: None,
                            },
                            SurfaceTableRowAction {
                                interaction_id: InteractionId::new(ACTION_DELETE)
                                    .expect("interaction id is valid"),
                                visible_when: Some(SurfaceRowVisibleWhen {
                                    field: "client_id".to_string(),
                                    condition: SurfaceRowCondition::Present,
                                }),
                            },
                        ],
                    },
                ],
            })
            .build(),
        interactions: build_interactions(),
        data_sources: vec![DataSourceDescriptor {
            data_source_id,
            kind: DataSourceKind::ProviderQuery {
                operation_id: ACTION_LIST.to_string(),
            },
            result_schema: surfaces::SchemaContract::Object,
            pagination: Some(DataSourcePagination {
                default_page_size: 50,
                max_page_size: 200,
            }),
            sorting: None,
            filtering: None,
            refresh_policy: RefreshPolicy::Manual,
            empty_state: None,
        }],
    };

    Some(SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id,
            provider_kind: surfaces::ProviderKind::Service,
            provider_namespace: "service".to_string(),
        },
        framework_generation: FrameworkGeneration::new(1, 0),
        capabilities: required_capabilities,
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: binding_scope,
            tenant_id: binding_tenant_id,
        },
        surfaces: vec![registered_surface],
        encryption_metadata: encryption_public_key.map(|public_key| ProviderEncryptionMetadata {
            key_id: service_id
                .map(|id| format!("mqtt-{id}"))
                .unwrap_or_else(|| "mqtt".to_string()),
            algorithm: ProviderEncryptionAlgorithm::EciesP256,
            public_key,
        }),
    })
}

/// Handle a list action request.
///
/// Builds a JSON summary of all current MQTT client configurations and returns
/// a success response. Returns `None` if the action ID does not match.
pub(crate) fn handle_list_action(
    request: &SurfaceActionRequest,
    tenant_id: uuid::Uuid,
    configs: &[crate::client_manager::ParsedMqttClientConfig],
) -> Option<SurfaceActionResponse> {
    if request.interaction_id.as_str() != ACTION_LIST {
        return None;
    }

    let page = request
        .params
        .get("page")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(LIST_DEFAULT_PAGE)
        .max(1);
    let per_page = request
        .params
        .get("per_page")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(LIST_DEFAULT_PER_PAGE)
        .clamp(1, LIST_MAX_PER_PAGE);

    let all_items: Vec<serde_json::Value> = configs
        .iter()
        .filter(|c| c.tenant_id == tenant_id)
        .map(|c| {
            serde_json::json!({
                "id": c.mqtt_client_id.to_string(),
                "client_id": c.client_id,
                "host": c.host,
                "transport": c.transport.as_str(),
                "enabled": c.enabled,
                "ha_discovery": c.ha_discovery,
                "topic_prefix": c.topic_prefix,
            })
        })
        .collect();
    let total = all_items.len() as u64;
    let total_pages = total.div_ceil(per_page);
    let offset_u64 = page.saturating_sub(1).saturating_mul(per_page);
    let offset = usize::try_from(offset_u64).unwrap_or(usize::MAX);
    let per_page_usize = usize::try_from(per_page).unwrap_or(usize::MAX);
    let items: Vec<serde_json::Value> = all_items
        .into_iter()
        .skip(offset)
        .take(per_page_usize)
        .collect();

    Some(SurfaceActionResponse {
        request_id: request.request_id,
        success: true,
        result: Some(serde_json::json!({
            "items": items,
            "total": total,
            "page": page,
            "per_page": per_page,
            "total_pages": total_pages,
        })),
        error: None,
    })
}

/// Handle the edit-form preload action.
///
/// Returns the non-sensitive MQTT client config for the requested entry.
pub(crate) fn handle_get_action(
    request: &SurfaceActionRequest,
    tenant_id: uuid::Uuid,
    configs: &[crate::client_manager::ParsedMqttClientConfig],
) -> Option<SurfaceActionResponse> {
    if request.interaction_id.as_str() != ACTION_GET {
        return None;
    }

    let id = request.params.get("id")?.as_str()?;
    let config = configs
        .iter()
        .find(|cfg| cfg.mqtt_client_id.to_string() == id && cfg.tenant_id == tenant_id)?;

    Some(SurfaceActionResponse {
        request_id: request.request_id,
        success: true,
        result: Some(serde_json::json!({
            "id": config.mqtt_client_id.to_string(),
            "client_id": config.client_id,
            "host": config.host,
            "port": config.port,
            "transport": config.transport.as_str(),
            "topic_prefix": config.topic_prefix,
            "username": config.username.as_ref().map(|value| value.expose_secret()),
            "ha_discovery": config.ha_discovery,
            "ha_discovery_prefix": config.ha_discovery_prefix,
            "enabled": config.enabled,
        })),
        error: None,
    })
}

/// Send an error response back to the controller for an unhandled or failed action.
pub(crate) async fn send_error_response(
    transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    request_id: uuid::Uuid,
    code: SurfaceActionErrorCode,
    message: impl Into<String>,
) -> Result<(), uptrakit_internal_wire::TransportError> {
    let response = SurfaceActionResponse {
        request_id,
        success: false,
        result: None,
        error: Some(SurfaceActionError {
            code,
            message: message.into(),
            details: None,
        }),
    };
    transport
        .transport_send(ServiceMessage::SurfaceActionResponse(response))
        .await
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::client_manager::ParsedMqttClientConfig;
    use uptrakit_internal_wire::SecretString;
    use uuid::Uuid;

    #[test]
    fn registration_places_surface_in_settings_tab() {
        let payload = build_surface_registration_with_ids(
            Some("test-key".to_string()),
            None,
            Some(Uuid::now_v7()),
        )
        .expect("registration");
        assert_eq!(
            payload
                .encryption_metadata
                .as_ref()
                .map(|metadata| metadata.public_key.as_str()),
            Some("test-key")
        );

        let surface = &payload.surfaces[0].descriptor;
        assert_eq!(surface.surface_id.as_str(), EXT_ID);
        assert_eq!(surface.slot, surfaces::SLOT_SETTINGS_TABS);
        assert_eq!(
            payload.surfaces[0].data_sources[0].result_schema,
            surfaces::SchemaContract::Object
        );
    }

    #[test]
    fn edit_form_uses_dedicated_preload_action() {
        let registration = build_surface_registration_with_ids(None, None, Some(Uuid::now_v7()))
            .expect("registration");
        let interactions = &registration.surfaces[0].interactions;
        let edit = interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == ACTION_EDIT)
            .expect("edit interaction");

        assert_eq!(edit.kind, InteractionKind::FormSubmit);
        assert_eq!(
            edit.form_ui
                .as_ref()
                .and_then(|ui| ui.pre_load_interaction_id.as_ref())
                .map(InteractionId::as_str),
            Some(ACTION_GET)
        );
    }

    #[test]
    fn mutating_interactions_publish_their_actual_result_schema() {
        let registration = build_surface_registration_with_ids(None, None, Some(Uuid::now_v7()))
            .expect("registration");
        let interactions = &registration.surfaces[0].interactions;
        let create = interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == ACTION_CREATE)
            .expect("create interaction");
        let edit = interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == ACTION_EDIT)
            .expect("edit interaction");
        let delete = interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == ACTION_DELETE)
            .expect("delete interaction");

        assert_eq!(create.result_schema, Some(surfaces::SchemaContract::Object));
        assert_eq!(edit.result_schema, Some(surfaces::SchemaContract::Null));
        assert_eq!(delete.result_schema, Some(surfaces::SchemaContract::Null));
    }

    #[test]
    fn registration_is_omitted_without_tenant_binding() {
        assert!(build_surface_registration_with_ids(None, None, None).is_none());
    }

    #[test]
    fn get_action_omits_sensitive_fields() {
        let tenant_id = Uuid::now_v7();
        let request = SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: tenant_id.to_string(),
            surface_id: SurfaceId::new(EXT_ID).expect("surface id"),
            interaction_id: InteractionId::new(ACTION_GET).expect("interaction id"),
            idempotency_key: "req-1".to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::BuiltInSystem {
                principal: "tests".to_string(),
            },
            params: serde_json::Map::from_iter([(
                "id".to_string(),
                serde_json::json!("019471a0-0000-7000-8000-000000000001"),
            )]),
            encrypted_sensitive_params: None,
        };
        let configs = vec![ParsedMqttClientConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000001").unwrap(),
            tenant_id,
            enabled: true,
            transport: crate::types::MqttTransport::Tls,
            host: "broker.example.com".to_string(),
            port: 8883,
            client_id: "mqtt-client".to_string(),
            username: Some(SecretString::new("user")),
            password: Some(SecretString::new("secret")),
            ca_pem: Some(SecretString::new("pem")),
            topic_prefix: "uptrakit".to_string(),
            ha_discovery: true,
            ha_discovery_prefix: "homeassistant".to_string(),
        }];

        let response = handle_get_action(&request, tenant_id, &configs).expect("response");
        let data = response
            .result
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .expect("object response");
        assert_eq!(
            data.get("client_id"),
            Some(&serde_json::json!("mqtt-client"))
        );
        assert_eq!(data.get("username"), Some(&serde_json::json!("user")));
        assert!(!data.contains_key("password"));
        assert!(!data.contains_key("ca_pem"));
    }

    #[test]
    fn list_action_filters_to_request_tenant() {
        let tenant_a = Uuid::now_v7();
        let tenant_b = Uuid::now_v7();
        let request = SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: tenant_a.to_string(),
            surface_id: SurfaceId::new(EXT_ID).expect("surface id"),
            interaction_id: InteractionId::new(ACTION_LIST).expect("interaction id"),
            idempotency_key: "req-1".to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::BuiltInSystem {
                principal: "tests".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: None,
        };
        let id_a = Uuid::now_v7();
        let id_b = Uuid::now_v7();
        let configs = vec![
            ParsedMqttClientConfig {
                mqtt_client_id: id_a,
                tenant_id: tenant_a,
                enabled: true,
                transport: crate::types::MqttTransport::Tcp,
                host: "a.example.com".to_string(),
                port: 1883,
                client_id: "client-a".to_string(),
                username: None,
                password: None,
                ca_pem: None,
                topic_prefix: "uptrakit".to_string(),
                ha_discovery: false,
                ha_discovery_prefix: "homeassistant".to_string(),
            },
            ParsedMqttClientConfig {
                mqtt_client_id: id_b,
                tenant_id: tenant_b,
                enabled: true,
                transport: crate::types::MqttTransport::Tcp,
                host: "b.example.com".to_string(),
                port: 1883,
                client_id: "client-b".to_string(),
                username: None,
                password: None,
                ca_pem: None,
                topic_prefix: "uptrakit".to_string(),
                ha_discovery: false,
                ha_discovery_prefix: "homeassistant".to_string(),
            },
        ];

        let response = handle_list_action(&request, tenant_a, &configs).expect("response");
        let result = response.result.as_ref().expect("result payload");
        let items = result
            .get("items")
            .and_then(serde_json::Value::as_array)
            .expect("items array");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].get("id"),
            Some(&serde_json::json!(id_a.to_string()))
        );
        assert_eq!(result.get("total"), Some(&serde_json::json!(1_u64)));
        assert_eq!(result.get("page"), Some(&serde_json::json!(1_u64)));
        assert_eq!(result.get("per_page"), Some(&serde_json::json!(50_u64)));
        assert_eq!(result.get("total_pages"), Some(&serde_json::json!(1_u64)));
    }

    #[test]
    fn list_action_returns_paginated_table_shape() {
        let tenant_id = Uuid::now_v7();
        let request = SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: tenant_id.to_string(),
            surface_id: SurfaceId::new(EXT_ID).expect("surface id"),
            interaction_id: InteractionId::new(ACTION_LIST).expect("interaction id"),
            idempotency_key: "req-1".to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::BuiltInSystem {
                principal: "tests".to_string(),
            },
            params: serde_json::Map::from_iter([
                ("page".to_string(), serde_json::json!(2_u64)),
                ("per_page".to_string(), serde_json::json!(1_u64)),
            ]),
            encrypted_sensitive_params: None,
        };
        let id_1 = Uuid::parse_str("019471a0-0000-7000-8000-000000000001").unwrap();
        let id_2 = Uuid::parse_str("019471a0-0000-7000-8000-000000000002").unwrap();
        let id_3 = Uuid::parse_str("019471a0-0000-7000-8000-000000000003").unwrap();
        let configs = vec![
            ParsedMqttClientConfig {
                mqtt_client_id: id_1,
                tenant_id,
                enabled: true,
                transport: crate::types::MqttTransport::Tcp,
                host: "a.example.com".to_string(),
                port: 1883,
                client_id: "client-a".to_string(),
                username: None,
                password: None,
                ca_pem: None,
                topic_prefix: "uptrakit".to_string(),
                ha_discovery: false,
                ha_discovery_prefix: "homeassistant".to_string(),
            },
            ParsedMqttClientConfig {
                mqtt_client_id: id_2,
                tenant_id,
                enabled: true,
                transport: crate::types::MqttTransport::Tcp,
                host: "b.example.com".to_string(),
                port: 1883,
                client_id: "client-b".to_string(),
                username: None,
                password: None,
                ca_pem: None,
                topic_prefix: "uptrakit".to_string(),
                ha_discovery: false,
                ha_discovery_prefix: "homeassistant".to_string(),
            },
            ParsedMqttClientConfig {
                mqtt_client_id: id_3,
                tenant_id,
                enabled: true,
                transport: crate::types::MqttTransport::Tcp,
                host: "c.example.com".to_string(),
                port: 1883,
                client_id: "client-c".to_string(),
                username: None,
                password: None,
                ca_pem: None,
                topic_prefix: "uptrakit".to_string(),
                ha_discovery: false,
                ha_discovery_prefix: "homeassistant".to_string(),
            },
        ];

        let response = handle_list_action(&request, tenant_id, &configs).expect("response");
        let result = response.result.as_ref().expect("result payload");
        let items = result
            .get("items")
            .and_then(serde_json::Value::as_array)
            .expect("items array");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].get("id"),
            Some(&serde_json::json!(id_2.to_string()))
        );
        assert_eq!(result.get("total"), Some(&serde_json::json!(3_u64)));
        assert_eq!(result.get("page"), Some(&serde_json::json!(2_u64)));
        assert_eq!(result.get("per_page"), Some(&serde_json::json!(1_u64)));
        assert_eq!(result.get("total_pages"), Some(&serde_json::json!(3_u64)));
    }

    #[test]
    fn list_action_handles_huge_page_without_overflow() {
        let tenant_id = Uuid::now_v7();
        let request = SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: tenant_id.to_string(),
            surface_id: SurfaceId::new(EXT_ID).expect("surface id"),
            interaction_id: InteractionId::new(ACTION_LIST).expect("interaction id"),
            idempotency_key: "req-1".to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::BuiltInSystem {
                principal: "tests".to_string(),
            },
            params: serde_json::Map::from_iter([
                ("page".to_string(), serde_json::json!(u64::MAX)),
                ("per_page".to_string(), serde_json::json!(LIST_MAX_PER_PAGE)),
            ]),
            encrypted_sensitive_params: None,
        };
        let id = Uuid::parse_str("019471a0-0000-7000-8000-000000000001").unwrap();
        let configs = vec![ParsedMqttClientConfig {
            mqtt_client_id: id,
            tenant_id,
            enabled: true,
            transport: crate::types::MqttTransport::Tcp,
            host: "a.example.com".to_string(),
            port: 1883,
            client_id: "client-a".to_string(),
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: "uptrakit".to_string(),
            ha_discovery: false,
            ha_discovery_prefix: "homeassistant".to_string(),
        }];

        let response = handle_list_action(&request, tenant_id, &configs).expect("response");
        let result = response.result.as_ref().expect("result payload");
        let items = result
            .get("items")
            .and_then(serde_json::Value::as_array)
            .expect("items array");
        assert!(items.is_empty());
        assert_eq!(result.get("total"), Some(&serde_json::json!(1_u64)));
        assert_eq!(result.get("page"), Some(&serde_json::json!(u64::MAX)));
        assert_eq!(
            result.get("per_page"),
            Some(&serde_json::json!(LIST_MAX_PER_PAGE))
        );
        assert_eq!(result.get("total_pages"), Some(&serde_json::json!(1_u64)));
    }

    #[test]
    fn get_action_rejects_cross_tenant_lookup() {
        let tenant_a = Uuid::now_v7();
        let tenant_b = Uuid::now_v7();
        let target_id = Uuid::parse_str("019471a0-0000-7000-8000-000000000001").unwrap();
        let request = SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: tenant_a.to_string(),
            surface_id: SurfaceId::new(EXT_ID).expect("surface id"),
            interaction_id: InteractionId::new(ACTION_GET).expect("interaction id"),
            idempotency_key: "req-1".to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::BuiltInSystem {
                principal: "tests".to_string(),
            },
            params: serde_json::Map::from_iter([(
                "id".to_string(),
                serde_json::json!(target_id.to_string()),
            )]),
            encrypted_sensitive_params: None,
        };
        let configs = vec![ParsedMqttClientConfig {
            mqtt_client_id: target_id,
            tenant_id: tenant_b,
            enabled: true,
            transport: crate::types::MqttTransport::Tls,
            host: "broker.example.com".to_string(),
            port: 8883,
            client_id: "mqtt-client".to_string(),
            username: Some(SecretString::new("user")),
            password: Some(SecretString::new("secret")),
            ca_pem: Some(SecretString::new("pem")),
            topic_prefix: "uptrakit".to_string(),
            ha_discovery: true,
            ha_discovery_prefix: "homeassistant".to_string(),
        }];

        assert!(handle_get_action(&request, tenant_a, &configs).is_none());
    }
}

fn build_interactions() -> Vec<InteractionDescriptor> {
    vec![
        InteractionDescriptor {
            interaction_id: InteractionId::new(ACTION_LIST).expect("interaction id is valid"),
            kind: InteractionKind::DataLoad,
            label: "List MQTT Clients".to_string(),
            required_permission: Some("update_system_services".to_string()),
            input_schema: Some(surfaces::SchemaContract::Object),
            result_schema: Some(surfaces::SchemaContract::Object),
            sensitive_fields: Vec::new(),
            timeout_seconds: Some(30),
            confirmation: None,
            transport: InteractionTransport::ProviderProxied,
            workflow_steps: Vec::new(),
            form_ui: None,
        },
        InteractionDescriptor {
            interaction_id: InteractionId::new(ACTION_CREATE).expect("interaction id is valid"),
            kind: InteractionKind::FormSubmit,
            label: "Add MQTT Client".to_string(),
            required_permission: Some("update_system_services".to_string()),
            input_schema: Some(surfaces::SchemaContract::Object),
            result_schema: Some(surfaces::SchemaContract::Object),
            sensitive_fields: vec!["password".to_string(), "ca_pem".to_string()],
            timeout_seconds: Some(30),
            confirmation: None,
            transport: InteractionTransport::ProviderProxied,
            workflow_steps: Vec::new(),
            form_ui: Some(build_client_form_ui(false)),
        },
        InteractionDescriptor {
            interaction_id: InteractionId::new(ACTION_EDIT).expect("interaction id is valid"),
            kind: InteractionKind::FormSubmit,
            label: "Edit MQTT Client".to_string(),
            required_permission: Some("update_system_services".to_string()),
            input_schema: Some(surfaces::SchemaContract::Object),
            result_schema: Some(surfaces::SchemaContract::Null),
            sensitive_fields: vec!["password".to_string(), "ca_pem".to_string()],
            timeout_seconds: Some(30),
            confirmation: None,
            transport: InteractionTransport::ProviderProxied,
            workflow_steps: Vec::new(),
            form_ui: Some(build_client_form_ui(true)),
        },
        InteractionDescriptor {
            interaction_id: InteractionId::new(ACTION_GET).expect("interaction id is valid"),
            kind: InteractionKind::DataLoad,
            label: "Get MQTT Client".to_string(),
            required_permission: Some("update_system_services".to_string()),
            input_schema: Some(surfaces::SchemaContract::Object),
            result_schema: Some(surfaces::SchemaContract::Object),
            sensitive_fields: Vec::new(),
            timeout_seconds: Some(30),
            confirmation: None,
            transport: InteractionTransport::ProviderProxied,
            workflow_steps: Vec::new(),
            form_ui: None,
        },
        InteractionDescriptor {
            interaction_id: InteractionId::new(ACTION_DELETE).expect("interaction id is valid"),
            kind: InteractionKind::ConfirmableAction,
            label: "Delete MQTT Client".to_string(),
            required_permission: Some("update_system_services".to_string()),
            input_schema: Some(surfaces::SchemaContract::Object),
            result_schema: Some(surfaces::SchemaContract::Null),
            sensitive_fields: Vec::new(),
            timeout_seconds: Some(30),
            confirmation: Some(InteractionConfirmation {
                title: "Delete MQTT Client".to_string(),
                message: "Delete this MQTT client configuration?".to_string(),
                confirm_label: Some("Delete".to_string()),
                cancel_label: Some("Cancel".to_string()),
                severity: surfaces::ConfirmationSeverity::Danger,
            }),
            transport: InteractionTransport::ProviderProxied,
            workflow_steps: Vec::new(),
            form_ui: None,
        },
    ]
}

fn build_client_form_ui(pre_load: bool) -> surfaces::FormUiDescriptor {
    let mut form_ui = surfaces::FormUiDescriptor {
        fields: vec![
            surfaces::FormFieldDescriptor {
                key: "id".to_string(),
                label: "MQTT Client UUID".to_string(),
                field_type: "hidden".to_string(),
                required: false,
                placeholder: None,
                help_text: None,
                default_value: None,
                options: Vec::new(),
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "client_id".to_string(),
                label: "MQTT Client ID".to_string(),
                field_type: "text".to_string(),
                required: true,
                placeholder: Some("my-uptrakit-client".to_string()),
                help_text: Some("Unique identifier sent to the MQTT broker.".to_string()),
                default_value: None,
                options: Vec::new(),
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "host".to_string(),
                label: "Broker Host".to_string(),
                field_type: "text".to_string(),
                required: true,
                placeholder: Some("mqtt.example.com".to_string()),
                help_text: None,
                default_value: None,
                options: Vec::new(),
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "port".to_string(),
                label: "Broker Port".to_string(),
                field_type: "number".to_string(),
                required: false,
                placeholder: Some("0".to_string()),
                help_text: Some("0 = use the default port for the selected transport.".to_string()),
                default_value: None,
                options: Vec::new(),
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "transport".to_string(),
                label: "Transport".to_string(),
                field_type: "select".to_string(),
                required: true,
                placeholder: None,
                help_text: None,
                default_value: None,
                options: vec![
                    surfaces::FormSelectOption {
                        value: "tcp".to_string(),
                        label: "TCP (plain)".to_string(),
                    },
                    surfaces::FormSelectOption {
                        value: "tls".to_string(),
                        label: "TLS".to_string(),
                    },
                ],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "topic_prefix".to_string(),
                label: "Topic Prefix".to_string(),
                field_type: "text".to_string(),
                required: true,
                placeholder: Some("uptrakit".to_string()),
                help_text: Some("Base topic path for all published messages.".to_string()),
                default_value: None,
                options: Vec::new(),
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "username".to_string(),
                label: "Username".to_string(),
                field_type: "text".to_string(),
                required: false,
                placeholder: None,
                help_text: None,
                default_value: None,
                options: Vec::new(),
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "password".to_string(),
                label: "Password".to_string(),
                field_type: "password".to_string(),
                required: false,
                placeholder: None,
                help_text: None,
                default_value: None,
                options: Vec::new(),
                select_source: None,
                sensitive: true,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "ca_pem".to_string(),
                label: "Custom CA Certificate (PEM)".to_string(),
                field_type: "textarea".to_string(),
                required: false,
                placeholder: None,
                help_text: Some(
                    "Optional PEM-encoded CA certificate for broker TLS verification.".to_string(),
                ),
                default_value: None,
                options: Vec::new(),
                select_source: None,
                sensitive: true,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "ha_discovery".to_string(),
                label: "Enable HA Discovery".to_string(),
                field_type: "toggle".to_string(),
                required: false,
                placeholder: None,
                help_text: Some("Publish Home Assistant MQTT discovery topics.".to_string()),
                default_value: None,
                options: Vec::new(),
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "ha_discovery_prefix".to_string(),
                label: "HA Discovery Prefix".to_string(),
                field_type: "text".to_string(),
                required: false,
                placeholder: Some("homeassistant".to_string()),
                help_text: Some("Topic prefix for HA discovery messages.".to_string()),
                default_value: None,
                options: Vec::new(),
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
            surfaces::FormFieldDescriptor {
                key: "enabled".to_string(),
                label: "Enabled".to_string(),
                field_type: "toggle".to_string(),
                required: false,
                placeholder: None,
                help_text: None,
                default_value: Some("true".to_string()),
                options: Vec::new(),
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            },
        ],
        pre_load_interaction_id: None,
    };

    if pre_load {
        form_ui.pre_load_interaction_id =
            Some(InteractionId::new(ACTION_GET).expect("interaction id is valid"));
    }
    form_ui
}
