//! Shared-surface registration and interaction helpers for the MQTT service.
//!
//! The MQTT service exposes a settings tab that lets users create, edit, list,
//! and delete MQTT client configurations stored in the service config store.
//!
//! Sensitive fields (`password`, `ca_pem`) are preserved as provider-proxied
//! interaction fields so the frontend can submit encrypted sensitive payloads.

use uptrakit_wire::{
    ServiceMessage,
    surfaces::{
        self, ActionRef, Capability, CapabilitySet, DataSourceDescriptor, DataSourceId,
        DataSourceKind, DataSourcePagination, FrameworkGeneration, InteractionConfirmation,
        InteractionDescriptor, InteractionId, InteractionKind, InteractionTransport,
        ProviderEncryptionAlgorithm, ProviderEncryptionMetadata, RefreshPolicy, SurfaceActionError,
        SurfaceActionErrorCode, SurfaceActionRequest, SurfaceActionResponse, SurfaceDescriptor,
        SurfaceId, SurfaceNode, SurfaceRegistration, SurfaceRowCondition, SurfaceRowVisibleWhen,
        SurfaceTableColumn, SurfaceTableRowAction,
    },
};

/// Surface and interaction IDs — kept as constants to avoid magic strings.
pub(crate) const EXT_ID: &str = "mqtt.clients";
/// REST-noun id shared across all four HTTP methods (GET merged list/get,
/// POST create, PUT edit, DELETE delete); each registration is disambiguated
/// by its `http_method`.
pub(crate) const ACTION_CLIENTS: &str = "clients";
const DATA_SOURCE_CLIENTS: &str = "clients";
const LIST_DEFAULT_PAGE: u64 = 1;
const LIST_DEFAULT_PER_PAGE: u64 = 50;
const LIST_MAX_PER_PAGE: u64 = 200;

/// Build the `ProviderIdentity` block shared by every MQTT surface
/// registration (populated and empty alike).
fn provider_identity(service_id: Option<uuid::Uuid>) -> surfaces::ProviderIdentity {
    let app_name = crate::bootstrap::MQTT_SERVICE_APP_NAME;
    let provider_id = service_id
        .map(|id| format!("service.{app_name}.{id}"))
        .unwrap_or_else(|| format!("service.{app_name}"));
    surfaces::ProviderIdentity {
        provider_id,
        provider_kind: surfaces::ProviderKind::Service,
        provider_namespace: "service".to_string(),
    }
}

/// Build an empty registration (`surfaces: vec![]`) for the same provider,
/// used to relinquish the surface — e.g. when yielding to an external MQTT
/// service.
pub fn build_empty_surface_registration(service_id: Option<uuid::Uuid>) -> SurfaceRegistration {
    SurfaceRegistration {
        provider: provider_identity(service_id),
        framework_generation: FrameworkGeneration::new(1, 0),
        capabilities: CapabilitySet::default(),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Global,
            tenant_id: None,
        },
        surfaces: Vec::new(),
        encryption_metadata: None,
    }
}

