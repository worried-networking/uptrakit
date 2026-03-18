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
            ServiceMessage::ExtensionRegister(p) => p.wire_validate(),
            ServiceMessage::ExtensionActionsRegister(p) => p.wire_validate(),
            ServiceMessage::ExtensionResponse(p) => p.wire_validate(),
            ServiceMessage::ExtensionRequest(p) => p.wire_validate(),
            ServiceMessage::StoreServiceConfig(p) => p.wire_validate(),
            ServiceMessage::DeleteServiceConfig(p) => p.wire_validate(),
            ServiceMessage::WorkloadClaim(p) => p.wire_validate(),
            ServiceMessage::WorkloadRelease(p) => p.wire_validate(),
            ServiceMessage::TestPluginConfigResult(p) => p.wire_validate(),
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
            ControllerMessage::ExtensionRequest(p) => p.wire_validate(),
            ControllerMessage::ExtensionResponse(p) => p.wire_validate(),
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

// ── Extension payload impls ──────────────────────────────────────────────────

impl WireValidate for extension::ExtensionRegisterPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_vec_len(&self.manifests, MAX_EXTENSION_MANIFESTS, "manifests")?;
        for manifest in &self.manifests {
            manifest.wire_validate()?;
        }
        check_opt_string_len(
            &self.encryption_public_key,
            MAX_SHORT_STRING_LEN,
            "encryption_public_key",
        )?;
        Ok(())
    }
}

impl WireValidate for extension::ExtensionActionsPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_vec_len(&self.actions, MAX_EXTENSION_ACTIONS, "actions")?;
        for action in &self.actions {
            action.wire_validate()?;
        }
        Ok(())
    }
}

impl WireValidate for extension::ExtensionManifest {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.id, MAX_SHORT_STRING_LEN, "extension.id")?;
        check_string_len(&self.label, MAX_SHORT_STRING_LEN, "extension.label")?;
        check_string_len(
            &self.required_permission,
            MAX_SHORT_STRING_LEN,
            "extension.required_permission",
        )?;
        self.placement.wire_validate()?;
        self.ui.wire_validate()?;
        Ok(())
    }
}

impl WireValidate for extension::ExtensionPlacement {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        match self {
            extension::ExtensionPlacement::Page { nav_section, icon } => {
                check_string_len(nav_section, MAX_SHORT_STRING_LEN, "placement.nav_section")?;
                check_opt_string_len(icon, MAX_SHORT_STRING_LEN, "placement.icon")?;
            }
            extension::ExtensionPlacement::Panel {
                target_page,
                position: _,
                tab_group: _,
            } => {
                check_string_len(target_page, MAX_SHORT_STRING_LEN, "placement.target_page")?;
            }
            extension::ExtensionPlacement::ContextMenuGroup {
                target_entity,
                group_label,
            } => {
                check_string_len(
                    target_entity,
                    MAX_SHORT_STRING_LEN,
                    "placement.target_entity",
                )?;
                check_string_len(group_label, MAX_SHORT_STRING_LEN, "placement.group_label")?;
            }
            extension::ExtensionPlacement::TableColumns {
                target_table,
                columns,
            } => {
                check_string_len(target_table, MAX_SHORT_STRING_LEN, "placement.target_table")?;
                check_vec_len(columns, MAX_EXTENSION_COLUMNS, "placement.columns")?;
                for col in columns {
                    col.wire_validate()?;
                }
            }
            _ => {
                tracing::warn!("unknown ExtensionPlacement variant, skipping validation");
            }
        }
        Ok(())
    }
}

impl WireValidate for extension::ExtensionColumn {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.key, MAX_SHORT_STRING_LEN, "column.key")?;
        check_string_len(&self.label, MAX_SHORT_STRING_LEN, "column.label")?;
        check_string_len(
            &self.data_action,
            MAX_SHORT_STRING_LEN,
            "column.data_action",
        )?;
        Ok(())
    }
}

impl WireValidate for extension::ContextSelectorSource {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        match self {
            extension::ContextSelectorSource::Action { action_id } => {
                check_string_len(
                    action_id,
                    MAX_SHORT_STRING_LEN,
                    "context_selector.action_id",
                )?;
            }
            extension::ContextSelectorSource::PluginConfigs { plugin_type } => {
                check_string_len(
                    plugin_type,
                    MAX_SHORT_STRING_LEN,
                    "context_selector.plugin_type",
                )?;
            }
            _ => {
                tracing::warn!("unknown ContextSelectorSource variant, skipping validation");
            }
        }
        Ok(())
    }
}

