use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use serde::Serialize;
use std::ffi::OsString;
use uptrakit_internal_wire::extension::{
    ActionDef, ActionUi, ExtensionManifest, ExtensionTargeting, ExtensionUi, FieldDef, FieldType,
    FormDef,
};
use uptrakit_openapi_client::types::extensions::{ExtensionProviderInfo, ExtensionResponse};
use uuid::Uuid;

// ── HumanOutput impls ──────────────────────────────────────────────────────

impl HumanOutput for Vec<ExtensionResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No extensions registered.\n".to_string();
        }
        let mut out = String::new();
        out.push_str(&format!(
            "{:<40} {:<30} {:<15} {:<10}\n",
            "ID", "Label", "Placement", "Providers"
        ));
        out.push_str(&format!("{}\n", "-".repeat(95)));
        for ext in self {
            let placement = format!("{:?}", ext.manifest.placement);
            // Truncate placement type for display.
            let placement = placement
                .split_once(['{', '('])
                .map_or(placement.as_str(), |(prefix, _)| prefix.trim());
            out.push_str(&format!(
                "{:<40} {:<30} {:<15} {:<10}\n",
                ext.manifest.id, ext.manifest.label, placement, ext.provider_count
            ));
        }
        out
    }
}

impl HumanOutput for Vec<ExtensionProviderInfo> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No providers connected.\n".to_string();
        }
        let mut out = String::new();
        out.push_str(&format!(
            "{:<38} {:<30} {}\n",
            "Service ID", "Label", "Hostname"
        ));
        out.push_str(&format!("{}\n", "-".repeat(80)));
        for p in self {
            out.push_str(&format!(
                "{:<38} {:<30} {}\n",
                p.service_id,
                p.service_label,
                p.hostname.as_deref().unwrap_or("-")
            ));
        }
        out
    }
}

/// Wrapper for invoke action response output.
#[derive(Debug, Serialize)]
pub struct InvokeOutput(pub serde_json::Value);

impl HumanOutput for InvokeOutput {
    fn to_human_string(&self) -> String {
        serde_json::to_string_pretty(&self.0).unwrap_or_else(|_| self.0.to_string()) + "\n"
    }
}

// ── Params ─────────────────────────────────────────────────────────────────

pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ProvidersParams<'a> {
    pub extension_id: String,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct InvokeParams<'a> {
    pub extension_id: String,
    pub action_id: String,
    pub params: serde_json::Value,
    pub service_id: Option<Uuid>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Commands ───────────────────────────────────────────────────────────────

pub async fn list(params: ListParams<'_>) -> Result<Vec<ExtensionResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.list_extensions().await.context_to()
}

pub async fn providers(params: ProvidersParams<'_>) -> Result<Vec<ExtensionProviderInfo>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .list_extension_providers(&params.extension_id)
        .await
        .context_to()
}

pub async fn invoke(params: InvokeParams<'_>) -> Result<InvokeOutput> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let result = client
        .invoke_extension_action(
            &params.extension_id,
            &params.action_id,
            params.params,
            None,
            params.service_id.as_ref(),
        )
        .await
        .context_to()?;
    Ok(InvokeOutput(result))
}

// ── Dynamic extension subcommands ──────────────────────────────────────────

