// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
//! `WireValidate` implementations for all wire protocol payload structs.
//!
//! Separated from `lib.rs` for readability. Each impl validates the struct's
//! own fields and delegates to nested structs that also implement `WireValidate`.
use crate::generated::wire::limits::*;
use crate::generated::wire::*;
fn validate_report_page_limit(
    value: u32,
    max: u32,
    field: &'static str,
) -> Result<(), WireValidationError> {
    if value == 0 || value > max {
        return Err(WireValidationError {
            field,
            message: format!("value is {value}, must be 1..={max}"),
        });
    }
    Ok(())
}
impl WireValidate for RegisterPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        Ok(())
    }
}
impl WireValidate for ServiceMessage {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        match self {
            ServiceMessage::Ping(_) => Ok(()),
            ServiceMessage::Register(_) => Ok(()),
            ServiceMessage::Enroll(p) => p.wire_validate(),
            ServiceMessage::RequestCertificate(p) => p.wire_validate(),
            ServiceMessage::RenewCertificate(p) => p.wire_validate(),
            ServiceMessage::ReportHosts(p) => p.wire_validate(),
            ServiceMessage::VersionCheckResults(p) => p.wire_validate(),
            ServiceMessage::UpdateStarted(p) => p.wire_validate(),
            ServiceMessage::UpdateOutput(p) => p.wire_validate(),
            ServiceMessage::UpdateResult(p) => p.wire_validate(),
            ServiceMessage::BatchUpdateResult(p) => p.wire_validate(),
            ServiceMessage::DiscoveryResults(p) => p.wire_validate(),
            ServiceMessage::StdinAttention(p) => p.wire_validate(),
            ServiceMessage::ServiceTriggerUpdate(p) => p.wire_validate(),
            ServiceMessage::ServiceTriggerHostBatchUpdate(_) => Ok(()),
            ServiceMessage::Disconnecting(p) => p.wire_validate(),
            ServiceMessage::ReportPluginConfig(p) => p.wire_validate(),
            ServiceMessage::SurfaceRegistration(p) => p.wire_validate(),
            ServiceMessage::SurfaceActionResponse(p) => p.wire_validate(),
            ServiceMessage::SurfaceActionRequest(p) => p.wire_validate(),
            ServiceMessage::StoreServiceConfig(p) => p.wire_validate(),
            ServiceMessage::DeleteServiceConfig(p) => p.wire_validate(),
            ServiceMessage::WorkloadClaim(p) => p.wire_validate(),
            ServiceMessage::WorkloadRelease(p) => p.wire_validate(),
            ServiceMessage::TestPluginConfigResult(p) => p.wire_validate(),
            ServiceMessage::AuditEvent(_) => Ok(()),
            _ => {
                tracing::debug!(
                    "received unknown ServiceMessage variant from peer; skipping validation"
                );
                Ok(())
            }
        }
    }
}
impl WireValidate for ControllerMessage {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        match self {
            ControllerMessage::Pong(_) => Ok(()),
            ControllerMessage::Enrolled(_) => Ok(()),
            ControllerMessage::Approved(_) => Ok(()),
            ControllerMessage::Rejected(_) => Ok(()),
            ControllerMessage::Certificate(p) => p.wire_validate(),
            ControllerMessage::Error(p) => p.wire_validate(),
            ControllerMessage::ServiceSettings(p) => p.wire_validate(),
            ControllerMessage::CaBundleUpdated(p) => p.wire_validate(),
            ControllerMessage::RequestCertRenewal(p) => p.wire_validate(),
            ControllerMessage::ServerRestarting(p) => p.wire_validate(),
            ControllerMessage::CheckVersions(p) => p.wire_validate(),
            ControllerMessage::ExecuteUpdate(p) => p.wire_validate(),
            ControllerMessage::ExecuteBatchUpdate(p) => p.wire_validate(),
            ControllerMessage::DiscoverSoftware(p) => p.wire_validate(),
            ControllerMessage::SetUpdateFreeze(p) => p.wire_validate(),
            ControllerMessage::UpdateStdinData(p) => p.wire_validate(),
            ControllerMessage::SoftwareStates(p) => p.wire_validate(),
            ControllerMessage::HostConnectivityUpdated(p) => p.wire_validate(),
            ControllerMessage::ReportPluginConfigResponse(p) => p.wire_validate(),
            ControllerMessage::SurfaceActionRequest(p) => p.wire_validate(),
            ControllerMessage::SurfaceActionCancel(p) => p.wire_validate(),
            ControllerMessage::SurfaceActionResponse(p) => p.wire_validate(),
            ControllerMessage::ServiceCredentials(_) => Ok(()),
            ControllerMessage::ServiceConfigDelivery(p) => p.wire_validate(),
            ControllerMessage::ServiceConfigAck(p) => p.wire_validate(),
            ControllerMessage::ServiceConfigUpdated(p) => p.wire_validate(),
            ControllerMessage::RequestCaRotation(p) => p.wire_validate(),
            ControllerMessage::RequestCrlRenewal(_) => Ok(()),
            ControllerMessage::TokenRevoked(_) => Ok(()),
            ControllerMessage::WorkloadClaimResult(p) => p.wire_validate(),
            ControllerMessage::WorkloadClaimAnnouncement(p) => p.wire_validate(),
            ControllerMessage::WorkloadClaimSyncRequest(_) => Ok(()),
            ControllerMessage::WorkloadClaimSyncResponse(p) => p.wire_validate(),
            ControllerMessage::TestPluginConfig(p) => p.wire_validate(),
            _ => {
                tracing::debug!(
                    "received unknown ControllerMessage variant from peer; skipping validation"
                );
                Ok(())
            }
        }
    }
}
impl WireValidate for crate::generated::wire::envelope::ReportPagination {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        if self.total_pages == 0 || self.total_pages > MAX_REPORT_PAGES {
            return Err(WireValidationError {
                field: "pagination.total_pages",
                message: format!(
                    "total_pages is {}, must be 1..={MAX_REPORT_PAGES}",
                    self.total_pages
                ),
            });
        }
        if self.page == 0 || self.page > self.total_pages {
            return Err(WireValidationError {
                field: "pagination.page",
                message: format!("page is {}, must be 1..={}", self.page, self.total_pages),
            });
        }
        Ok(())
    }
}
impl WireValidate for EnrollPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.hostname, MAX_SHORT_STRING_LEN, "hostname")?;
        check_string_len(&self.friendly_name, MAX_SHORT_STRING_LEN, "friendly_name")?;
        check_string_len(
            &self.service_app_name,
            MAX_SHORT_STRING_LEN,
            "service_app_name",
        )?;
        Ok(())
    }
}
impl WireValidate for RequestCertificatePayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.csr_pem, MAX_LONG_STRING_LEN, "csr_pem")?;
        Ok(())
    }
}
impl WireValidate for RenewCertificatePayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.csr_pem, MAX_LONG_STRING_LEN, "csr_pem")?;
        Ok(())
    }
}
impl WireValidate for ReportHostsPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_vec_len(&self.hosts, MAX_REPORT_HOSTS, "hosts")?;
        check_string_len(&self.agent_version, MAX_SHORT_STRING_LEN, "agent_version")?;
        for host in &self.hosts {
            host.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for HostInfo {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.machine_id, MAX_SHORT_STRING_LEN, "machine_id")?;
        check_opt_string_len(&self.os_type, MAX_SHORT_STRING_LEN, "os_type")?;
        check_opt_string_len(&self.os_version, MAX_SHORT_STRING_LEN, "os_version")?;
        check_opt_string_len(&self.architecture, MAX_SHORT_STRING_LEN, "architecture")?;
        check_opt_string_len(&self.hostname, MAX_SHORT_STRING_LEN, "hostname")?;
        check_opt_string_len(&self.ip_address, MAX_SHORT_STRING_LEN, "ip_address")?;
        Ok(())
    }
}
impl WireValidate for VersionCheckResultsPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_vec_len(&self.results, MAX_VERSION_CHECK_RESULTS, "results")?;
        for result in &self.results {
            result.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for VersionCheckResult {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_opt_string_len(
            &self.installed_version,
            MAX_SHORT_STRING_LEN,
            "installed_version",
        )?;
        check_opt_string_len(&self.latest_version, MAX_SHORT_STRING_LEN, "latest_version")?;
        check_opt_string_len(&self.error, MAX_MEDIUM_STRING_LEN, "error")?;
        Ok(())
    }
}
impl WireValidate for UpdateStartedPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_opt_string_len(&self.from_version, MAX_SHORT_STRING_LEN, "from_version")?;
        Ok(())
    }
}
impl WireValidate for UpdateOutputPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.output, MAX_OUTPUT_STRING_LEN, "output")?;
        Ok(())
    }
}
impl WireValidate for UpdateResultPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.output, MAX_OUTPUT_STRING_LEN, "output")?;
        check_opt_string_len(&self.from_version, MAX_SHORT_STRING_LEN, "from_version")?;
        check_opt_string_len(&self.to_version, MAX_SHORT_STRING_LEN, "to_version")?;
        check_opt_string_len(&self.error, MAX_MEDIUM_STRING_LEN, "error")?;
        Ok(())
    }
}
impl WireValidate for BatchUpdateResultPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_vec_len(&self.results, MAX_BATCH_UPDATE_RESULTS, "results")?;
        for result in &self.results {
            result.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for BatchUpdateItemResult {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.output, MAX_OUTPUT_STRING_LEN, "output")?;
        check_opt_string_len(
            &self.installed_version,
            MAX_SHORT_STRING_LEN,
            "installed_version",
        )?;
        check_opt_string_len(&self.error, MAX_MEDIUM_STRING_LEN, "error")?;
        Ok(())
    }
}
impl WireValidate for DiscoveryResultsPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(
            &self.host_machine_id,
            MAX_SHORT_STRING_LEN,
            "host_machine_id",
        )?;
        check_vec_len(&self.results, MAX_DISCOVERY_PLUGIN_RESULTS, "results")?;
        for result in &self.results {
            result.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for DiscoveryPluginResult {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_vec_len(&self.discoveries, MAX_DISCOVERIES_PER_PLUGIN, "discoveries")?;
        check_opt_string_len(&self.error, MAX_MEDIUM_STRING_LEN, "error")?;
        for discovery in &self.discoveries {
            discovery.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for crate::generated::shared_types::DiscoveredSoftware {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(
            &self.package_identifier,
            MAX_SHORT_STRING_LEN,
            "package_identifier",
        )?;
        check_string_len(&self.name, MAX_SHORT_STRING_LEN, "name")?;
        check_string_len(
            &self.installed_version,
            MAX_SHORT_STRING_LEN,
            "installed_version",
        )?;
        check_opt_string_len(&self.qualifier, MAX_DISCOVERED_QUALIFIER_LEN, "qualifier")?;
        check_opt_string_len(
            &self.plugin_package_identifier,
            MAX_SHORT_STRING_LEN,
            "plugin_package_identifier",
        )?;
        Ok(())
    }
}
impl WireValidate for ServiceUpdateTriggerPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.to_version, MAX_SHORT_STRING_LEN, "to_version")?;
        Ok(())
    }
}
impl WireValidate for DisconnectingPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        Ok(())
    }
}
fn validate_surface_json_bounds(
    value: &serde_json::Value,
    field: &'static str,
) -> Result<(), WireValidationError> {
    let mut node_count = 0usize;
    fn walk(
        value: &serde_json::Value,
        depth: usize,
        node_count: &mut usize,
        field: &'static str,
    ) -> Result<(), WireValidationError> {
        if depth > MAX_SURFACE_JSON_DEPTH {
            return Err(WireValidationError {
                field,
                message: format!(
                    "JSON depth exceeds max {MAX_SURFACE_JSON_DEPTH} (observed depth {depth})"
                ),
            });
        }
        *node_count += 1;
        if *node_count > MAX_SURFACE_JSON_NODES {
            return Err(WireValidationError {
                field,
                message: format!("JSON node count exceeds max {MAX_SURFACE_JSON_NODES}"),
            });
        }
        match value {
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, depth + 1, node_count, field)?;
                }
            }
            serde_json::Value::Object(map) => {
                for item in map.values() {
                    walk(item, depth + 1, node_count, field)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(value, 1, &mut node_count, field)
}
fn validate_surface_node(
    node: &surfaces::SurfaceNode,
    depth: usize,
) -> Result<(), WireValidationError> {
    if depth > MAX_SURFACE_JSON_DEPTH {
        return Err(WireValidationError {
            field: "surfaces[].descriptor.root_node",
            message: format!(
                "root node depth exceeds max {MAX_SURFACE_JSON_DEPTH} (observed depth {depth})"
            ),
        });
    }
    match node {
        surfaces::SurfaceNode::Section { title, children } => {
            check_opt_string_len(
                title,
                MAX_SHORT_STRING_LEN,
                "surfaces[].descriptor.root_node.title",
            )?;
            check_vec_len(
                children,
                MAX_SURFACE_FIELDS,
                "surfaces[].descriptor.root_node.children",
            )?;
            for child in children {
                validate_surface_node(child, depth + 1)?;
            }
        }
        surfaces::SurfaceNode::TextBlock { text } => {
            check_string_len(
                text,
                MAX_MEDIUM_STRING_LEN,
                "surfaces[].descriptor.root_node.text",
            )?;
        }
        surfaces::SurfaceNode::KeyValue { .. } | surfaces::SurfaceNode::Table { .. } => {}
        surfaces::SurfaceNode::Form { .. } => {}
        surfaces::SurfaceNode::ActionBar { action_ids } => {
            check_vec_len(
                action_ids,
                MAX_SURFACE_ACTION_REFS,
                "surfaces[].descriptor.root_node.action_ids",
            )?;
        }
        surfaces::SurfaceNode::Tabs { tabs } => {
            check_vec_len(
                tabs,
                MAX_SURFACE_COLUMNS,
                "surfaces[].descriptor.root_node.tabs",
            )?;
            for tab in tabs {
                check_string_len(
                    &tab.label,
                    MAX_SHORT_STRING_LEN,
                    "surfaces[].descriptor.root_node.tabs[].label",
                )?;
                validate_surface_node(&tab.root, depth + 1)?;
            }
        }
        surfaces::SurfaceNode::Callout { text, .. } => {
            check_string_len(
                text,
                MAX_MEDIUM_STRING_LEN,
                "surfaces[].descriptor.root_node.callout",
            )?;
        }
        surfaces::SurfaceNode::EmptyState { title, description } => {
            check_string_len(
                title,
                MAX_SHORT_STRING_LEN,
                "surfaces[].descriptor.root_node.empty_state.title",
            )?;
            check_opt_string_len(
                description,
                MAX_MEDIUM_STRING_LEN,
                "surfaces[].descriptor.root_node.empty_state.description",
            )?;
        }
        surfaces::SurfaceNode::ModalTrigger { modal_nodes, .. } => {
            check_vec_len(
                modal_nodes,
                MAX_SURFACE_FIELDS,
                "surfaces[].descriptor.root_node.modal_nodes",
            )?;
            for child in modal_nodes {
                validate_surface_node(child, depth + 1)?;
            }
        }
        surfaces::SurfaceNode::WorkflowTrigger { step_nodes, .. } => {
            check_vec_len(
                step_nodes,
                MAX_SURFACE_WIZARD_STEPS,
                "surfaces[].descriptor.root_node.step_nodes",
            )?;
            for child in step_nodes {
                validate_surface_node(child, depth + 1)?;
            }
        }
        _ => {
            tracing::warn!(
                ?node,
                "unknown SurfaceNode variant; skipping wire validation"
            );
        }
    }
    Ok(())
}
fn validate_surface_interaction(
    interaction: &surfaces::InteractionDescriptor,
) -> Result<(), WireValidationError> {
    check_opt_string_len(
        &interaction.required_permission,
        MAX_SHORT_STRING_LEN,
        "surfaces[].interactions[].required_permission",
    )?;
    check_vec_len(
        &interaction.sensitive_fields,
        MAX_SURFACE_FIELDS,
        "surfaces[].interactions[].sensitive_fields",
    )?;
    for field in &interaction.sensitive_fields {
        check_string_len(
            field,
            MAX_SHORT_STRING_LEN,
            "surfaces[].interactions[].sensitive_fields[]",
        )?;
    }
    if let Some(confirmation) = &interaction.confirmation {
        check_string_len(
            &confirmation.title,
            MAX_SHORT_STRING_LEN,
            "surfaces[].interactions[].confirmation.title",
        )?;
        check_string_len(
            &confirmation.message,
            MAX_MEDIUM_STRING_LEN,
            "surfaces[].interactions[].confirmation.message",
        )?;
        check_opt_string_len(
            &confirmation.confirm_label,
            MAX_SHORT_STRING_LEN,
            "surfaces[].interactions[].confirmation.confirm_label",
        )?;
        check_opt_string_len(
            &confirmation.cancel_label,
            MAX_SHORT_STRING_LEN,
            "surfaces[].interactions[].confirmation.cancel_label",
        )?;
    }
    check_vec_len(
        &interaction.workflow_steps,
        MAX_SURFACE_WIZARD_STEPS,
        "surfaces[].interactions[].workflow_steps",
    )?;
    for step in &interaction.workflow_steps {
        check_string_len(
            &step.step_id,
            MAX_SHORT_STRING_LEN,
            "surfaces[].interactions[].workflow_steps[].step_id",
        )?;
    }
    if let surfaces::InteractionTransport::DirectBuiltInApi { operation_id } =
        &interaction.transport
    {
        check_string_len(
            operation_id.as_str(),
            MAX_SHORT_STRING_LEN,
            "surfaces[].interactions[].transport.operation_id",
        )?;
    }
    Ok(())
}
fn validate_surface_data_source(
    data_source: &surfaces::DataSourceDescriptor,
) -> Result<(), WireValidationError> {
    match &data_source.kind {
        surfaces::DataSourceKind::Static { data } => {
            let data_len = serde_json::to_vec(data)
                .map_err(|error| WireValidationError {
                    field: "surfaces[].data_sources[].kind.static.data",
                    message: format!("failed to serialize static data: {error}"),
                })?
                .len();
            if data_len > MAX_SURFACE_PARAMS_LEN {
                return Err(WireValidationError {
                    field: "surfaces[].data_sources[].kind.static.data",
                    message: format!(
                        "static data JSON is {data_len} bytes, max {MAX_SURFACE_PARAMS_LEN}"
                    ),
                });
            }
            validate_surface_json_bounds(data, "surfaces[].data_sources[].kind.static.data")?;
        }
        surfaces::DataSourceKind::ControllerQuery { .. } => {}
        surfaces::DataSourceKind::ProviderQuery { operation_id } => {
            check_string_len(
                operation_id,
                MAX_SHORT_STRING_LEN,
                "surfaces[].data_sources[].kind.provider_query.operation_id",
            )?;
        }
    }
    if let Some(pagination) = &data_source.pagination {
        if pagination.default_page_size == 0 || pagination.max_page_size == 0 {
            return Err(WireValidationError {
                field: "surfaces[].data_sources[].pagination",
                message: "page size values must be greater than zero".to_string(),
            });
        }
        if pagination.default_page_size > pagination.max_page_size {
            return Err(WireValidationError {
                field: "surfaces[].data_sources[].pagination",
                message: "default_page_size cannot exceed max_page_size".to_string(),
            });
        }
    }
    if let Some(sorting) = &data_source.sorting {
        check_vec_len(
            &sorting.sortable_fields,
            MAX_SURFACE_COLUMNS,
            "surfaces[].data_sources[].sorting.sortable_fields",
        )?;
        for field in &sorting.sortable_fields {
            check_string_len(
                field,
                MAX_SHORT_STRING_LEN,
                "surfaces[].data_sources[].sorting.sortable_fields[]",
            )?;
        }
        check_opt_string_len(
            &sorting.default_sort_field,
            MAX_SHORT_STRING_LEN,
            "surfaces[].data_sources[].sorting.default_sort_field",
        )?;
    }
    if let Some(filtering) = &data_source.filtering {
        check_vec_len(
            &filtering.filter_fields,
            MAX_SURFACE_COLUMNS,
            "surfaces[].data_sources[].filtering.filter_fields",
        )?;
        for field in &filtering.filter_fields {
            check_string_len(
                field,
                MAX_SHORT_STRING_LEN,
                "surfaces[].data_sources[].filtering.filter_fields[]",
            )?;
        }
    }
    match &data_source.refresh_policy {
        surfaces::RefreshPolicy::Manual => {}
        surfaces::RefreshPolicy::Interval { seconds } => {
            if *seconds == 0 {
                return Err(WireValidationError {
                    field: "surfaces[].data_sources[].refresh_policy.interval.seconds",
                    message: "interval seconds must be greater than zero".to_string(),
                });
            }
        }
        surfaces::RefreshPolicy::Sse { .. } => {}
    }
    if let Some(empty_state) = &data_source.empty_state {
        check_string_len(
            &empty_state.title,
            MAX_SHORT_STRING_LEN,
            "surfaces[].data_sources[].empty_state.title",
        )?;
        check_opt_string_len(
            &empty_state.description,
            MAX_MEDIUM_STRING_LEN,
            "surfaces[].data_sources[].empty_state.description",
        )?;
    }
    Ok(())
}
impl WireValidate for surfaces::SurfaceRegistration {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(
            &self.provider.provider_id,
            MAX_SHORT_STRING_LEN,
            "provider.provider_id",
        )?;
        check_string_len(
            &self.provider.provider_namespace,
            MAX_SHORT_STRING_LEN,
            "provider.provider_namespace",
        )?;
        check_opt_string_len(
            &self.effective_tenant_binding.tenant_id,
            MAX_SHORT_STRING_LEN,
            "effective_tenant_binding.tenant_id",
        )?;
        if self.effective_tenant_binding.scope == surfaces::Scope::Tenant {
            let tenant_id =
                self.effective_tenant_binding
                    .tenant_id
                    .as_deref()
                    .ok_or(WireValidationError {
                        field: "effective_tenant_binding.tenant_id",
                        message: "tenant scope requires tenant_id".to_string(),
                    })?;
            uuid::Uuid::parse_str(tenant_id).map_err(|error| WireValidationError {
                field: "effective_tenant_binding.tenant_id",
                message: format!("invalid tenant UUID: {error}"),
            })?;
        } else if let Some(tenant_id) = &self.effective_tenant_binding.tenant_id {
            uuid::Uuid::parse_str(tenant_id).map_err(|error| WireValidationError {
                field: "effective_tenant_binding.tenant_id",
                message: format!("invalid tenant UUID: {error}"),
            })?;
        }
        check_vec_len(&self.surfaces, MAX_SURFACE_MANIFESTS, "surfaces")?;
        if let Some(ref metadata) = self.encryption_metadata {
            check_string_len(
                &metadata.key_id,
                MAX_SHORT_STRING_LEN,
                "encryption_metadata.key_id",
            )?;
            check_string_len(
                &metadata.public_key,
                MAX_LONG_STRING_LEN,
                "encryption_metadata.public_key",
            )?;
        }
        for surface in &self.surfaces {
            check_string_len(
                &surface.descriptor.label,
                MAX_SHORT_STRING_LEN,
                "surfaces[].descriptor.label",
            )?;
            check_string_len(
                &surface.descriptor.slot,
                MAX_SHORT_STRING_LEN,
                "surfaces[].descriptor.slot",
            )?;
            check_opt_string_len(
                &surface.descriptor.required_permission,
                MAX_SHORT_STRING_LEN,
                "surfaces[].descriptor.required_permission",
            )?;
            check_vec_len(
                &surface.interactions,
                MAX_SURFACE_ACTIONS,
                "surfaces[].interactions",
            )?;
            check_vec_len(
                &surface.data_sources,
                MAX_SURFACE_FIELDS,
                "surfaces[].data_sources",
            )?;
            validate_surface_node(&surface.descriptor.root_node, 1)?;
            for interaction in &surface.interactions {
                validate_surface_interaction(interaction)?;
            }
            for data_source in &surface.data_sources {
                validate_surface_data_source(data_source)?;
            }
        }
        Ok(())
    }
}
impl WireValidate for surfaces::SurfaceActionRequest {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.tenant_id, MAX_SHORT_STRING_LEN, "tenant_id")?;
        uuid::Uuid::parse_str(&self.tenant_id).map_err(|error| WireValidationError {
            field: "tenant_id",
            message: format!("invalid tenant UUID: {error}"),
        })?;
        check_string_len(
            &self.idempotency_key,
            MAX_SHORT_STRING_LEN,
            "idempotency_key",
        )?;
        check_opt_string_len(
            &self.target_provider_id,
            MAX_SHORT_STRING_LEN,
            "target_provider_id",
        )?;
        match &self.caller_origin {
            surfaces::CallerOrigin::UserSession {
                user_id,
                session_id,
            } => {
                check_string_len(user_id, MAX_SHORT_STRING_LEN, "caller_origin.user_id")?;
                check_string_len(session_id, MAX_SHORT_STRING_LEN, "caller_origin.session_id")?;
            }
            surfaces::CallerOrigin::BuiltInSystem { principal } => {
                check_string_len(principal, MAX_SHORT_STRING_LEN, "caller_origin.principal")?;
            }
            surfaces::CallerOrigin::Provider { provider_id } => {
                check_string_len(
                    provider_id,
                    MAX_SHORT_STRING_LEN,
                    "caller_origin.provider_id",
                )?;
            }
        }
        let params_len = serde_json::to_vec(&self.params)
            .map_err(|error| WireValidationError {
                field: "params",
                message: format!("failed to serialize params: {error}"),
            })?
            .len();
        if params_len > MAX_SURFACE_PARAMS_LEN {
            return Err(WireValidationError {
                field: "params",
                message: format!("params JSON is {params_len} bytes, max {MAX_SURFACE_PARAMS_LEN}"),
            });
        }
        validate_surface_json_bounds(&serde_json::Value::Object(self.params.clone()), "params")?;
        if let Some(ref encrypted) = self.encrypted_sensitive_params {
            check_string_len(
                &encrypted.key_id,
                MAX_SHORT_STRING_LEN,
                "encrypted_sensitive_params.key_id",
            )?;
            check_string_len(
                &encrypted.ciphertext_b64,
                MAX_LONG_STRING_LEN,
                "encrypted_sensitive_params.ciphertext_b64",
            )?;
        }
        Ok(())
    }
}
impl WireValidate for surfaces::SurfaceActionCancel {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(
            &self.target_provider_id,
            MAX_SHORT_STRING_LEN,
            "target_provider_id",
        )?;
        Ok(())
    }
}
impl WireValidate for surfaces::SurfaceActionResponse {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        if let Some(ref result) = self.result {
            let result_len = serde_json::to_vec(result)
                .map_err(|error| WireValidationError {
                    field: "result",
                    message: format!("failed to serialize result: {error}"),
                })?
                .len();
            if result_len > MAX_SURFACE_RESPONSE_LEN {
                return Err(WireValidationError {
                    field: "result",
                    message: format!(
                        "response result is {result_len} bytes, max {MAX_SURFACE_RESPONSE_LEN}"
                    ),
                });
            }
            validate_surface_json_bounds(result, "result")?;
        }
        if let Some(ref error) = self.error {
            error.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for surfaces::SurfaceActionError {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.message, MAX_MEDIUM_STRING_LEN, "error.message")?;
        if let Some(ref details) = self.details {
            let details_len = serde_json::to_vec(details)
                .map_err(|error| WireValidationError {
                    field: "error.details",
                    message: format!("failed to serialize details: {error}"),
                })?
                .len();
            if details_len > MAX_SURFACE_RESPONSE_LEN {
                return Err(WireValidationError {
                    field: "error.details",
                    message: format!(
                        "error details are {details_len} bytes, max {MAX_SURFACE_RESPONSE_LEN}"
                    ),
                });
            }
            validate_surface_json_bounds(details, "error.details")?;
        }
        Ok(())
    }
}
impl WireValidate for ReportPluginConfigPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.request_id, MAX_SHORT_STRING_LEN, "request_id")?;
        check_string_len(&self.plugin_type, MAX_SHORT_STRING_LEN, "plugin_type")?;
        check_string_len(&self.name, MAX_SHORT_STRING_LEN, "name")?;
        let config_str = self.config.to_string();
        check_string_len(&config_str, MAX_PLUGIN_CONFIG_JSON_LEN, "config")?;
        Ok(())
    }
}
impl WireValidate for ReportPluginConfigResponsePayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.request_id, MAX_SHORT_STRING_LEN, "request_id")?;
        check_opt_string_len(&self.error, MAX_MEDIUM_STRING_LEN, "error")?;
        Ok(())
    }
}
impl WireValidate for CertificatePayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.cert_pem, MAX_LONG_STRING_LEN, "cert_pem")?;
        Ok(())
    }
}
impl WireValidate for ErrorPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.message, MAX_MEDIUM_STRING_LEN, "message")?;
        Ok(())
    }
}
impl WireValidate for ServiceSettingsPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.ca_bundle_hash, MAX_SHORT_STRING_LEN, "ca_bundle_hash")?;
        self.report_page_limits.wire_validate()?;
        Ok(())
    }
}
impl WireValidate for ReportPageLimits {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        validate_report_page_limit(
            self.report_hosts,
            MAX_REPORT_HOSTS as u32,
            "report_page_limits.report_hosts",
        )?;
        validate_report_page_limit(
            self.version_check_results,
            MAX_VERSION_CHECK_RESULTS as u32,
            "report_page_limits.version_check_results",
        )?;
        validate_report_page_limit(
            self.discovery_results,
            MAX_DISCOVERY_PLUGIN_RESULTS as u32,
            "report_page_limits.discovery_results",
        )?;
        validate_report_page_limit(
            self.batch_update_results,
            MAX_BATCH_UPDATE_RESULTS as u32,
            "report_page_limits.batch_update_results",
        )?;
        Ok(())
    }
}
impl WireValidate for CaBundleUpdatedPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.ca_bundle_pem, MAX_LONG_STRING_LEN, "ca_bundle_pem")?;
        Ok(())
    }
}
impl WireValidate for RequestCertRenewalPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.reason, MAX_MEDIUM_STRING_LEN, "reason")?;
        Ok(())
    }
}
impl WireValidate for ServerRestartingPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.reason, MAX_MEDIUM_STRING_LEN, "reason")?;
        Ok(())
    }
}
impl WireValidate for CheckVersionsPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(
            &self.host_machine_id,
            MAX_SHORT_STRING_LEN,
            "host_machine_id",
        )?;
        check_vec_len(
            &self.assignments,
            MAX_VERSION_CHECK_ASSIGNMENTS,
            "assignments",
        )?;
        for assignment in &self.assignments {
            assignment.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for VersionCheckAssignment {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.name, MAX_SHORT_STRING_LEN, "name")?;
        if let Some(ref pa) = self.detect_version {
            pa.wire_validate()?;
        }
        if let Some(ref pa) = self.fetch_releases {
            pa.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for PluginAssignment {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(
            &self.package_identifier,
            MAX_SHORT_STRING_LEN,
            "package_identifier",
        )?;
        Ok(())
    }
}
impl WireValidate for ReleaseAsset {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.name, MAX_SHORT_STRING_LEN, "asset.name")?;
        check_string_len(
            &self.download_url,
            MAX_MEDIUM_STRING_LEN,
            "asset.download_url",
        )?;
        if let Some(ref d) = self.sha256_digest
            && (d.len() != SHA256_DIGEST_LEN || !d.chars().all(|c| c.is_ascii_hexdigit()))
        {
            return Err(WireValidationError {
                field: "asset.sha256_digest",
                message: format!("expected {SHA256_DIGEST_LEN} hex chars, got {}", d.len()),
            });
        }
        Ok(())
    }
}
impl WireValidate for ReleaseInfo {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.tag, MAX_SHORT_STRING_LEN, "release_info.tag")?;
        check_string_len(
            &self.release_url,
            MAX_MEDIUM_STRING_LEN,
            "release_info.release_url",
        )?;
        check_vec_len(&self.assets, MAX_RELEASE_ASSETS, "release_info.assets")?;
        for asset in &self.assets {
            asset.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for ExecuteUpdatePayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(
            &self.host_machine_id,
            MAX_SHORT_STRING_LEN,
            "host_machine_id",
        )?;
        check_string_len(
            &self.software_item_name,
            MAX_SHORT_STRING_LEN,
            "software_item_name",
        )?;
        check_string_len(&self.to_version, MAX_SHORT_STRING_LEN, "to_version")?;
        check_vec_len(
            &self.pre_update_hook_plugins,
            MAX_UPDATE_HOOKS,
            "pre_update_hook_plugins",
        )?;
        check_vec_len(
            &self.post_update_hook_plugins,
            MAX_UPDATE_HOOKS,
            "post_update_hook_plugins",
        )?;
        self.execute_update_plugin.wire_validate()?;
        if let Some(ref detect) = self.detect_version_plugin {
            detect.wire_validate()?;
        }
        if let Some(ref ri) = self.release_info {
            ri.wire_validate()?;
        }
        for plugin in &self.pre_update_hook_plugins {
            plugin.wire_validate()?;
        }
        for plugin in &self.post_update_hook_plugins {
            plugin.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for ExecuteBatchUpdatePayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(
            &self.host_machine_id,
            MAX_SHORT_STRING_LEN,
            "host_machine_id",
        )?;
        check_vec_len(&self.updates, MAX_BATCH_UPDATES, "updates")?;
        check_vec_len(
            &self.pre_update_hook_plugins,
            MAX_UPDATE_HOOKS,
            "pre_update_hook_plugins",
        )?;
        check_vec_len(
            &self.post_update_hook_plugins,
            MAX_UPDATE_HOOKS,
            "post_update_hook_plugins",
        )?;
        for update in &self.updates {
            update.wire_validate()?;
        }
        for plugin in &self.pre_update_hook_plugins {
            plugin.wire_validate()?;
        }
        for plugin in &self.post_update_hook_plugins {
            plugin.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for BatchUpdateItem {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(
            &self.package_identifier,
            MAX_SHORT_STRING_LEN,
            "package_identifier",
        )?;
        check_string_len(&self.to_version, MAX_SHORT_STRING_LEN, "to_version")?;
        Ok(())
    }
}
impl WireValidate for DiscoverSoftwarePayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(
            &self.host_machine_id,
            MAX_SHORT_STRING_LEN,
            "host_machine_id",
        )?;
        check_vec_len(&self.plugins, MAX_DISCOVERY_PLUGINS, "plugins")?;
        Ok(())
    }
}
impl WireValidate for SetUpdateFreezePayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_opt_string_len(&self.reason, MAX_MEDIUM_STRING_LEN, "reason")?;
        Ok(())
    }
}
impl WireValidate for UpdateStdinDataPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.data, MAX_STDIN_DATA_LEN, "data")?;
        Ok(())
    }
}
impl WireValidate for StdinAttentionPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_opt_string_len(&self.hint, MAX_MEDIUM_STRING_LEN, "hint")?;
        Ok(())
    }
}
impl WireValidate for SoftwareStatesPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        if self.page.total_pages < 1 {
            return Err(WireValidationError {
                field: "page.total_pages",
                message: "total_pages must be at least 1".to_string(),
            });
        }
        if self.page.page_index >= self.page.total_pages {
            return Err(WireValidationError {
                field: "page.page_index",
                message: format!(
                    "page_index {} must be less than total_pages {}",
                    self.page.page_index, self.page.total_pages
                ),
            });
        }
        check_vec_len(&self.items, MAX_SOFTWARE_STATE_ITEMS, "items")?;
        check_vec_len(
            &self.host_summaries,
            MAX_HOST_PACKAGE_HOST_STATES,
            "host_summaries",
        )?;
        check_vec_len(&self.hosts, MAX_MQTT_HOSTS, "hosts")?;
        for item in &self.items {
            item.wire_validate()?;
        }
        for host_state in &self.host_summaries {
            host_state.wire_validate()?;
        }
        for host in &self.hosts {
            host.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for SoftwareStateItem {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.name, MAX_SHORT_STRING_LEN, "name")?;
        check_opt_string_len(&self.icon_url, MAX_ICON_URL_LEN, "icon_url")?;
        check_vec_len(&self.hosts, MAX_SOFTWARE_STATE_HOSTS, "hosts")?;
        for host in &self.hosts {
            host.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for SoftwareStateHostEntry {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.hostname, MAX_SHORT_STRING_LEN, "hostname")?;
        check_string_len(&self.friendly_name, MAX_SHORT_STRING_LEN, "friendly_name")?;
        check_opt_string_len(
            &self.installed_version,
            MAX_SHORT_STRING_LEN,
            "installed_version",
        )?;
        check_opt_string_len(&self.latest_version, MAX_SHORT_STRING_LEN, "latest_version")?;
        check_opt_string_len(&self.release_url, MAX_MEDIUM_STRING_LEN, "release_url")?;
        check_opt_string_len(&self.release_notes, MAX_LONG_STRING_LEN, "release_notes")?;
        check_opt_string_len(
            &self.update_category,
            MAX_SHORT_STRING_LEN,
            "update_category",
        )?;
        check_opt_string_len(&self.release_date, MAX_SHORT_STRING_LEN, "release_date")?;
        check_opt_string_len(
            &self.last_checked_at,
            MAX_SHORT_STRING_LEN,
            "last_checked_at",
        )?;
        Ok(())
    }
}
impl WireValidate for HostPackageSummary {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.hostname, MAX_SHORT_STRING_LEN, "hostname")?;
        check_string_len(&self.friendly_name, MAX_SHORT_STRING_LEN, "friendly_name")?;
        Ok(())
    }
}
impl WireValidate for HostStateMetadata {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.hostname, MAX_SHORT_STRING_LEN, "hostname")?;
        check_string_len(&self.friendly_name, MAX_SHORT_STRING_LEN, "friendly_name")?;
        check_opt_string_len(&self.os_type, MAX_SHORT_STRING_LEN, "os_type")?;
        check_opt_string_len(&self.os_version, MAX_SHORT_STRING_LEN, "os_version")?;
        check_opt_string_len(&self.architecture, MAX_SHORT_STRING_LEN, "architecture")?;
        check_vec_len(&self.tags, MAX_HOST_TAGS, "tags")?;
        for tag in &self.tags {
            check_string_len(tag, MAX_SHORT_STRING_LEN, "tags[]")?;
        }
        check_opt_string_len(&self.agent_version, MAX_SHORT_STRING_LEN, "agent_version")?;
        check_opt_string_len(
            &self.agent_last_seen_at,
            MAX_SHORT_STRING_LEN,
            "agent_last_seen_at",
        )?;
        Ok(())
    }
}
impl WireValidate for HostConnectivityUpdatedPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_vec_len(&self.updates, MAX_CONNECTIVITY_UPDATES, "updates")?;
        for update in &self.updates {
            check_opt_string_len(&update.last_seen_at, MAX_SHORT_STRING_LEN, "last_seen_at")?;
            check_opt_string_len(&update.agent_version, MAX_SHORT_STRING_LEN, "agent_version")?;
        }
        Ok(())
    }
}
impl WireValidate for RequestCaRotationPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.reason, MAX_MEDIUM_STRING_LEN, "reason")?;
        Ok(())
    }
}
impl WireValidate for StoreServiceConfigPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.request_id, MAX_SHORT_STRING_LEN, "request_id")?;
        check_string_len(&self.key, MAX_SHORT_STRING_LEN, "key")?;
        let value_str = self.value.to_string();
        check_string_len(&value_str, MAX_SERVICE_CONFIG_VALUE_LEN, "value")?;
        Ok(())
    }
}
impl WireValidate for DeleteServiceConfigPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.request_id, MAX_SHORT_STRING_LEN, "request_id")?;
        check_string_len(&self.key, MAX_SHORT_STRING_LEN, "key")?;
        Ok(())
    }
}
impl WireValidate for ServiceConfigAckPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.request_id, MAX_SHORT_STRING_LEN, "request_id")?;
        check_opt_string_len(&self.error, MAX_MEDIUM_STRING_LEN, "error")?;
        Ok(())
    }
}
impl WireValidate for ServiceConfigEntry {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.key, MAX_SHORT_STRING_LEN, "key")?;
        let value_str = self.value.to_string();
        check_string_len(&value_str, MAX_SERVICE_CONFIG_VALUE_LEN, "value")?;
        Ok(())
    }
}
impl WireValidate for ServiceConfigKey {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.key, MAX_SHORT_STRING_LEN, "key")?;
        Ok(())
    }
}
impl WireValidate for ServiceConfigDeliveryPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_vec_len(&self.entries, MAX_SERVICE_CONFIG_ENTRIES, "entries")?;
        for (i, entry) in self.entries.iter().enumerate() {
            entry.wire_validate().map_err(|mut e| {
                e.field = "entries[i]";
                e
            })?;
            let _ = i;
        }
        Ok(())
    }
}
impl WireValidate for ServiceConfigUpdatedPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_vec_len(&self.changed, MAX_SERVICE_CONFIG_ENTRIES, "changed")?;
        check_vec_len(&self.deleted, MAX_SERVICE_CONFIG_ENTRIES, "deleted")?;
        for entry in &self.changed {
            entry.wire_validate()?;
        }
        for key in &self.deleted {
            key.wire_validate()?;
        }
        Ok(())
    }
}
impl WireValidate for WorkloadClaimPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_map_len(&self.claims, MAX_WORKLOAD_CLAIM_KEYS, "claims")?;
        for key in self.claims.keys() {
            check_string_len(key, MAX_SHORT_STRING_LEN, "claims[key]")?;
        }
        Ok(())
    }
}
impl WireValidate for WorkloadClaimResultPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_set_len(&self.granted, MAX_WORKLOAD_CLAIM_KEYS, "granted")?;
        check_set_len(&self.rejected, MAX_WORKLOAD_CLAIM_KEYS, "rejected")?;
        for key in &self.granted {
            check_string_len(key, MAX_SHORT_STRING_LEN, "granted[key]")?;
        }
        for key in &self.rejected {
            check_string_len(key, MAX_SHORT_STRING_LEN, "rejected[key]")?;
        }
        Ok(())
    }
}
impl WireValidate for WorkloadReleasePayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_set_len(&self.keys, MAX_WORKLOAD_CLAIM_KEYS, "keys")?;
        for key in &self.keys {
            check_string_len(key, MAX_SHORT_STRING_LEN, "keys[key]")?;
        }
        Ok(())
    }
}
impl WireValidate for WorkloadClaimAnnouncementPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_map_len(&self.claimed, MAX_WORKLOAD_CLAIM_KEYS, "claimed")?;
        check_set_len(&self.released, MAX_WORKLOAD_CLAIM_KEYS, "released")?;
        check_string_len(&self.claimed_at, MAX_SHORT_STRING_LEN, "claimed_at")?;
        for key in self.claimed.keys() {
            check_string_len(key, MAX_SHORT_STRING_LEN, "claimed[key]")?;
        }
        for key in &self.released {
            check_string_len(key, MAX_SHORT_STRING_LEN, "released[key]")?;
        }
        Ok(())
    }
}
impl WireValidate for WorkloadClaimSyncResponsePayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_map_len(&self.claims, MAX_WORKLOAD_CLAIM_KEYS, "claims")?;
        for (key, entry) in &self.claims {
            check_string_len(key, MAX_SHORT_STRING_LEN, "claims[key]")?;
            check_string_len(
                &entry.claimed_at,
                MAX_SHORT_STRING_LEN,
                "claims[].claimed_at",
            )?;
        }
        Ok(())
    }
}
impl WireValidate for TestPluginConfigPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.request_id, MAX_SHORT_STRING_LEN, "request_id")?;
        check_string_len(
            &self.host_machine_id,
            MAX_SHORT_STRING_LEN,
            "host_machine_id",
        )?;
        check_string_len(&self.plugin_type, MAX_SHORT_STRING_LEN, "plugin_type")?;
        check_opt_string_len(
            &self.package_identifier,
            MAX_SHORT_STRING_LEN,
            "package_identifier",
        )?;
        let config_str = self.config.to_string();
        check_string_len(&config_str, MAX_PLUGIN_CONFIG_JSON_LEN, "config")?;
        Ok(())
    }
}
impl WireValidate for TestPluginConfigResultPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.request_id, MAX_SHORT_STRING_LEN, "request_id")?;
        check_opt_string_len(&self.output, MAX_CONFIG_TEST_OUTPUT_LEN, "output")?;
        check_opt_string_len(&self.error, MAX_MEDIUM_STRING_LEN, "error")?;
        check_opt_string_len(
            &self.detected_version,
            MAX_SHORT_STRING_LEN,
            "detected_version",
        )?;
        Ok(())
    }
}