impl WireValidate for extension::ContextSelectorDef {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(
            &self.param_key,
            MAX_SHORT_STRING_LEN,
            "context_selector.param_key",
        )?;
        check_string_len(&self.label, MAX_SHORT_STRING_LEN, "context_selector.label")?;
        self.source.wire_validate()?;
        check_opt_string_len(
            &self.add_action,
            MAX_SHORT_STRING_LEN,
            "context_selector.add_action",
        )?;
        check_opt_string_len(
            &self.empty_message,
            MAX_SHORT_STRING_LEN,
            "context_selector.empty_message",
        )?;
        Ok(())
    }
}

impl WireValidate for extension::ExtensionUi {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        match self {
            extension::ExtensionUi::DataTable {
                columns,
                data_action,
                row_actions,
                primary_actions,
                context_selector,
                ..
            } => {
                check_vec_len(columns, MAX_EXTENSION_COLUMNS, "ui.columns")?;
                check_string_len(data_action, MAX_SHORT_STRING_LEN, "ui.data_action")?;
                check_vec_len(row_actions, MAX_EXTENSION_ACTION_REFS, "ui.row_actions")?;
                check_vec_len(
                    primary_actions,
                    MAX_EXTENSION_ACTION_REFS,
                    "ui.primary_actions",
                )?;
                for col in columns {
                    col.wire_validate()?;
                }
                for action_id in row_actions {
                    check_string_len(action_id, MAX_SHORT_STRING_LEN, "ui.row_actions[]")?;
                }
                for action_id in primary_actions {
                    check_string_len(action_id, MAX_SHORT_STRING_LEN, "ui.primary_actions[]")?;
                }
                if let Some(cs) = context_selector {
                    cs.wire_validate()?;
                }
            }
            extension::ExtensionUi::Form(form) => {
                form.wire_validate()?;
            }
            extension::ExtensionUi::KeyValue { data_action } => {
                check_string_len(data_action, MAX_SHORT_STRING_LEN, "ui.data_action")?;
            }
            extension::ExtensionUi::Actions { actions } => {
                check_vec_len(actions, MAX_EXTENSION_ACTION_REFS, "ui.actions")?;
                for action_id in actions {
                    check_string_len(action_id, MAX_SHORT_STRING_LEN, "ui.actions[]")?;
                }
            }
            _ => {
                tracing::warn!("unknown ExtensionUi variant, skipping validation");
            }
        }
        Ok(())
    }
}

impl WireValidate for extension::TableColumn {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.key, MAX_SHORT_STRING_LEN, "table_column.key")?;
        check_string_len(&self.label, MAX_SHORT_STRING_LEN, "table_column.label")?;
        Ok(())
    }
}

impl WireValidate for extension::ApiSubmitDef {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.method, MAX_SHORT_STRING_LEN, "api_submit.method")?;
        check_string_len(&self.path, MAX_SHORT_STRING_LEN, "api_submit.path")?;
        check_opt_string_len(
            &self.response_id_field,
            MAX_SHORT_STRING_LEN,
            "api_submit.response_id_field",
        )?;
        check_opt_string_len(
            &self.response_label_field,
            MAX_SHORT_STRING_LEN,
            "api_submit.response_label_field",
        )?;
        Ok(())
    }
}

impl WireValidate for extension::ActionDef {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.action_id, MAX_SHORT_STRING_LEN, "action.action_id")?;
        check_string_len(&self.label, MAX_SHORT_STRING_LEN, "action.label")?;
        check_string_len(&self.permission, MAX_SHORT_STRING_LEN, "action.permission")?;
        if let Some(ui) = &self.ui {
            ui.wire_validate()?;
        }
        if let Some(api_submit) = &self.api_submit {
            api_submit.wire_validate()?;
        }
        Ok(())
    }
}

impl WireValidate for extension::ActionUi {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        match self {
            extension::ActionUi::Form(form) => form.wire_validate()?,
            extension::ActionUi::Wizard { steps } => {
                check_vec_len(steps, MAX_EXTENSION_WIZARD_STEPS, "wizard.steps")?;
                for step in steps {
                    step.wire_validate()?;
                }
            }
            _ => {
                tracing::warn!("unknown ActionUi variant, skipping validation");
            }
        }
        Ok(())
    }
}

impl WireValidate for extension::WizardStep {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.step_id, MAX_SHORT_STRING_LEN, "step.step_id")?;
        check_string_len(&self.label, MAX_SHORT_STRING_LEN, "step.label")?;
        check_opt_string_len(
            &self.submit_action,
            MAX_SHORT_STRING_LEN,
            "step.submit_action",
        )?;
        self.form.wire_validate()?;
        Ok(())
    }
}

impl WireValidate for extension::FormDef {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_vec_len(&self.fields, MAX_EXTENSION_FIELDS, "form.fields")?;
        for field in &self.fields {
            field.wire_validate()?;
        }
        Ok(())
    }
}

