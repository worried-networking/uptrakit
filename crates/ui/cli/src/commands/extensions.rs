use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use serde::Serialize;
use std::ffi::OsString;
use uptrakit_internal_wire::extension::{
    ActionDef, ActionUi, ApiSubmitDef, ExtensionManifest, ExtensionTargeting, ExtensionUi,
    FieldDef, FieldType, FormDef,
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
            "ID", "Label", "Placement", "Source"
        ));
        out.push_str(&format!("{}\n", "-".repeat(95)));
        for ext in self {
            let placement = format!("{:?}", ext.manifest.placement);
            // Truncate placement type for display.
            let placement = placement
                .split_once(['{', '('])
                .map_or(placement.as_str(), |(prefix, _)| prefix.trim());
            let source = if ext.provider_count == 0 {
                "built-in".to_string()
            } else {
                format!("{} provider(s)", ext.provider_count)
            };
            out.push_str(&format!(
                "{:<40} {:<30} {:<15} {:<10}\n",
                ext.manifest.id, ext.manifest.label, placement, source
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

    // Build a clap Command from the manifest and its resolved actions.
    let cmd = build_extension_command(&ext.manifest, &ext.actions);

    // Parse the remaining args (everything after extension_id).
    // clap's try_get_matches_from handles --help by printing and exiting.
    let matches = match cmd.try_get_matches_from(&args[1..]) {
        Ok(m) => m,
        Err(e) => e.exit(),
    };

    // Extract --service-id only when the arg was registered (Targeted extensions).
    let service_id: Option<Uuid> = if ext.manifest.targeting == ExtensionTargeting::Targeted {
        let raw = matches.get_one::<String>("service-id").ok_or_else(|| {
            report!(CliError::Other(
                "targeted extension requires --service-id <UUID>".to_string()
            ))
        })?;
        let parsed = raw
            .parse::<Uuid>()
            .map_err(|e| report!(CliError::Other(format!("invalid --service-id UUID: {e}"))))?;
        Some(parsed)
    } else {
        None
    };

    // Extract context selector param if present (e.g., --plugin-config-id).
    let context_param = extract_context_selector_param(&ext.manifest.ui, &matches);

    // Get the matched subcommand (action).
    let (action_id, action_matches) = matches.subcommand().ok_or_else(|| {
        report!(CliError::Other(
            "action is required; use --help to see available actions".to_string()
        ))
    })?;

    // Extract params from the action matches.
    let action_def = ext.actions.iter().find(|a| a.action_id == action_id);
    let mut params = extract_params_from_matches(action_matches, action_def);

    // Inject the context selector param into the action params.
    if let Some((key, val)) = context_param
        && let serde_json::Value::Object(ref mut map) = params
    {
        map.entry(key).or_insert(serde_json::Value::String(val));
    }

    // If the action has api_submit, call the REST API directly instead of the
    // extension proxy. api_submit actions are not handled by plugin extension
    // handlers — they are designed for the frontend to call the REST API.
    if let Some(action) = action_def
        && let Some(ref api_submit) = action.api_submit
    {
        return execute_api_submit(&client, api_submit, &params).await;
    }

    // Invoke the action via the extension proxy.
    let result = client
        .invoke_extension_action(&extension_id, action_id, params, None, service_id.as_ref())
        .await
        .context_to()?;
    Ok(InvokeOutput(result))
}

/// Build a `clap::Command` from an extension manifest and its action catalogue.
///
/// The top-level command has `--service-id` (only for targeted extensions),
/// an optional context selector param (e.g., `--plugin-config-id`), and one
/// subcommand per action. Each action's arguments are built from its form fields.
fn build_extension_command(manifest: &ExtensionManifest, actions: &[ActionDef]) -> clap::Command {
    let mut cmd = clap::Command::new(manifest.id.clone())
        .about(manifest.label.clone())
        .subcommand_required(true);

    // Add --service-id only for targeted extensions. Marked global so it can
    // appear before or after the subcommand. Not marked `required` at the clap
    // level (clap forbids global + required); instead validated in dynamic_invoke.
    if manifest.targeting == ExtensionTargeting::Targeted {
        cmd = cmd.arg(
            clap::Arg::new("service-id")
                .long("service-id")
                .help("Service instance UUID (required for targeted extensions)")
                .global(true),
        );
    }

    // Add context selector param as a global arg (e.g., --plugin-config-id).
    if let Some((param_key, label)) = context_selector_info(&manifest.ui) {
        let long_name = param_key.replace('_', "-");
        cmd = cmd.arg(
            clap::Arg::new(param_key.to_string())
                .long(long_name)
                .help(format!("{label} (injected into all action params)"))
                .global(true),
        );
    }

    // Collect action IDs referenced by the UI.
    let referenced_ids = collect_action_ids(&manifest.ui);

    for action in actions {
        // Only include actions referenced by this extension's UI.
        if !referenced_ids.contains(&action.action_id.as_str()) {
            continue;
        }

        let mut subcmd = clap::Command::new(action.action_id.clone()).about(action.label.clone());

        // Add form field args if the action has a form UI.
        if let Some(fields) = action_form_fields(action) {
            for field in fields {
                subcmd = subcmd.arg(build_arg_from_field(field));
            }
        }

        // Row actions get a positional `id` argument.
        if is_row_action(&action.action_id, &manifest.ui) {
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
    action_def: Option<&ActionDef>,
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

// ── api_submit support ─────────────────────────────────────────────────────

/// Execute an action by calling the REST API directly via `api_submit`.
///
/// This is used for actions that define an `api_submit` target — the CLI
/// renders the body template, substitutes form-field values, and calls the
/// REST endpoint instead of routing through the extension proxy.
async fn execute_api_submit(
    client: &uptrakit_openapi_client::UptrakitClient,
    api_submit: &ApiSubmitDef,
    params: &serde_json::Value,
) -> Result<InvokeOutput> {
    use crate::error::CliError;

    let body = render_api_submit_body(&api_submit.body, params);
    let resp = client
        .raw_request(&api_submit.method, &api_submit.path, Some(body))
        .await
        .context_to()?;

    if resp.status.is_success() {
        Ok(InvokeOutput(resp.body))
    } else {
        let message = resp
            .body
            .get("error")
            .and_then(|v| v.as_str())
            .or_else(|| resp.body.get("message").and_then(|v| v.as_str()))
            .unwrap_or("Request failed")
            .to_string();
        Err(report!(CliError::Api {
            status: resp.status,
            message,
        }))
    }
}

/// Render an `api_submit` body template by substituting `{{field}}` and
/// `{{field:coercion}}` placeholders with values from the action params.
///
/// The template is a JSON value tree. Each string leaf that exactly matches
/// `{{key}}` or `{{key:coercion}}` is replaced with the corresponding value
/// from `params`. Non-matching strings and non-string leaves pass through
/// unchanged.
///
/// ## Coercion types
///
/// | Syntax | Effect |
/// |--------|--------|
/// | `{{key}}` | String (default) |
/// | `{{key:bool}}` | `"true"` → `true`, anything else → `false` |
/// | `{{key:csv_array}}` | Split on `,`, trim, drop empties → JSON array |
/// | `{{key:number}}` | Parse as JSON number |
fn render_api_submit_body(
    template: &serde_json::Value,
    params: &serde_json::Value,
) -> serde_json::Value {
    match template {
        serde_json::Value::String(s) => apply_template_string(s, params),
        serde_json::Value::Object(map) => {
            let rendered: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), render_api_submit_body(v, params)))
                .collect();
            serde_json::Value::Object(rendered)
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.iter()
                .map(|v| render_api_submit_body(v, params))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Apply template substitution to a single string value.
///
/// Returns the original string as a JSON string if it does not match the
/// `{{key}}` / `{{key:coercion}}` pattern.
fn apply_template_string(s: &str, params: &serde_json::Value) -> serde_json::Value {
    // Must match the full string: {{key}} or {{key:coercion}}.
    if !s.starts_with("{{") || !s.ends_with("}}") || s.len() < 5 {
        return serde_json::Value::String(s.to_string());
    }

    let inner = &s[2..s.len() - 2];

    // Reject if the inner part contains nested {{ or }}.
    if inner.contains("{{") || inner.contains("}}") {
        return serde_json::Value::String(s.to_string());
    }

    let (key, coercion) = match inner.split_once(':') {
        Some((k, c)) => (k, Some(c)),
        None => (inner, None),
    };

    let raw_value = params.get(key);

    match coercion {
        None => {
            // Default: return as string.
            match raw_value {
                Some(serde_json::Value::String(v)) => serde_json::Value::String(v.clone()),
                Some(v) => serde_json::Value::String(v.to_string()),
                None => serde_json::Value::Null,
            }
        }
        Some("bool") => {
            let v = raw_value.and_then(serde_json::Value::as_str).unwrap_or("");
            serde_json::Value::Bool(v == "true")
        }
        Some("number") => {
            let v = raw_value.and_then(serde_json::Value::as_str).unwrap_or("0");
            // Try integer first, then float.
            if let Ok(n) = v.parse::<i64>() {
                serde_json::json!(n)
            } else if let Ok(n) = v.parse::<f64>() {
                serde_json::json!(n)
            } else {
                serde_json::Value::Null
            }
        }
        Some("csv_array") => {
            let v = raw_value.and_then(serde_json::Value::as_str).unwrap_or("");
            let items: Vec<serde_json::Value> = v
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| serde_json::Value::String(s.to_string()))
                .collect();
            serde_json::Value::Array(items)
        }
        Some(_) => {
            // Unknown coercion — treat as string.
            match raw_value {
                Some(serde_json::Value::String(v)) => serde_json::Value::String(v.clone()),
                Some(v) => serde_json::Value::String(v.to_string()),
                None => serde_json::Value::Null,
            }
        }
    }
}

// ── Helpers for manifest introspection ─────────────────────────────────────

/// Collect all action ID strings referenced from an extension UI definition.
fn collect_action_ids(ui: &ExtensionUi) -> Vec<&str> {
    match ui {
        ExtensionUi::DataTable {
            row_actions,
            primary_actions,
            context_selector,
            ..
        } => {
            let mut ids: Vec<&str> = row_actions
                .iter()
                .chain(primary_actions.iter())
                .map(String::as_str)
                .collect();
            if let Some(cs) = context_selector
                && let Some(add_action) = &cs.add_action
            {
                ids.push(add_action.as_str());
            }
            ids
        }
        ExtensionUi::Actions { actions, .. } => actions.iter().map(String::as_str).collect(),
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

/// Extract context selector metadata (param_key, label) from a DataTable UI.
fn context_selector_info(ui: &ExtensionUi) -> Option<(&str, &str)> {
    match ui {
        ExtensionUi::DataTable {
            context_selector: Some(cs),
            ..
        } => Some((cs.param_key.as_str(), cs.label.as_str())),
        _ => None,
    }
}

/// Extract the context selector param value from top-level clap matches.
///
/// Returns `Some((param_key, value))` if the extension has a context selector
/// and the user supplied the corresponding CLI flag.
fn extract_context_selector_param(
    ui: &ExtensionUi,
    matches: &clap::ArgMatches,
) -> Option<(String, String)> {
    let (param_key, _) = context_selector_info(ui)?;
    let value = matches.get_one::<String>(param_key)?;
    Some((param_key.to_string(), value.clone()))
}

/// Extract form fields from an action definition.
fn action_form_fields(action: &ActionDef) -> Option<&[FieldDef]> {
    match &action.ui {
        Some(ActionUi::Form(FormDef { fields, .. })) => Some(fields),
        _ => None,
    }
}

/// Check if an action ID is a row action (appears in `row_actions`).
fn is_row_action(action_id: &str, ui: &ExtensionUi) -> bool {
    match ui {
        ExtensionUi::DataTable { row_actions, .. } => row_actions.iter().any(|a| a == action_id),
        _ => false,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_internal_wire::extension::{
        ContextSelectorDef, ContextSelectorSource, TableColumn,
    };

    // ── render_api_submit_body ──────────────────────────────────────────

    #[test]
    fn template_string_substitution() {
        let template = serde_json::json!({"name": "{{name}}"});
        let params = serde_json::json!({"name": "test-config"});
        let result = render_api_submit_body(&template, &params);
        assert_eq!(result, serde_json::json!({"name": "test-config"}));
    }

    #[test]
    fn template_bool_coercion() {
        let template = serde_json::json!({"enabled": "{{flag:bool}}"});
        let params = serde_json::json!({"flag": "true"});
        assert_eq!(
            render_api_submit_body(&template, &params),
            serde_json::json!({"enabled": true})
        );

        let params_false = serde_json::json!({"flag": "false"});
        assert_eq!(
            render_api_submit_body(&template, &params_false),
            serde_json::json!({"enabled": false})
        );
    }

    #[test]
    fn template_number_coercion() {
        let template = serde_json::json!({"count": "{{n:number}}"});
        let params = serde_json::json!({"n": "42"});
        assert_eq!(
            render_api_submit_body(&template, &params),
            serde_json::json!({"count": 42})
        );

        let params_float = serde_json::json!({"n": "1.5"});
        assert_eq!(
            render_api_submit_body(&template, &params_float),
            serde_json::json!({"count": 1.5})
        );

        let params_invalid = serde_json::json!({"n": "abc"});
        assert_eq!(
            render_api_submit_body(&template, &params_invalid),
            serde_json::json!({"count": null})
        );
    }

    #[test]
    fn template_csv_array_coercion() {
        let template = serde_json::json!({"tags": "{{list:csv_array}}"});
        let params = serde_json::json!({"list": "a, b , c"});
        assert_eq!(
            render_api_submit_body(&template, &params),
            serde_json::json!({"tags": ["a", "b", "c"]})
        );

        // Empty segments are dropped.
        let params_empty = serde_json::json!({"list": "x,,y,"});
        assert_eq!(
            render_api_submit_body(&template, &params_empty),
            serde_json::json!({"tags": ["x", "y"]})
        );
    }

    #[test]
    fn template_missing_param_produces_null() {
        let template = serde_json::json!({"name": "{{missing}}"});
        let params = serde_json::json!({});
        assert_eq!(
            render_api_submit_body(&template, &params),
            serde_json::json!({"name": null})
        );
    }

    #[test]
    fn template_non_template_string_passes_through() {
        let template = serde_json::json!({"type": "infrastructure_proxmox"});
        let params = serde_json::json!({});
        assert_eq!(
            render_api_submit_body(&template, &params),
            serde_json::json!({"type": "infrastructure_proxmox"})
        );
    }

    #[test]
    fn template_nested_object() {
        let template = serde_json::json!({
            "name": "{{name}}",
            "enabled": true,
            "config": {
                "url": "{{url}}",
                "verify": "{{verify:bool}}"
            }
        });
        let params = serde_json::json!({
            "name": "my-config",
            "url": "https://example.com",
            "verify": "true"
        });
        assert_eq!(
            render_api_submit_body(&template, &params),
            serde_json::json!({
                "name": "my-config",
                "enabled": true,
                "config": {
                    "url": "https://example.com",
                    "verify": true
                }
            })
        );
    }

    #[test]
    fn template_array_in_body() {
        let template = serde_json::json!({"items": ["{{a}}", "{{b}}"]});
        let params = serde_json::json!({"a": "x", "b": "y"});
        assert_eq!(
            render_api_submit_body(&template, &params),
            serde_json::json!({"items": ["x", "y"]})
        );
    }

    #[test]
    fn template_non_string_leaves_pass_through() {
        let template = serde_json::json!({"count": 42, "active": true, "data": null});
        let params = serde_json::json!({});
        assert_eq!(
            render_api_submit_body(&template, &params),
            serde_json::json!({"count": 42, "active": true, "data": null})
        );
    }

    #[test]
    fn template_unknown_coercion_treated_as_string() {
        let template = serde_json::json!({"val": "{{x:custom}}"});
        let params = serde_json::json!({"x": "hello"});
        assert_eq!(
            render_api_submit_body(&template, &params),
            serde_json::json!({"val": "hello"})
        );
    }

    #[test]
    fn template_short_string_not_treated_as_template() {
        // "{{a}}" is 5 chars — minimum valid template.
        let template = serde_json::json!({"v": "{{a}}"});
        let params = serde_json::json!({"a": "ok"});
        assert_eq!(
            render_api_submit_body(&template, &params),
            serde_json::json!({"v": "ok"})
        );

        // "{{}}" is 4 chars — too short, not a template.
        let template_short = serde_json::json!({"v": "{{}}"});
        assert_eq!(
            render_api_submit_body(&template_short, &params),
            serde_json::json!({"v": "{{}}"})
        );
    }

    // ── build_extension_command ─────────────────────────────────────────

    fn test_manifest(targeting: ExtensionTargeting) -> ExtensionManifest {
        ExtensionManifest::new(
            "test.ext",
            "Test Extension",
            0,
            uptrakit_internal_wire::extension::ExtensionPlacement::Page {
                nav_section: "test".to_string(),
                icon: None,
            },
            ExtensionUi::DataTable {
                columns: vec![TableColumn::new("col", "Column")],
                data_action: "list".to_string(),
                row_actions: vec!["edit".to_string()],
                primary_actions: vec!["create".to_string()],
                context_selector: None,
                default_per_page: None,
            },
        )
        .with_targeting(targeting)
    }

    fn test_actions() -> Vec<ActionDef> {
        vec![
            ActionDef::new("edit", "Edit"),
            ActionDef::new("create", "Create"),
        ]
    }

    #[test]
    fn universal_extension_has_no_service_id_arg() {
        let manifest = test_manifest(ExtensionTargeting::Universal);
        let cmd = build_extension_command(&manifest, &test_actions());
        let result = cmd.try_get_matches_from(["test.ext", "create"]);
        assert!(result.is_ok());
        let m = result.expect("parse should succeed");
        // Accessing "service-id" would panic if we tried get_one; verify no panic.
        assert!(m.subcommand().is_some());
    }

    #[test]
    fn targeted_extension_has_service_id_arg() {
        let manifest = test_manifest(ExtensionTargeting::Targeted);
        let cmd = build_extension_command(&manifest, &test_actions());

        // --service-id is optional at the clap level (validated in dynamic_invoke).
        // Without it, clap still succeeds.
        let result = cmd.clone().try_get_matches_from(["test.ext", "create"]);
        assert!(result.is_ok());
        let m = result.expect("parse should succeed");
        assert!(m.get_one::<String>("service-id").is_none());

        // With --service-id, the value is available.
        let result = cmd.try_get_matches_from([
            "test.ext",
            "--service-id",
            "00000000-0000-0000-0000-000000000001",
            "create",
        ]);
        assert!(result.is_ok());
        let m = result.expect("parse should succeed");
        assert_eq!(
            m.get_one::<String>("service-id").map(String::as_str),
            Some("00000000-0000-0000-0000-000000000001")
        );
    }

    #[test]
    fn context_selector_adds_global_arg() {
        let manifest = ExtensionManifest::new(
            "ctx.ext",
            "Context Extension",
            0,
            uptrakit_internal_wire::extension::ExtensionPlacement::Page {
                nav_section: "test".to_string(),
                icon: None,
            },
            ExtensionUi::DataTable {
                columns: vec![TableColumn::new("col", "Column")],
                data_action: "list".to_string(),
                row_actions: vec![],
                primary_actions: vec!["discover".to_string()],
                default_per_page: None,
                context_selector: Some(Box::new(ContextSelectorDef::new(
                    "plugin_config_id",
                    "Configuration",
                    ContextSelectorSource::PluginConfigs {
                        plugin_type: "test_plugin".to_string(),
                    },
                ))),
            },
        );
        let actions = vec![ActionDef::new("discover", "Discover")];
        let cmd = build_extension_command(&manifest, &actions);

        // Should accept --plugin-config-id as a global arg.
        let result =
            cmd.try_get_matches_from(["ctx.ext", "--plugin-config-id", "some-uuid", "discover"]);
        assert!(result.is_ok());
        let m = result.expect("parse should succeed");
        let val = m.get_one::<String>("plugin_config_id");
        assert_eq!(val.map(String::as_str), Some("some-uuid"));
    }

    #[test]
    fn extract_context_selector_param_from_matches() {
        let ui = ExtensionUi::DataTable {
            columns: vec![],
            data_action: "list".to_string(),
            row_actions: vec![],
            primary_actions: vec![],
            context_selector: Some(Box::new(ContextSelectorDef::new(
                "config_id",
                "Config",
                ContextSelectorSource::PluginConfigs {
                    plugin_type: "test".to_string(),
                },
            ))),
            default_per_page: None,
        };

        let mut cmd = clap::Command::new("test")
            .arg(clap::Arg::new("config_id").long("config-id").global(true));
        cmd = cmd.subcommand(clap::Command::new("action"));

        let m = cmd
            .try_get_matches_from(["test", "--config-id", "abc-123", "action"])
            .expect("parse should succeed");

        let result = extract_context_selector_param(&ui, &m);
        assert_eq!(
            result,
            Some(("config_id".to_string(), "abc-123".to_string()))
        );
    }

    #[test]
    fn extract_context_selector_param_absent() {
        let ui = ExtensionUi::DataTable {
            columns: vec![],
            data_action: "list".to_string(),
            row_actions: vec![],
            primary_actions: vec![],
            context_selector: Some(Box::new(ContextSelectorDef::new(
                "config_id",
                "Config",
                ContextSelectorSource::PluginConfigs {
                    plugin_type: "test".to_string(),
                },
            ))),
            default_per_page: None,
        };

        let mut cmd = clap::Command::new("test")
            .arg(clap::Arg::new("config_id").long("config-id").global(true));
        cmd = cmd.subcommand(clap::Command::new("action"));

        let m = cmd
            .try_get_matches_from(["test", "action"])
            .expect("parse should succeed");

        // No --config-id supplied → None.
        let result = extract_context_selector_param(&ui, &m);
        assert_eq!(result, None);
    }
}