#[expect(
    clippy::expect_used,
    reason = "infallible: all IDs and interaction IDs are compile-time-valid constants; a parse failure indicates a programming error"
)]
pub(crate) fn build_surface_registration_with_ids(
    encryption_public_key: Option<String>,
    service_id: Option<uuid::Uuid>,
) -> SurfaceRegistration {
    let scope = surfaces::Scope::Global;
    let targeting = surfaces::Targeting::Universal;
    let binding_scope = surfaces::Scope::Global;
    let binding_tenant_id: Option<String> = None;

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

    let data_source_id = DataSourceId::new(DATA_SOURCE_CLIENTS).expect("data source id is valid");
    let registered_surface = surfaces::RegisteredSurface {
        descriptor: SurfaceDescriptor::builder()
            .surface_id(SurfaceId::new(EXT_ID).expect("surface id is valid"))
            .label("MQTT Clients")
            .priority(100)
            .slot(surfaces::SLOT_SETTINGS_TABS)
            .scope(scope)
            .targeting(targeting)
            .required_action(uptrakit_shared_types::access::actions::SYSTEM_SERVICES_UPDATE)
            .provider_kind(surfaces::ProviderKind::Service)
            .required_capabilities(required_capabilities.clone())
            .root_node(SurfaceNode::section(
                None::<String>,
                vec![
                    SurfaceNode::ActionBar {
                        action_ids: vec![ActionRef::WithMethod {
                            interaction_id: InteractionId::new(ACTION_CLIENTS)
                                .expect("interaction id is valid"),
                            http_method: Some(surfaces::InteractionHttpMethod::Post),
                        }],
                    },
                    SurfaceNode::Table {
                        data_source_id: data_source_id.clone(),
                        columns: vec![
                            SurfaceTableColumn::new("client_id", "Client ID"),
                            SurfaceTableColumn::new("host", "Broker Host"),
                            SurfaceTableColumn::new("transport", "Transport"),
                            SurfaceTableColumn::new("enabled", "Enabled"),
                            SurfaceTableColumn::new("ha_discovery", "HA Discovery"),
                        ],
                        row_actions: vec![
                            SurfaceTableRowAction {
                                interaction_id: InteractionId::new(ACTION_CLIENTS)
                                    .expect("interaction id is valid"),
                                http_method: Some(surfaces::InteractionHttpMethod::Put),
                                visible_when: None,
                            },
                            SurfaceTableRowAction {
                                interaction_id: InteractionId::new(ACTION_CLIENTS)
                                    .expect("interaction id is valid"),
                                http_method: Some(surfaces::InteractionHttpMethod::Delete),
                                visible_when: Some(SurfaceRowVisibleWhen {
                                    field: "client_id".to_string(),
                                    condition: SurfaceRowCondition::Present,
                                }),
                            },
                        ],
                    },
                ],
            ))
            .build(),
        interactions: build_interactions(),
        data_sources: vec![DataSourceDescriptor {
            data_source_id,
            kind: DataSourceKind::ProviderQuery {
                operation_id: ACTION_CLIENTS.to_string(),
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

    SurfaceRegistration {
        provider: provider_identity(service_id),
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
    }
}

/// Handle a `clients` GET request: merged list/item read (REST method model).
///
/// Absent `params["id"]` returns the paginated table shape (list of all
/// configs for the request tenant); a present `id` returns the single-client
/// shape (non-sensitive fields for the matching config, cross-tenant lookups
/// rejected). Returns `None` for a present-but-unresolvable `id`.
pub(crate) fn handle_clients_action(
    request: &SurfaceActionRequest,
    tenant_id: uuid::Uuid,
    configs: &[crate::client_manager::ParsedMqttClientConfig],
) -> Option<SurfaceActionResponse> {
    if let Some(id) = request.params.get("id").and_then(|v| v.as_str()) {
        return handle_get_item(request, tenant_id, configs, id);
    }
    Some(handle_list_items(request, tenant_id, configs))
}

/// Single-client shape: non-sensitive fields for the matching config.
///
/// Returns `None` if no config with the given `id` exists for the request
/// tenant (rejects cross-tenant lookups byte-for-byte with the pre-merge
/// behavior).
fn handle_get_item(
    request: &SurfaceActionRequest,
    tenant_id: uuid::Uuid,
    configs: &[crate::client_manager::ParsedMqttClientConfig],
    id: &str,
) -> Option<SurfaceActionResponse> {
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

/// Paginated table shape: a JSON summary of all current MQTT client
/// configurations for the request tenant.
fn handle_list_items(
    request: &SurfaceActionRequest,
    tenant_id: uuid::Uuid,
    configs: &[crate::client_manager::ParsedMqttClientConfig],
) -> SurfaceActionResponse {
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

    SurfaceActionResponse {
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
    }
}

/// Send an error response back to the controller for an unhandled or failed action.
pub(crate) async fn send_error_response(
    transport: &mut dyn uptrakit_wire::ServiceTransport,
    request_id: uuid::Uuid,
    code: SurfaceActionErrorCode,
    message: impl Into<String>,
) -> Result<(), uptrakit_wire::TransportError> {
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
#[expect(
    clippy::items_after_test_module,
    reason = "items after mod tests are required by the surface_runtime module structure"
)]
mod tests {
    use super::*;
    use crate::client_manager::ParsedMqttClientConfig;
    use uptrakit_wire::SecretString;
    use uuid::Uuid;

    #[test]
    fn provider_id_embeds_the_single_source_app_name() {
        let service_id = Uuid::now_v7();

        let without_service = build_surface_registration_with_ids(None, None);
        assert_eq!(
            without_service.provider.provider_id,
            format!("service.{}", crate::bootstrap::MQTT_SERVICE_APP_NAME)
        );

        let with_service = build_surface_registration_with_ids(None, Some(service_id));
        assert_eq!(
            with_service.provider.provider_id,
            format!(
                "service.{}.{service_id}",
                crate::bootstrap::MQTT_SERVICE_APP_NAME
            )
        );
    }

    #[test]
    fn registration_places_surface_in_settings_tab() {
        let payload = build_surface_registration_with_ids(Some("test-key".to_string()), None);
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
        let registration = build_surface_registration_with_ids(None, None);
        let interactions = &registration.surfaces[0].interactions;
        let edit = interactions
            .iter()
            .find(|interaction| {
                interaction.interaction_id.as_str() == ACTION_CLIENTS
                    && interaction.http_method == surfaces::InteractionHttpMethod::Put
            })
            .expect("edit interaction");

        assert_eq!(edit.kind, InteractionKind::FormSubmit);
        assert_eq!(
            edit.form_ui
                .as_ref()
                .and_then(|ui| ui.pre_load_interaction_id.as_ref())
                .map(InteractionId::as_str),
            Some(ACTION_CLIENTS)
        );
    }

    #[test]
    fn mutating_interactions_publish_their_actual_result_schema() {
        let registration = build_surface_registration_with_ids(None, None);
        let interactions = &registration.surfaces[0].interactions;
        let create = interactions
            .iter()
            .find(|interaction| {
                interaction.interaction_id.as_str() == ACTION_CLIENTS
                    && interaction.kind == InteractionKind::FormSubmit
                    && interaction.http_method == surfaces::InteractionHttpMethod::Post
            })
            .expect("create interaction");
        let edit = interactions
            .iter()
            .find(|interaction| {
                interaction.interaction_id.as_str() == ACTION_CLIENTS
                    && interaction.http_method == surfaces::InteractionHttpMethod::Put
            })
            .expect("edit interaction");
        let delete = interactions
            .iter()
            .find(|interaction| {
                interaction.interaction_id.as_str() == ACTION_CLIENTS
                    && interaction.http_method == surfaces::InteractionHttpMethod::Delete
            })
            .expect("delete interaction");

        assert_eq!(create.result_schema, Some(surfaces::SchemaContract::Object));
        assert_eq!(edit.result_schema, Some(surfaces::SchemaContract::Null));
        assert_eq!(delete.result_schema, Some(surfaces::SchemaContract::Null));
    }

    #[test]
    fn registration_is_global_and_universal_with_no_tenant_binding() {
        let registration = build_surface_registration_with_ids(None, None);

        let surface = &registration.surfaces[0].descriptor;
        assert_eq!(surface.scope, surfaces::Scope::Global);
        assert_eq!(surface.targeting, surfaces::Targeting::Universal);
        assert_eq!(
            registration.effective_tenant_binding.scope,
            surfaces::Scope::Global
        );
        assert_eq!(registration.effective_tenant_binding.tenant_id, None);
        assert!(
            registration
                .capabilities
                .0
                .contains(&Capability::UniversalTargeting)
        );
        assert!(
            !registration
                .capabilities
                .0
                .contains(&Capability::TargetedTargeting)
        );
    }

    #[test]
    fn registration_preserves_provider_surface_and_action_identity() {
        let service_id = Uuid::now_v7();
        let registration = build_surface_registration_with_ids(None, Some(service_id));

        assert_eq!(
            registration.provider.provider_id,
            format!("service.uptrakit-mqtt.{service_id}")
        );
        let surface = &registration.surfaces[0].descriptor;
        assert_eq!(surface.surface_id.as_str(), EXT_ID);
        assert_eq!(surface.slot, surfaces::SLOT_SETTINGS_TABS);
        assert_eq!(
            surface.required_action.as_deref(),
            Some(uptrakit_shared_types::access::actions::SYSTEM_SERVICES_UPDATE_STR)
        );
    }

    #[test]
    fn registration_encryption_metadata_tracks_public_key_presence() {
        let with_key = build_surface_registration_with_ids(Some("test-key".to_string()), None);
        assert!(with_key.encryption_metadata.is_some());

        let without_key = build_surface_registration_with_ids(None, None);
        assert!(without_key.encryption_metadata.is_none());
    }

    #[test]
    fn empty_registration_carries_same_provider_block_with_no_surfaces() {
        let service_id = Uuid::now_v7();
        let full = build_surface_registration_with_ids(None, Some(service_id));
        let empty = build_empty_surface_registration(Some(service_id));

        assert_eq!(empty.provider, full.provider);
        assert!(empty.surfaces.is_empty());
    }

    #[test]
    fn clients_action_with_id_present_returns_single_client_shape_and_omits_sensitive_fields() {
        let tenant_id = Uuid::now_v7();
        let request = SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: tenant_id.to_string(),
            surface_id: SurfaceId::new(EXT_ID).expect("surface id"),
            interaction_id: InteractionId::new(ACTION_CLIENTS).expect("interaction id"),
            method: surfaces::InteractionHttpMethod::Get,
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

        let response = handle_clients_action(&request, tenant_id, &configs).expect("response");
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
        // single-client shape has no `items`/`total` list envelope
        assert!(!data.contains_key("items"));
        assert!(!data.contains_key("total"));
    }

    #[test]
    fn clients_action_with_id_absent_returns_list_shape_filtered_to_request_tenant() {
        let tenant_a = Uuid::now_v7();
        let tenant_b = Uuid::now_v7();
        let request = SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: tenant_a.to_string(),
            surface_id: SurfaceId::new(EXT_ID).expect("surface id"),
            interaction_id: InteractionId::new(ACTION_CLIENTS).expect("interaction id"),
            method: surfaces::InteractionHttpMethod::Get,
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

        let response = handle_clients_action(&request, tenant_a, &configs).expect("response");
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
    fn clients_action_with_id_absent_returns_paginated_table_shape() {
        let tenant_id = Uuid::now_v7();
        let request = SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: tenant_id.to_string(),
            surface_id: SurfaceId::new(EXT_ID).expect("surface id"),
            interaction_id: InteractionId::new(ACTION_CLIENTS).expect("interaction id"),
            method: surfaces::InteractionHttpMethod::Get,
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

        let response = handle_clients_action(&request, tenant_id, &configs).expect("response");
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
    fn clients_action_with_id_absent_handles_huge_page_without_overflow() {
        let tenant_id = Uuid::now_v7();
        let request = SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: tenant_id.to_string(),
            surface_id: SurfaceId::new(EXT_ID).expect("surface id"),
            interaction_id: InteractionId::new(ACTION_CLIENTS).expect("interaction id"),
            method: surfaces::InteractionHttpMethod::Get,
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

        let response = handle_clients_action(&request, tenant_id, &configs).expect("response");
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
    fn clients_action_with_id_present_rejects_cross_tenant_lookup() {
        let tenant_a = Uuid::now_v7();
        let tenant_b = Uuid::now_v7();
        let target_id = Uuid::parse_str("019471a0-0000-7000-8000-000000000001").unwrap();
        let request = SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: tenant_a.to_string(),
            surface_id: SurfaceId::new(EXT_ID).expect("surface id"),
            interaction_id: InteractionId::new(ACTION_CLIENTS).expect("interaction id"),
            method: surfaces::InteractionHttpMethod::Get,
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

        assert!(handle_clients_action(&request, tenant_a, &configs).is_none());
    }

    /// Guard: every declared `required_action` on the MQTT registration
    /// (surface-level and interaction-level) parses against the catalog
    /// `Action` type — catches field-write string seams
    /// (`Some(actions::X.to_string())` refactors gone wrong), not presence.
    #[test]
    fn declared_required_action_values_parse_against_the_catalog() {
        let registration = build_surface_registration_with_ids(None, None);
        let mut checked = 0usize;
        for surface in &registration.surfaces {
            let surface_id = surface.descriptor.surface_id.as_str();
            if let Some(required_action) = surface.descriptor.required_action.as_deref() {
                checked += 1;
                assert!(
                    required_action
                        .parse::<uptrakit_shared_types::access::Action>()
                        .is_ok(),
                    "surface `{surface_id}` declares invalid required_action `{required_action}`"
                );
            }
            for interaction in &surface.interactions {
                if let Some(required_action) = interaction.required_action.as_deref() {
                    checked += 1;
                    let interaction_id = interaction.interaction_id.as_str();
                    assert!(
                        required_action
                            .parse::<uptrakit_shared_types::access::Action>()
                            .is_ok(),
                        "interaction `{interaction_id}` on `{surface_id}` declares invalid required_action `{required_action}`"
                    );
                }
            }
        }
        assert!(
            checked > 0,
            "no required_action values found in the MQTT registration — guard is vacuous"
        );
    }

    /// REST-noun convention guard: surface/interaction/data-source ids stay
    /// kebab-case, and every `ProviderQuery` data source's `operation_id`
    /// resolves to a GET interaction sharing the data source's id.
    #[test]
    fn mqtt_surface_ids_follow_kebab_convention() {
        fn is_kebab(s: &str) -> bool {
            let bytes = s.as_bytes();
            if bytes.is_empty() || !bytes[0].is_ascii_lowercase() {
                return false;
            }
            let mut prev_dash = false;
            for (i, &b) in bytes.iter().enumerate() {
                match b {
                    b'a'..=b'z' | b'0'..=b'9' => prev_dash = false,
                    b'-' => {
                        if i == 0 || prev_dash || i == bytes.len() - 1 {
                            return false;
                        }
                        prev_dash = true;
                    }
                    _ => return false,
                }
            }
            true
        }

        let registration = build_surface_registration_with_ids(None, None);
        let surface = &registration.surfaces[0];

        for segment in surface.descriptor.surface_id.as_str().split('.') {
            assert!(
                is_kebab(segment),
                "surface id segment `{segment}` is not kebab-case"
            );
        }
        assert!(
            !surface.descriptor.surface_id.as_str().starts_with("mqtt-"),
            "surface id must not carry an `mqtt-` prefix on its segments"
        );

        for interaction in &surface.interactions {
            let id = interaction.interaction_id.as_str();
            assert!(is_kebab(id), "interaction id `{id}` is not kebab-case");
            assert!(
                !id.starts_with("mqtt."),
                "interaction id `{id}` must not carry the `mqtt.` prefix"
            );
        }

        for data_source in &surface.data_sources {
            let id = data_source.data_source_id.as_str();
            assert!(is_kebab(id), "data source id `{id}` is not kebab-case");
            assert!(
                !id.starts_with("mqtt."),
                "data source id `{id}` must not carry the `mqtt.` prefix"
            );

            if let DataSourceKind::ProviderQuery { operation_id } = &data_source.kind {
                let matching = surface.interactions.iter().find(|interaction| {
                    interaction.interaction_id.as_str() == operation_id
                        && interaction.effective_http_method()
                            == surfaces::InteractionHttpMethod::Get
                });
                assert!(
                    matching.is_some(),
                    "data source `{id}` operation_id `{operation_id}` must resolve to a GET interaction"
                );
                assert_eq!(
                    id, operation_id,
                    "data source id must equal its GET interaction's operation_id"
                );
            }
        }
    }
}

#[expect(
    clippy::expect_used,
    reason = "infallible: all interaction IDs are compile-time-valid constants"
)]
fn build_interactions() -> Vec<InteractionDescriptor> {
    vec![
        {
            let mut i = InteractionDescriptor::new(
                InteractionId::new(ACTION_CLIENTS).expect("interaction id is valid"),
                InteractionKind::DataLoad,
                "List MQTT Clients",
                InteractionTransport::ProviderProxied,
            );
            i.required_action = Some(
                uptrakit_shared_types::access::actions::SYSTEM_SERVICES_UPDATE_STR.to_string(),
            );
            i.input_schema = Some(surfaces::SchemaContract::Object);
            // merged list/item read — two result shapes (`SchemaContract` has
            // no union/oneOf variant).
            i.result_schema = Some(surfaces::SchemaContract::Any);
            i.timeout_seconds = Some(30);
            i
        },
        {
            let mut i = InteractionDescriptor::new(
                InteractionId::new(ACTION_CLIENTS).expect("interaction id is valid"),
                InteractionKind::FormSubmit,
                "Add MQTT Client",
                InteractionTransport::ProviderProxied,
            );
            i.required_action = Some(
                uptrakit_shared_types::access::actions::SYSTEM_SERVICES_UPDATE_STR.to_string(),
            );
            i.input_schema = Some(surfaces::SchemaContract::Object);
            i.result_schema = Some(surfaces::SchemaContract::Object);
            i.sensitive_fields = vec!["password".to_string(), "ca_pem".to_string()];
            i.timeout_seconds = Some(30);
            i.form_ui = Some(build_client_form_ui(false));
            i
        },
        {
            let mut i = InteractionDescriptor::new(
                InteractionId::new(ACTION_CLIENTS).expect("interaction id is valid"),
                InteractionKind::FormSubmit,
                "Edit MQTT Client",
                InteractionTransport::ProviderProxied,
            );
            i.http_method = surfaces::InteractionHttpMethod::Put;
            i.required_action = Some(
                uptrakit_shared_types::access::actions::SYSTEM_SERVICES_UPDATE_STR.to_string(),
            );
            i.input_schema = Some(surfaces::SchemaContract::Object);
            i.result_schema = Some(surfaces::SchemaContract::Null);
            i.sensitive_fields = vec!["password".to_string(), "ca_pem".to_string()];
            i.timeout_seconds = Some(30);
            i.form_ui = Some(build_client_form_ui(true));
            i
        },
        {
            let mut i = InteractionDescriptor::new(
                InteractionId::new(ACTION_CLIENTS).expect("interaction id is valid"),
                InteractionKind::ConfirmableAction,
                "Delete MQTT Client",
                InteractionTransport::ProviderProxied,
            );
            i.http_method = surfaces::InteractionHttpMethod::Delete;
            i.required_action = Some(
                uptrakit_shared_types::access::actions::SYSTEM_SERVICES_UPDATE_STR.to_string(),
            );
            i.input_schema = Some(surfaces::SchemaContract::Object);
            i.result_schema = Some(surfaces::SchemaContract::Null);
            i.timeout_seconds = Some(30);
            i.confirmation = Some(InteractionConfirmation {
                title: "Delete MQTT Client".to_string(),
                message: "Delete this MQTT client configuration?".to_string(),
                confirm_label: Some("Delete".to_string()),
                cancel_label: Some("Cancel".to_string()),
                severity: surfaces::ConfirmationSeverity::Danger,
            });
            i
        },
    ]
}

#[expect(
    clippy::expect_used,
    reason = "infallible: interaction ID is a compile-time-valid constant"
)]
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
            Some(InteractionId::new(ACTION_CLIENTS).expect("interaction id is valid"));
    }
    form_ui
}