impl WireValidate for extension::FieldDef {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.key, MAX_SHORT_STRING_LEN, "field.key")?;
        check_string_len(&self.label, MAX_SHORT_STRING_LEN, "field.label")?;
        check_opt_string_len(&self.placeholder, MAX_SHORT_STRING_LEN, "field.placeholder")?;
        check_opt_string_len(&self.help_text, MAX_MEDIUM_STRING_LEN, "field.help_text")?;
        check_vec_len(&self.options, MAX_EXTENSION_SELECT_OPTIONS, "field.options")?;
        for opt in &self.options {
            opt.wire_validate()?;
        }
        if let Some(ref vw) = self.visible_when {
            vw.wire_validate()?;
        }
        Ok(())
    }
}

impl WireValidate for extension::VisibleWhen {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.field, MAX_SHORT_STRING_LEN, "visible_when.field")?;
        check_vec_len(
            &self.values,
            MAX_EXTENSION_SELECT_OPTIONS,
            "visible_when.values",
        )?;
        for v in &self.values {
            check_string_len(v, MAX_SHORT_STRING_LEN, "visible_when.values[]")?;
        }
        Ok(())
    }
}

impl WireValidate for extension::SelectOption {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.value, MAX_SHORT_STRING_LEN, "option.value")?;
        check_string_len(&self.label, MAX_SHORT_STRING_LEN, "option.label")?;
        Ok(())
    }
}

impl WireValidate for extension::ExtensionRequestPayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.request_id, MAX_SHORT_STRING_LEN, "request_id")?;
        check_string_len(&self.extension_id, MAX_SHORT_STRING_LEN, "extension_id")?;
        check_string_len(&self.action_id, MAX_SHORT_STRING_LEN, "action_id")?;
        let params_len = self.params.to_string().len();
        if params_len > MAX_EXTENSION_PARAMS_LEN {
            return Err(WireValidationError {
                field: "params",
                message: format!(
                    "params JSON is {params_len} bytes, max {MAX_EXTENSION_PARAMS_LEN}"
                ),
            });
        }
        if let Some(ref sp) = self.sensitive_params {
            let sp_len = sp.expose_secret().len();
            if sp_len > MAX_EXTENSION_PARAMS_LEN {
                return Err(WireValidationError {
                    field: "sensitive_params",
                    message: format!(
                        "sensitive_params is {sp_len} bytes, max {MAX_EXTENSION_PARAMS_LEN}"
                    ),
                });
            }
        }
        Ok(())
    }
}

