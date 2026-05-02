//! `WireValidate` implementations for all wire protocol payload structs.
//!
//! Separated from `lib.rs` for readability. Each impl validates the struct's
//! own fields and delegates to nested structs that also implement `WireValidate`.

use crate::limits::*;
use crate::*;

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

// ── ServiceMessage dispatcher ─────────────────────────────────────────────────

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
            // Forward-compatible: unknown variants from newer peers pass validation.
            _ => {
                tracing::debug!(
                    "received unknown ServiceMessage variant from peer; skipping validation"
                );
                Ok(())
            }
        }
    }
}

// ── ControllerMessage dispatcher ──────────────────────────────────────────────

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
            // Forward-compatible: unknown variants from newer peers pass validation.
            _ => {
                tracing::debug!(
                    "received unknown ControllerMessage variant from peer; skipping validation"
                );
                Ok(())
            }
        }
    }
}

// ── ReportPagination ─────────────────────────────────────────────────────────

impl WireValidate for crate::envelope::ReportPagination {
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

// ── ServiceMessage payload impls ──────────────────────────────────────────────

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

impl WireValidate for uptrakit_shared_types::DiscoveredSoftware {
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
            if let Some(nav_icon) = &surface.descriptor.nav_icon {
                if nav_icon.is_empty() {
                    return Err(WireValidationError {
                        field: "surfaces[].descriptor.nav_icon",
                        message: "must not be empty".to_string(),
                    });
                }
                check_string_len(nav_icon, MAX_NAV_ICON_LEN, "surfaces[].descriptor.nav_icon")?;
            }
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

// ── ControllerMessage payload impls ───────────────────────────────────────────

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

// ── Service config store ──────────────────────────────────────────────────────

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
            let _ = i; // avoid warning
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

// ── Workload claim protocol ─────────────────────────────────────────────────

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

// ── Config test payload impls ────────────────────────────────────────────────

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

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — assert!(result.is_ok()) are idiomatic in tests"
    )]

    use super::*;

    #[test]
    fn service_message_report_hosts_validates() {
        let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
            hosts: vec![HostInfo {
                machine_id: "test-id".to_string(),
                os_type: Some("linux".to_string()),
                os_version: None,
                architecture: None,
                hostname: None,
                ip_address: None,
                agent_host_id: None,
                features: None,
            }],
            agent_version: "1.0.0".to_string(),
            capabilities: std::collections::BTreeSet::new(),
        });
        assert!(msg.wire_validate().is_ok());
    }

    #[test]
    fn service_message_report_hosts_too_many() {
        let hosts: Vec<HostInfo> = (0..MAX_REPORT_HOSTS + 1)
            .map(|i| HostInfo {
                machine_id: format!("host-{i}"),
                os_type: None,
                os_version: None,
                architecture: None,
                hostname: None,
                ip_address: None,
                agent_host_id: None,
                features: None,
            })
            .collect();
        let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
            hosts,
            agent_version: "1.0.0".to_string(),
            capabilities: std::collections::BTreeSet::new(),
        });
        let err = msg.wire_validate().unwrap_err();
        assert_eq!(err.field, "hosts");
    }

    #[test]
    fn controller_message_check_versions_validates() {
        let msg = ControllerMessage::CheckVersions(CheckVersionsPayload {
            host_machine_id: "test".to_string(),
            assignments: vec![],
        });
        assert!(msg.wire_validate().is_ok());
    }

    #[test]
    fn controller_message_check_versions_too_many() {
        let assignments: Vec<VersionCheckAssignment> = (0..MAX_VERSION_CHECK_ASSIGNMENTS + 1)
            .map(|i| VersionCheckAssignment {
                software_item_id: uuid::Uuid::nil(),
                name: format!("item-{i}"),
                detect_version: None,
                fetch_releases: None,
                host_software_item_id: None,
            })
            .collect();
        let msg = ControllerMessage::CheckVersions(CheckVersionsPayload {
            host_machine_id: "test".to_string(),
            assignments,
        });
        let err = msg.wire_validate().unwrap_err();
        assert_eq!(err.field, "assignments");
    }

    #[test]
    fn set_update_freeze_validates() {
        let payload = SetUpdateFreezePayload {
            enabled: true,
            reason: Some("test".to_string()),
        };
        assert!(payload.wire_validate().is_ok());
    }

    #[test]
    fn set_update_freeze_reason_too_long() {
        let payload = SetUpdateFreezePayload {
            enabled: true,
            reason: Some("x".repeat(MAX_MEDIUM_STRING_LEN + 1)),
        };
        assert!(payload.wire_validate().is_err());
    }

    #[test]
    fn release_asset_validates() {
        let asset = ReleaseAsset {
            name: "app.tar.gz".to_string(),
            download_url: "https://example.com/app".to_string(),
            size: None,
            content_type: None,
            sha256_digest: Some("a".repeat(64)),
        };
        assert!(asset.wire_validate().is_ok());
    }

    #[test]
    fn release_asset_invalid_digest_wrong_length() {
        let asset = ReleaseAsset {
            name: "app.tar.gz".to_string(),
            download_url: "https://example.com/app".to_string(),
            size: None,
            content_type: None,
            sha256_digest: Some("abc".to_string()),
        };
        let err = asset.wire_validate().unwrap_err();
        assert_eq!(err.field, "asset.sha256_digest");
    }

    #[test]
    fn release_asset_invalid_digest_non_hex() {
        let asset = ReleaseAsset {
            name: "app.tar.gz".to_string(),
            download_url: "https://example.com/app".to_string(),
            size: None,
            content_type: None,
            sha256_digest: Some("z".repeat(64)),
        };
        let err = asset.wire_validate().unwrap_err();
        assert_eq!(err.field, "asset.sha256_digest");
    }

    #[test]
    fn release_info_validates() {
        let info = ReleaseInfo {
            tag: "v1.0.0".to_string(),
            release_url: "https://example.com/release".to_string(),
            assets: vec![],
            attestation_status: None,
            require_attestation: false,
        };
        assert!(info.wire_validate().is_ok());
    }

    #[test]
    fn release_info_too_many_assets() {
        let assets: Vec<ReleaseAsset> = (0..MAX_RELEASE_ASSETS + 1)
            .map(|i| ReleaseAsset {
                name: format!("asset-{i}"),
                download_url: format!("https://example.com/{i}"),
                size: None,
                content_type: None,
                sha256_digest: None,
            })
            .collect();
        let info = ReleaseInfo {
            tag: "v1.0.0".to_string(),
            release_url: "https://example.com".to_string(),
            assets,
            attestation_status: None,
            require_attestation: false,
        };
        let err = info.wire_validate().unwrap_err();
        assert_eq!(err.field, "release_info.assets");
    }

    #[test]
    fn execute_update_validates() {
        let payload = ExecuteUpdatePayload {
            host_machine_id: "test".to_string(),
            update_history_id: uuid::Uuid::nil(),
            software_item_id: uuid::Uuid::nil(),
            software_item_name: "test".to_string(),
            to_version: "1.0".to_string(),
            detect_version_plugin: None,
            execute_update_plugin: PluginAssignment {
                plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
                package_identifier: "test".to_string(),
                config: serde_json::json!({}),
            },
            pre_update_hook_plugins: vec![],
            post_update_hook_plugins: vec![],
            release_info: Some(ReleaseInfo {
                tag: "v1.0".to_string(),
                release_url: "https://example.com".to_string(),
                assets: vec![],
                attestation_status: None,
                require_attestation: false,
            }),
            timeout: std::time::Duration::from_secs(60),
            interactive: false,
        };
        assert!(payload.wire_validate().is_ok());
    }

    #[test]
    fn execute_update_too_many_hook_plugins() {
        let plugins: Vec<PluginAssignment> = (0..MAX_UPDATE_HOOKS + 1)
            .map(|_| PluginAssignment {
                plugin_type: plugin_ids::HOOK_SHELL.clone(),
                package_identifier: String::new(),
                config: serde_json::json!({}),
            })
            .collect();
        let payload = ExecuteUpdatePayload {
            host_machine_id: "test".to_string(),
            update_history_id: uuid::Uuid::nil(),
            software_item_id: uuid::Uuid::nil(),
            software_item_name: "test".to_string(),
            to_version: "1.0".to_string(),
            detect_version_plugin: None,
            execute_update_plugin: PluginAssignment {
                plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
                package_identifier: "test".to_string(),
                config: serde_json::json!({}),
            },
            pre_update_hook_plugins: plugins,
            post_update_hook_plugins: vec![],
            release_info: None,
            timeout: std::time::Duration::from_secs(60),
            interactive: false,
        };
        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "pre_update_hook_plugins");
    }

    #[test]
    fn discovery_results_validates() {
        let payload = DiscoveryResultsPayload {
            host_machine_id: "test".to_string(),
            results: vec![],
        };
        assert!(payload.wire_validate().is_ok());
    }

    #[test]
    fn unknown_service_message_passes() {
        let msg = ServiceMessage::Unknown;
        assert!(msg.wire_validate().is_ok());
    }

    #[test]
    fn unknown_controller_message_passes() {
        let msg = ControllerMessage::Unknown;
        assert!(msg.wire_validate().is_ok());
    }

    #[test]
    fn batch_update_result_validates() {
        let payload = BatchUpdateResultPayload {
            batch_id: uuid::Uuid::nil(),
            results: vec![],
        };
        assert!(payload.wire_validate().is_ok());
    }

    #[test]
    fn batch_update_result_too_many() {
        let results: Vec<BatchUpdateItemResult> = (0..MAX_BATCH_UPDATE_RESULTS + 1)
            .map(|_| BatchUpdateItemResult {
                host_software_item_id: uuid::Uuid::nil(),
                update_history_id: uuid::Uuid::nil(),
                status: UpdateFinalStatus::Completed,
                output: String::new(),
                installed_version: None,
                error: None,
            })
            .collect();
        let payload = BatchUpdateResultPayload {
            batch_id: uuid::Uuid::nil(),
            results,
        };
        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "results");
    }

    // ── Extension wire validation tests ─────────────────────────────────────

    fn test_surface_registration() -> surfaces::SurfaceRegistration {
        surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: "uptrakit-agent-ssh".to_string(),
                provider_kind: surfaces::ProviderKind::Service,
                provider_namespace: "uptrakit.agent.ssh".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::default(),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Tenant,
                tenant_id: Some(uuid::Uuid::nil().to_string()),
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("ssh.guest.panel").unwrap())
                    .label("SSH Guests")
                    .priority(100)
                    .slot(surfaces::SLOT_SETTINGS_TABS)
                    .scope(surfaces::Scope::Tenant)
                    .targeting(surfaces::Targeting::Universal)
                    .provider_kind(surfaces::ProviderKind::Service)
                    .required_capabilities(surfaces::CapabilitySet::default())
                    .root_node(surfaces::SurfaceNode::Section {
                        title: Some("Guests".to_string()),
                        children: vec![surfaces::SurfaceNode::TextBlock {
                            text: "Guests view".to_string(),
                        }],
                    })
                    .build(),
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: "Refresh".to_string(),
                    required_permission: None,
                    input_schema: None,
                    result_schema: None,
                    sensitive_fields: vec![],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ProviderProxied,
                    workflow_steps: vec![],
                    form_ui: None,
                }],
                data_sources: vec![surfaces::DataSourceDescriptor {
                    data_source_id: surfaces::DataSourceId::new("guest.rows").unwrap(),
                    kind: surfaces::DataSourceKind::Static {
                        data: serde_json::json!({"rows": []}),
                    },
                    result_schema: surfaces::SchemaContract::Object,
                    pagination: None,
                    sorting: None,
                    filtering: None,
                    refresh_policy: surfaces::RefreshPolicy::Manual,
                    empty_state: None,
                }],
            }],
            encryption_metadata: None,
        }
    }

    fn nested_json_array(depth: usize) -> serde_json::Value {
        let mut value = serde_json::json!(0);
        for _ in 0..depth {
            value = serde_json::json!([value]);
        }
        value
    }

    #[test]
    fn surface_registration_rejects_oversized_nested_root_node_text() {
        let mut payload = test_surface_registration();
        payload.surfaces[0].descriptor.root_node = surfaces::SurfaceNode::Section {
            title: None,
            children: vec![surfaces::SurfaceNode::Tabs {
                tabs: vec![surfaces::SurfaceTab {
                    id: surfaces::SurfaceTabId::new("guests").unwrap(),
                    label: "Guests".to_string(),
                    root: surfaces::SurfaceNode::TextBlock {
                        text: "x".repeat(MAX_MEDIUM_STRING_LEN + 1),
                    },
                }],
            }],
        };

        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "surfaces[].descriptor.root_node.text");
    }

    #[test]
    fn surface_registration_rejects_invalid_interaction_confirmation_text() {
        let mut payload = test_surface_registration();
        payload.surfaces[0].interactions[0] = surfaces::InteractionDescriptor {
            interaction_id: surfaces::InteractionId::new("danger.refresh").unwrap(),
            kind: surfaces::InteractionKind::ConfirmableAction,
            label: "Danger Refresh".to_string(),
            required_permission: None,
            input_schema: None,
            result_schema: None,
            sensitive_fields: vec![],
            timeout_seconds: None,
            confirmation: Some(surfaces::InteractionConfirmation {
                title: "Confirm".to_string(),
                message: "x".repeat(MAX_MEDIUM_STRING_LEN + 1),
                confirm_label: None,
                cancel_label: None,
                severity: surfaces::ConfirmationSeverity::Warning,
            }),
            transport: surfaces::InteractionTransport::ProviderProxied,
            workflow_steps: vec![],
            form_ui: None,
        };

        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "surfaces[].interactions[].confirmation.message");
    }

    #[test]
    fn surface_registration_rejects_empty_nav_icon() {
        let mut payload = test_surface_registration();
        payload.surfaces[0].descriptor.nav_icon = Some(String::new());
        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "surfaces[].descriptor.nav_icon");
    }

    #[test]
    fn surface_registration_rejects_oversized_nav_icon() {
        let mut payload = test_surface_registration();
        payload.surfaces[0].descriptor.nav_icon = Some("x".repeat(65));
        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "surfaces[].descriptor.nav_icon");
    }

    #[test]
    fn surface_registration_accepts_valid_nav_icon() {
        let mut payload = test_surface_registration();
        payload.surfaces[0].descriptor.nav_icon = Some("Package".to_string());
        assert!(payload.wire_validate().is_ok());
    }

    #[test]
    fn surface_registration_rejects_invalid_data_source_metadata() {
        let mut payload = test_surface_registration();
        payload.surfaces[0].data_sources[0] = surfaces::DataSourceDescriptor {
            data_source_id: surfaces::DataSourceId::new("guest.query").unwrap(),
            kind: surfaces::DataSourceKind::ProviderQuery {
                operation_id: "x".repeat(MAX_SHORT_STRING_LEN + 1),
            },
            result_schema: surfaces::SchemaContract::Object,
            pagination: Some(surfaces::DataSourcePagination {
                default_page_size: 100,
                max_page_size: 10,
            }),
            sorting: None,
            filtering: None,
            refresh_policy: surfaces::RefreshPolicy::Interval { seconds: 0 },
            empty_state: None,
        };

        let err = payload.wire_validate().unwrap_err();
        assert_eq!(
            err.field,
            "surfaces[].data_sources[].kind.provider_query.operation_id"
        );
    }

    #[test]
    fn surface_registration_rejects_overdeep_static_data() {
        let mut payload = test_surface_registration();
        payload.surfaces[0].data_sources[0] = surfaces::DataSourceDescriptor {
            data_source_id: surfaces::DataSourceId::new("guest.deep").unwrap(),
            kind: surfaces::DataSourceKind::Static {
                data: nested_json_array(MAX_SURFACE_JSON_DEPTH + 1),
            },
            result_schema: surfaces::SchemaContract::Array,
            pagination: None,
            sorting: None,
            filtering: None,
            refresh_policy: surfaces::RefreshPolicy::Manual,
            empty_state: None,
        };

        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "surfaces[].data_sources[].kind.static.data");
    }

    #[test]
    fn surface_action_request_rejects_invalid_tenant_uuid() {
        let payload = surfaces::SurfaceActionRequest {
            request_id: uuid::Uuid::new_v4(),
            tenant_id: "not-a-uuid".to_string(),
            surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
            interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
            idempotency_key: "idem-1".to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "uptrakit-agent-ssh".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: None,
        };

        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "tenant_id");
    }

    #[test]
    fn surface_action_request_rejects_overdeep_params_json() {
        let payload = surfaces::SurfaceActionRequest {
            request_id: uuid::Uuid::new_v4(),
            tenant_id: uuid::Uuid::nil().to_string(),
            surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
            interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
            idempotency_key: "idem-1".to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "uptrakit-agent-ssh".to_string(),
            },
            params: serde_json::json!({
                "payload": nested_json_array(MAX_SURFACE_JSON_DEPTH + 1)
            })
            .as_object()
            .unwrap()
            .clone(),
            encrypted_sensitive_params: None,
        };

        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "params");
    }

    #[test]
    fn surface_action_request_rejects_over_node_count_params_json() {
        let payload = surfaces::SurfaceActionRequest {
            request_id: uuid::Uuid::new_v4(),
            tenant_id: uuid::Uuid::nil().to_string(),
            surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
            interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
            idempotency_key: "idem-1".to_string(),
            target_provider_id: None,
            caller_origin: surfaces::CallerOrigin::Provider {
                provider_id: "uptrakit-agent-ssh".to_string(),
            },
            params: serde_json::json!({
                "payload": vec![0u8; MAX_SURFACE_JSON_NODES + 1]
            })
            .as_object()
            .unwrap()
            .clone(),
            encrypted_sensitive_params: None,
        };

        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "params");
    }

    #[test]
    fn surface_action_response_rejects_overdeep_result_json() {
        let payload = surfaces::SurfaceActionResponse {
            request_id: uuid::Uuid::new_v4(),
            success: true,
            result: Some(serde_json::json!({
                "payload": nested_json_array(MAX_SURFACE_JSON_DEPTH + 1)
            })),
            error: None,
        };

        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "result");
    }

    #[test]
    fn surface_action_response_rejects_over_node_count_result_json() {
        let payload = surfaces::SurfaceActionResponse {
            request_id: uuid::Uuid::new_v4(),
            success: true,
            result: Some(serde_json::json!({
                "payload": vec![0u8; MAX_SURFACE_JSON_NODES + 1]
            })),
            error: None,
        };

        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "result");
    }

    #[test]
    fn surface_action_error_rejects_overdeep_details_json() {
        let payload = surfaces::SurfaceActionError {
            code: surfaces::SurfaceActionErrorCode::InternalError,
            message: "bad".to_string(),
            details: Some(serde_json::json!({
                "payload": nested_json_array(MAX_SURFACE_JSON_DEPTH + 1)
            })),
        };

        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "error.details");
    }

    #[test]
    fn surface_action_error_rejects_over_node_count_details_json() {
        let payload = surfaces::SurfaceActionError {
            code: surfaces::SurfaceActionErrorCode::InternalError,
            message: "bad".to_string(),
            details: Some(serde_json::json!({
                "payload": vec![0u8; MAX_SURFACE_JSON_NODES + 1]
            })),
        };

        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "error.details");
    }

    #[test]
    fn report_plugin_config_validates() {
        let msg = ServiceMessage::ReportPluginConfig(ReportPluginConfigPayload {
            request_id: "req-1".to_string(),
            plugin_type: "infrastructure_proxmox".to_string(),
            name: "pve.local".to_string(),
            config: serde_json::json!({"api_url": "https://pve:8006"}),
        });
        assert!(msg.wire_validate().is_ok());
    }

    #[test]
    fn report_plugin_config_response_validates() {
        let msg =
            ControllerMessage::ReportPluginConfigResponse(ReportPluginConfigResponsePayload {
                request_id: "req-1".to_string(),
                success: true,
                plugin_config_id: Some(uuid::Uuid::nil()),
                error: None,
            });
        assert!(msg.wire_validate().is_ok());
    }

    #[test]
    fn report_plugin_config_rejects_oversized_config() {
        let msg = ServiceMessage::ReportPluginConfig(ReportPluginConfigPayload {
            request_id: "req-1".to_string(),
            plugin_type: "infrastructure_proxmox".to_string(),
            name: "pve.local".to_string(),
            config: serde_json::Value::String("x".repeat(MAX_PLUGIN_CONFIG_JSON_LEN + 1)),
        });
        assert!(msg.wire_validate().is_err());
    }

    #[test]
    fn update_stdin_data_validates() {
        let msg = ControllerMessage::UpdateStdinData(UpdateStdinDataPayload {
            update_history_id: uuid::Uuid::nil(),
            data: "aGVsbG8=".to_string(),
            signal: None,
        });
        assert!(msg.wire_validate().is_ok());
    }

    #[test]
    fn update_stdin_data_rejects_oversized_data() {
        let msg = ControllerMessage::UpdateStdinData(UpdateStdinDataPayload {
            update_history_id: uuid::Uuid::nil(),
            data: "x".repeat(MAX_STDIN_DATA_LEN + 1),
            signal: None,
        });
        assert!(msg.wire_validate().is_err());
    }

    #[test]
    fn stdin_attention_validates() {
        let msg = ServiceMessage::StdinAttention(StdinAttentionPayload {
            update_history_id: uuid::Uuid::nil(),
            hint: Some("waiting for config file conflict resolution".to_string()),
        });
        assert!(msg.wire_validate().is_ok());
    }

    #[test]
    fn stdin_attention_rejects_oversized_hint() {
        let msg = ServiceMessage::StdinAttention(StdinAttentionPayload {
            update_history_id: uuid::Uuid::nil(),
            hint: Some("x".repeat(MAX_MEDIUM_STRING_LEN + 1)),
        });
        assert!(msg.wire_validate().is_err());
    }

    #[test]
    fn service_settings_report_page_limits_validate() {
        let msg = ControllerMessage::ServiceSettings(ServiceSettingsPayload {
            renewal_window_hours: 6,
            ca_bundle_hash: "hash".to_string(),
            capabilities: std::collections::BTreeSet::new(),
            report_page_limits: ReportPageLimits::default(),
            shutdown_timeout: None,
            ping_interval: std::time::Duration::from_secs(30),
            tenant_id: None,
        });

        assert!(msg.wire_validate().is_ok());
    }

    #[test]
    fn service_settings_reject_zero_report_page_limit() {
        let msg = ControllerMessage::ServiceSettings(ServiceSettingsPayload {
            renewal_window_hours: 6,
            ca_bundle_hash: "hash".to_string(),
            capabilities: std::collections::BTreeSet::new(),
            report_page_limits: ReportPageLimits {
                report_hosts: 0,
                ..ReportPageLimits::default()
            },
            shutdown_timeout: None,
            ping_interval: std::time::Duration::from_secs(30),
            tenant_id: None,
        });

        let err = msg.wire_validate().unwrap_err();
        assert_eq!(err.field, "report_page_limits.report_hosts");
    }
}