/// Execute a dynamic extension command with manifest-driven argument parsing.
///
/// `args[0]` is the extension ID. The remaining args are parsed against a
/// clap `Command` built dynamically from the extension's manifest.
pub async fn dynamic_invoke(
    args: Vec<OsString>,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<InvokeOutput> {
    use crate::error::CliError;

    if args.is_empty() {
        return Err(report!(CliError::Other(
            "extension ID is required (e.g., `extensions ssh-agent.hosts list-hosts`)".to_string()
        )));
    }

    let extension_id = args[0]
        .to_str()
        .ok_or_else(|| {
            report!(CliError::Other(
                "extension ID must be valid UTF-8".to_string()
            ))
        })?
        .to_string();

    // Fetch all extensions from the server and find the matching manifest.
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let extensions = client.list_extensions().await.context_to()?;
    let ext = extensions
        .iter()
        .find(|e| e.manifest.id == extension_id)
        .ok_or_else(|| {
            report!(CliError::Other(format!(
                "extension '{extension_id}' not found on the server"
            )))
        })?;

    // Build a clap Command from the manifest.
    let cmd = build_extension_command(&ext.manifest);

    // Parse the remaining args (everything after extension_id).
    // clap's try_get_matches_from handles --help by printing and exiting.
    let matches = match cmd.try_get_matches_from(&args[1..]) {
        Ok(m) => m,
        Err(e) => e.exit(),
    };

    // Extract --service-id if present (only for targeted extensions).
    let service_id: Option<Uuid> = matches
        .get_one::<String>("service-id")
        .and_then(|s| s.parse().ok());

    // Get the matched subcommand (action).
    let (action_id, action_matches) = matches.subcommand().ok_or_else(|| {
        report!(CliError::Other(
            "action is required; use --help to see available actions".to_string()
        ))
    })?;

    // Extract params from the action matches.
    let actions = collect_actions(&ext.manifest.ui);
    let action_def = actions.iter().find(|a| a.action_id == action_id);
    let params = extract_params_from_matches(action_matches, action_def);

    // Invoke the action.
    let result = client
        .invoke_extension_action(&extension_id, action_id, params, None, service_id.as_ref())
        .await
        .context_to()?;
    Ok(InvokeOutput(result))
}

/// Build a `clap::Command` from an extension manifest.
///
/// The top-level command has `--service-id` (only for targeted extensions)
/// and one subcommand per action. Each action's arguments are built from
/// its form fields.
fn build_extension_command(manifest: &ExtensionManifest) -> clap::Command {
    let mut cmd = clap::Command::new(manifest.id.clone())
        .about(manifest.label.clone())
        .subcommand_required(true);

    // Add --service-id only for targeted extensions.
    if manifest.targeting == ExtensionTargeting::Targeted {
        cmd = cmd.arg(
            clap::Arg::new("service-id")
                .long("service-id")
                .help("Service instance UUID (required for targeted extensions)")
                .required(true)
                .global(true),
        );
    }

    // Collect all actions from the manifest UI.
    let actions = collect_actions(&manifest.ui);

    for action in &actions {
        let mut subcmd = clap::Command::new(action.action_id.clone()).about(action.label.clone());

        // Add form field args if the action has a form UI.
        if let Some(fields) = action_form_fields(action) {
            for field in fields {
                subcmd = subcmd.arg(build_arg_from_field(field));
            }
        }

        // Row actions get a positional `id` argument.
        if is_row_action(action, &manifest.ui) {
            subcmd = subcmd.arg(
                clap::Arg::new("id")
                    .help("Row identifier")
                    .required(true)
                    .index(1),
            );
        }

        cmd = cmd.subcommand(subcmd);
    }

    // Add the data_action as a subcommand too (e.g., list-hosts).
    if let Some(data_action) = data_action_id(&manifest.ui)
        && !actions.iter().any(|a| a.action_id == data_action)
    {
        cmd = cmd.subcommand(
            clap::Command::new(data_action.to_string()).about("Fetch data for this extension"),
        );
    }

    cmd
}

/// Build a clap `Arg` from a `FieldDef`.
fn build_arg_from_field(field: &FieldDef) -> clap::Arg {
    let mut arg = clap::Arg::new(field.key.clone()).long(field.key.clone());

    // Help text.
    let mut help_parts = Vec::new();
    if let Some(ref ht) = field.help_text {
        help_parts.push(ht.clone());
    }
    if let Some(ref ph) = field.placeholder {
        help_parts.push(format!("(e.g., {ph})"));
    }
    if !help_parts.is_empty() {
        arg = arg.help(help_parts.join(" "));
    }

    // Required.
    if field.required {
        arg = arg.required(true);
    }

    // Default value.
    if let Some(ref dv) = field.default_value {
        match dv {
            serde_json::Value::String(s) => {
                arg = arg.default_value(s.clone());
            }
            serde_json::Value::Bool(b) => {
                if *b {
                    arg = arg.default_value("true");
                }
            }
            other => {
                arg = arg.default_value(other.to_string());
            }
        }
    }

    // Type-specific behavior.
    match &field.field_type {
        FieldType::Toggle => {
            arg = arg.action(clap::ArgAction::SetTrue).required(false);
        }
        FieldType::Number => {
            arg = arg.value_parser(clap::value_parser!(f64));
        }
        FieldType::Select => {
            let values: Vec<clap::builder::PossibleValue> = field
                .options
                .iter()
                .map(|o| clap::builder::PossibleValue::new(o.value.clone()).help(o.label.clone()))
                .collect();
            if !values.is_empty() {
                arg = arg.value_parser(clap::builder::PossibleValuesParser::new(values));
            }
        }
        FieldType::Hidden => {
            arg = arg.hide(true);
        }
        // Text, Password, Textarea — default string arg.
        _ => {}
    }

    arg
}

/// Extract action parameters from clap matches into a JSON Value.
fn extract_params_from_matches(
    matches: &clap::ArgMatches,
    action_def: Option<&&ActionDef>,
) -> serde_json::Value {
    let mut params = serde_json::Map::new();

    // Extract positional `id` if present.
    if let Some(id) = matches.get_one::<String>("id") {
        params.insert("id".to_string(), serde_json::Value::String(id.clone()));
    }

    // Extract form fields if we have an action definition.
    if let Some(action) = action_def
        && let Some(fields) = action_form_fields(action)
    {
        for field in fields {
            match &field.field_type {
                FieldType::Toggle => {
                    if matches.get_flag(field.key.as_str()) {
                        params.insert(field.key.clone(), serde_json::Value::Bool(true));
                    }
                }
                FieldType::Number => {
                    if let Some(val) = matches.get_one::<f64>(field.key.as_str()) {
                        params.insert(field.key.clone(), serde_json::json!(*val));
                    }
                }
                _ => {
                    if let Some(val) = matches.get_one::<String>(field.key.as_str()) {
                        params.insert(field.key.clone(), serde_json::Value::String(val.clone()));
                    }
                }
            }
        }
    }

    serde_json::Value::Object(params)
}

// ── Helpers for manifest introspection ─────────────────────────────────────

/// Collect all actions from an extension UI definition.
fn collect_actions(ui: &ExtensionUi) -> Vec<&ActionDef> {
    match ui {
        ExtensionUi::DataTable {
            row_actions,
            primary_actions,
            context_selector,
            ..
        } => {
            let mut v: Vec<&ActionDef> = row_actions.iter().chain(primary_actions.iter()).collect();
            if let Some(cs) = context_selector
                && let Some(add_action) = &cs.add_action
            {
                v.push(add_action);
            }
            v
        }
        ExtensionUi::Actions { actions, .. } => actions.iter().collect(),
        ExtensionUi::Form(_) | ExtensionUi::KeyValue { .. } => vec![],
        _ => vec![],
    }
}

/// Get the data_action ID from a DataTable or KeyValue UI.
fn data_action_id(ui: &ExtensionUi) -> Option<&str> {
    match ui {
        ExtensionUi::DataTable { data_action, .. } => Some(data_action.as_str()),
        ExtensionUi::KeyValue { data_action, .. } => Some(data_action.as_str()),
        _ => None,
    }
}

/// Extract form fields from an action definition.
fn action_form_fields(action: &ActionDef) -> Option<&[FieldDef]> {
    match &action.ui {
        Some(ActionUi::Form(FormDef { fields, .. })) => Some(fields),
        _ => None,
    }
}

/// Check if an action is a row action (appears in `row_actions`).
fn is_row_action(action: &ActionDef, ui: &ExtensionUi) -> bool {
    match ui {
        ExtensionUi::DataTable { row_actions, .. } => {
            row_actions.iter().any(|a| a.action_id == action.action_id)
        }
        _ => false,
    }
}