impl WireValidate for extension::ExtensionResponsePayload {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.request_id, MAX_SHORT_STRING_LEN, "request_id")?;
        check_opt_string_len(&self.error, MAX_MEDIUM_STRING_LEN, "error")?;
        let data_len = self.data.to_string().len();
        if data_len > MAX_EXTENSION_RESPONSE_LEN {
            return Err(WireValidationError {
                field: "data",
                message: format!(
                    "response data is {data_len} bytes, max {MAX_EXTENSION_RESPONSE_LEN}"
                ),
            });
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

    fn test_manifest() -> extension::ExtensionManifest {
        extension::ExtensionManifest::new(
            "test.ext",
            "Test",
            0,
            extension::ExtensionPlacement::Page {
                nav_section: "test".to_string(),
                icon: None,
            },
            extension::ExtensionUi::Actions { actions: vec![] },
        )
    }

    #[test]
    fn extension_register_validates() {
        let payload = extension::ExtensionRegisterPayload::new(vec![test_manifest()]);
        assert!(payload.wire_validate().is_ok());
    }

    #[test]
    fn extension_register_too_many_manifests() {
        let manifests: Vec<extension::ExtensionManifest> = (0..MAX_EXTENSION_MANIFESTS + 1)
            .map(|i| {
                extension::ExtensionManifest::new(
                    format!("ext-{i}"),
                    "Test",
                    0,
                    extension::ExtensionPlacement::Page {
                        nav_section: "test".to_string(),
                        icon: None,
                    },
                    extension::ExtensionUi::Actions { actions: vec![] },
                )
            })
            .collect();
        let payload = extension::ExtensionRegisterPayload::new(manifests);
        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "manifests");
    }

    #[test]
    fn extension_manifest_id_too_long() {
        let manifest = extension::ExtensionManifest::new(
            "x".repeat(MAX_SHORT_STRING_LEN + 1),
            "Test",
            0,
            extension::ExtensionPlacement::Page {
                nav_section: "test".to_string(),
                icon: None,
            },
            extension::ExtensionUi::Actions { actions: vec![] },
        );
        let err = manifest.wire_validate().unwrap_err();
        assert_eq!(err.field, "extension.id");
    }

    #[test]
    fn extension_table_columns_too_many() {
        let columns: Vec<extension::ExtensionColumn> = (0..MAX_EXTENSION_COLUMNS + 1)
            .map(|i| {
                extension::ExtensionColumn::new(format!("col-{i}"), format!("Column {i}"), "fetch")
            })
            .collect();
        let placement = extension::ExtensionPlacement::TableColumns {
            target_table: "hosts".to_string(),
            columns,
        };
        let err = placement.wire_validate().unwrap_err();
        assert_eq!(err.field, "placement.columns");
    }

    #[test]
    fn extension_form_too_many_fields() {
        let fields: Vec<extension::FieldDef> = (0..MAX_EXTENSION_FIELDS + 1)
            .map(|i| extension::FieldDef::new(format!("field-{i}"), format!("Field {i}")))
            .collect();
        let form = extension::FormDef::new(fields);
        let err = form.wire_validate().unwrap_err();
        assert_eq!(err.field, "form.fields");
    }

    #[test]
    fn extension_wizard_too_many_steps() {
        let steps: Vec<extension::WizardStep> = (0..MAX_EXTENSION_WIZARD_STEPS + 1)
            .map(|i| {
                extension::WizardStep::new(
                    format!("s-{i}"),
                    format!("Step {i}"),
                    extension::FormDef::new(vec![]),
                )
            })
            .collect();
        let ui = extension::ActionUi::Wizard { steps };
        let err = ui.wire_validate().unwrap_err();
        assert_eq!(err.field, "wizard.steps");
    }

    #[test]
    fn extension_select_too_many_options() {
        let options: Vec<extension::SelectOption> = (0..MAX_EXTENSION_SELECT_OPTIONS + 1)
            .map(|i| extension::SelectOption::new(format!("v-{i}"), format!("Label {i}")))
            .collect();
        let field = extension::FieldDef::new("select", "Select")
            .with_type(extension::FieldType::Select)
            .with_options(options);
        let err = field.wire_validate().unwrap_err();
        assert_eq!(err.field, "field.options");
    }

    #[test]
    fn extension_request_validates() {
        let payload = extension::ExtensionRequestPayload {
            request_id: "req-1".to_string(),
            extension_id: "test.ext".to_string(),
            action_id: "do-thing".to_string(),
            params: serde_json::json!({}),
            sensitive_params: None,
            tenant_id: None,
        };
        assert!(payload.wire_validate().is_ok());
    }

    #[test]
    fn extension_request_params_too_large() {
        let big_value = "x".repeat(MAX_EXTENSION_PARAMS_LEN + 1);
        let payload = extension::ExtensionRequestPayload {
            request_id: "req-1".to_string(),
            extension_id: "test.ext".to_string(),
            action_id: "do-thing".to_string(),
            params: serde_json::Value::String(big_value),
            sensitive_params: None,
            tenant_id: None,
        };
        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "params");
    }

    #[test]
    fn extension_response_validates() {
        let payload = extension::ExtensionResponsePayload {
            request_id: "req-1".to_string(),
            success: true,
            data: serde_json::json!({"ok": true}),
            error: None,
        };
        assert!(payload.wire_validate().is_ok());
    }

    #[test]
    fn extension_response_data_too_large() {
        let big_value = "x".repeat(MAX_EXTENSION_RESPONSE_LEN + 1);
        let payload = extension::ExtensionResponsePayload {
            request_id: "req-1".to_string(),
            success: false,
            data: serde_json::Value::String(big_value),
            error: None,
        };
        let err = payload.wire_validate().unwrap_err();
        assert_eq!(err.field, "data");
    }

    #[test]
    fn extension_service_message_register_validates() {
        let msg =
            ServiceMessage::ExtensionRegister(extension::ExtensionRegisterPayload::new(vec![
                test_manifest(),
            ]));
        assert!(msg.wire_validate().is_ok());
    }

    #[test]
    fn extension_service_message_response_validates() {
        let msg = ServiceMessage::ExtensionResponse(extension::ExtensionResponsePayload {
            request_id: "r1".to_string(),
            success: true,
            data: serde_json::Value::Null,
            error: None,
        });
        assert!(msg.wire_validate().is_ok());
    }

    #[test]
    fn extension_controller_message_request_validates() {
        let msg = ControllerMessage::ExtensionRequest(extension::ExtensionRequestPayload {
            request_id: "r1".to_string(),
            extension_id: "test.ext".to_string(),
            action_id: "action".to_string(),
            params: serde_json::json!({}),
            sensitive_params: None,
            tenant_id: None,
        });
        assert!(msg.wire_validate().is_ok());
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
