use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::Result;
use crate::output::HumanOutput;

use clap::Subcommand;
use rootcause::prelude::*;
use serde::Serialize;
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;
use uptrakit_crypto::ecies::sealed_box_encrypt_base64;
use uptrakit_internal_wire::surfaces::{
    FormFieldDescriptor, FormSelectSource, InteractionDescriptor, InteractionKind,
    InteractionTransport, ProviderEncryptionAlgorithm, ProviderEncryptionMetadata, Targeting,
};
use uptrakit_openapi_client::types::surfaces::{
    InvokeSurfaceInteractionRequest, SurfaceProviderInfo, SurfaceReadResponse, SurfaceResponse,
};

#[derive(Debug, Subcommand)]
pub enum SurfacesCommands {
    /// List all registered surfaces.
    List {
        /// Filter by slot.
        #[arg(long)]
        slot: Option<String>,
        /// Filter by page.
        #[arg(long)]
        page: Option<String>,
    },
    /// List targeted providers for a surface.
    Providers {
        /// Surface ID.
        surface_id: String,
    },
    /// Read a surface descriptor and its interactions/data sources.
    Read {
        /// Surface ID.
        surface_id: String,
    },
    /// Invoke a surface interaction with raw JSON params.
    Invoke {
        /// Surface ID.
        surface_id: String,
        /// Interaction ID to invoke.
        interaction_id: String,
        /// Raw JSON params.
        #[arg(long, default_value = "{}")]
        params: String,
        /// Target provider ID for targeted surfaces.
        #[arg(long)]
        target_provider_id: Option<String>,
        /// Timeout override for the invocation.
        #[arg(long)]
        timeout_seconds: Option<u16>,
    },
    /// Dynamic surface invocation driven by `read_surface`.
    #[command(external_subcommand)]
    Dynamic(Vec<OsString>),
}

pub async fn dispatch(command: SurfacesCommands, ctx: &CliContext) -> Result<()> {
    match command {
        SurfacesCommands::List { slot, page } => {
            let resp = list(ListParams {
                slot,
                page,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        SurfacesCommands::Providers { surface_id } => {
            let resp = providers(ProvidersParams {
                surface_id,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        SurfacesCommands::Read { surface_id } => {
            let resp = read(ReadParams {
                surface_id,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        SurfacesCommands::Invoke {
            surface_id,
            interaction_id,
            params,
            target_provider_id,
            timeout_seconds,
        } => {
            let params = parse_params(&params)?;
            let resp = invoke(InvokeParams {
                surface_id,
                interaction_id,
                params,
                target_provider_id,
                timeout_seconds,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        SurfacesCommands::Dynamic(args) => {
            let resp = dynamic_invoke(
                args,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct InvokeOutput(pub serde_json::Value);

impl HumanOutput for InvokeOutput {
    fn to_human_string(&self) -> String {
        serde_json::to_string_pretty(&self.0).unwrap_or_else(|_| self.0.to_string()) + "\n"
    }
}

#[derive(Debug, Clone)]
enum InteractionRenderMode<'a> {
    Typed {
        about: String,
        fields: Vec<&'a FormFieldDescriptor>,
    },
    RawOnly {
        about: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypedFieldKind {
    Toggle,
    Number,
    Select,
    Text,
    SshPrivateKey,
}

fn render_mode_for_interaction(interaction: &InteractionDescriptor) -> InteractionRenderMode<'_> {
    let label = interaction
        .label
        .as_deref()
        .unwrap_or(interaction.interaction_id.as_str());

    if interaction.kind == InteractionKind::Workflow {
        return InteractionRenderMode::RawOnly {
            about: format!(
                "{label} (raw JSON only: workflow interactions are not rendered dynamically)"
            ),
        };
    }

    let Some(form_ui) = interaction.form_ui.as_ref() else {
        return InteractionRenderMode::Typed {
            about: label.to_string(),
            fields: Vec::new(),
        };
    };

    let mut fields = Vec::with_capacity(form_ui.fields.len());
    for field in &form_ui.fields {
        match typed_field_kind(field) {
            Ok(kind) => {
                if matches!(kind, TypedFieldKind::SshPrivateKey) && field.list {
                    return InteractionRenderMode::RawOnly {
                        about: format!(
                            "{label} (raw JSON only: field `{}` uses unsupported list mode)",
                            field.key
                        ),
                    };
                }
                if matches!(kind, TypedFieldKind::Toggle) && field.list {
                    return InteractionRenderMode::RawOnly {
                        about: format!(
                            "{label} (raw JSON only: field `{}` uses unsupported list mode)",
                            field.key
                        ),
                    };
                }
                fields.push(field);
            }
            Err(reason) => {
                return InteractionRenderMode::RawOnly {
                    about: format!("{label} (raw JSON only: {reason})"),
                };
            }
        }
    }

    InteractionRenderMode::Typed {
        about: label.to_string(),
        fields,
    }
}

fn typed_field_kind(field: &FormFieldDescriptor) -> std::result::Result<TypedFieldKind, String> {
    if matches!(
        field.select_source,
        Some(FormSelectSource::Action { .. }) | Some(FormSelectSource::RestApi { .. })
    ) {
        return Err(format!(
            "field `{}` uses `select_source` and cannot be rendered dynamically",
            field.key
        ));
    }

    if field.field_type == "hidden" {
        return Err(format!(
            "field `{}` uses unsupported field_type `hidden`",
            field.key
        ));
    }

    match field.field_type.as_str() {
        "toggle" => Ok(TypedFieldKind::Toggle),
        "number" => Ok(TypedFieldKind::Number),
        "select" => Ok(TypedFieldKind::Select),
        "text" | "textarea" | "password" => Ok(TypedFieldKind::Text),
        "ssh_private_key" => Ok(TypedFieldKind::SshPrivateKey),
        other => Err(format!(
            "field `{}` uses unsupported field_type `{other}`",
            field.key
        )),
    }
}

pub struct ListParams<'a> {
    pub slot: Option<String>,
    pub page: Option<String>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ProvidersParams<'a> {
    pub surface_id: String,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ReadParams<'a> {
    pub surface_id: String,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct InvokeParams<'a> {
    pub surface_id: String,
    pub interaction_id: String,
    pub params: serde_json::Map<String, serde_json::Value>,
    pub target_provider_id: Option<String>,
    pub timeout_seconds: Option<u16>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub async fn list(params: ListParams<'_>) -> Result<Vec<SurfaceResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .list_surfaces(params.slot.as_deref(), params.page.as_deref())
        .await
        .context_to()
}

pub async fn providers(params: ProvidersParams<'_>) -> Result<Vec<SurfaceProviderInfo>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .list_surface_providers(&params.surface_id)
        .await
        .context_to()
}

pub async fn read(params: ReadParams<'_>) -> Result<SurfaceReadResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.read_surface(&params.surface_id).await.context_to()
}

pub async fn invoke(params: InvokeParams<'_>) -> Result<InvokeOutput> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let surface = client.read_surface(&params.surface_id).await.context_to()?;
    let interaction = surface
        .interactions
        .iter()
        .find(|candidate| candidate.interaction_id.as_str() == params.interaction_id)
        .ok_or_else(|| {
            report!(crate::error::CliError::Other(format!(
                "interaction '{}' is not part of surface '{}'",
                params.interaction_id, params.surface_id
            )))
        })?;
    if surface.descriptor.targeting == Targeting::Targeted && params.target_provider_id.is_none() {
        return Err(report!(crate::error::CliError::Other(
            "targeted surfaces require --target-provider-id <PROVIDER_ID>".to_string()
        )));
    }
    let request = build_invoke_request(
        &client,
        &params.surface_id,
        interaction,
        params.params,
        params.target_provider_id,
        params.timeout_seconds,
    )
    .await?;
    let result = client
        .invoke_surface_interaction(&params.surface_id, &params.interaction_id, &request)
        .await
        .context_to()?;
    Ok(InvokeOutput(result))
}

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
            "surface ID is required (e.g. `surfaces ssh-agent.hosts list-hosts`)".to_string()
        )));
    }

    let surface_id = args[0]
        .to_str()
        .ok_or_else(|| {
            report!(CliError::Other(
                "surface ID must be valid UTF-8".to_string()
            ))
        })?
        .to_string();

    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let surface = client.read_surface(&surface_id).await.context_to()?;
    let cmd = build_surface_command(&surface);

    let matches = match cmd.try_get_matches_from(
        std::iter::once(OsString::from("surfaces")).chain(args.into_iter().skip(1)),
    ) {
        Ok(matches) => matches,
        Err(err) => err.exit(),
    };

    let target_provider_id = if surface.descriptor.targeting
        == uptrakit_internal_wire::surfaces::Targeting::Targeted
    {
        Some(
            matches
                .get_one::<String>("target-provider-id")
                .cloned()
                .ok_or_else(|| {
                    report!(CliError::Other(
                        "targeted surfaces require --target-provider-id <PROVIDER_ID>".to_string()
                    ))
                })?,
        )
    } else {
        None
    };

    let (interaction_id, interaction_matches) = matches.subcommand().ok_or_else(|| {
        report!(CliError::Other(
            "interaction is required; use --help to see the available surface interactions"
                .to_string()
        ))
    })?;

    let interaction = surface
        .interactions
        .iter()
        .find(|candidate| candidate.interaction_id.as_str() == interaction_id)
        .ok_or_else(|| {
            report!(CliError::Other(format!(
                "interaction '{interaction_id}' is not part of surface '{surface_id}'"
            )))
        })?;

    let render_mode = render_mode_for_interaction(interaction);
    let params = extract_invocation_params(interaction, &render_mode, interaction_matches)?;
    let timeout_seconds = interaction.timeout_seconds;
    let request = build_invoke_request(
        &client,
        &surface_id,
        interaction,
        params,
        target_provider_id,
        timeout_seconds,
    )
    .await?;

    let result = client
        .invoke_surface_interaction(&surface_id, interaction_id, &request)
        .await
        .context_to()?;
    Ok(InvokeOutput(result))
}

impl HumanOutput for Vec<SurfaceResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No surfaces registered.\n".to_string();
        }

        let mut out = String::new();
        out.push_str(&format!(
            "{:<42} {:<28} {:<18} {:<12} {:<8}\n",
            "ID", "Label", "Slot", "Targeting", "Providers"
        ));
        out.push_str(&format!("{}\n", "-".repeat(112)));
        for surface in self {
            out.push_str(&format!(
                "{:<42} {:<28} {:<18} {:<12} {:<8}\n",
                surface.descriptor.surface_id,
                surface.descriptor.label,
                surface.descriptor.slot,
                format!("{:?}", surface.descriptor.targeting),
                surface.provider_count
            ));
        }
        out
    }
}

impl HumanOutput for Vec<SurfaceProviderInfo> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No providers connected.\n".to_string();
        }

        let mut out = String::new();
        out.push_str(&format!(
            "{:<38} {:<30} {:<18} {}\n",
            "Provider ID", "Label", "Availability", "Service ID"
        ));
        out.push_str(&format!("{}\n", "-".repeat(100)));
        for provider in self {
            out.push_str(&format!(
                "{:<38} {:<30} {:<18} {}\n",
                provider.provider_id,
                provider.display_label,
                format!("{:?}", provider.availability),
                provider
                    .service_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "-".to_string())
            ));
        }
        out
    }
}

impl HumanOutput for SurfaceReadResponse {
    fn to_human_string(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string()) + "\n"
    }
}

fn build_surface_command(surface: &SurfaceReadResponse) -> clap::Command {
    let mut cmd = clap::Command::new(surface.descriptor.surface_id.to_string())
        .about(surface.descriptor.label.clone())
        .subcommand_required(true);

    if surface.descriptor.targeting == Targeting::Targeted {
        cmd = cmd.arg(
            clap::Arg::new("target-provider-id")
                .long("target-provider-id")
                .help("Provider ID for targeted surfaces")
                .global(true),
        );
    }

    for interaction in &surface.interactions {
        let render_mode = render_mode_for_interaction(interaction);
        let mut subcmd = clap::Command::new(interaction.interaction_id.to_string())
            .about(render_mode_about(&render_mode))
            .arg(raw_params_arg());

        if let InteractionRenderMode::Typed { fields, .. } = render_mode {
            for field in fields {
                subcmd = subcmd.arg(build_field_arg(field));
            }
        }

        cmd = cmd.subcommand(subcmd);
    }

    cmd
}

fn render_mode_about(render_mode: &InteractionRenderMode<'_>) -> String {
    match render_mode {
        InteractionRenderMode::Typed { about, .. }
        | InteractionRenderMode::RawOnly { about, .. } => about.clone(),
    }
}

fn raw_params_arg() -> clap::Arg {
    clap::Arg::new("params")
        .long("params")
        .help("Raw JSON params merged into the request body")
        .default_value("{}")
}

fn build_field_arg(field: &FormFieldDescriptor) -> clap::Arg {
    let mut arg = clap::Arg::new(field.key.clone()).long(field.key.clone());

    if field.required {
        arg = arg.required(true);
    }

    if let Some(default_value) = &field.default_value {
        arg = arg.default_value(default_value.clone());
    }

    if let Some(help_text) = &field.help_text {
        arg = arg.help(help_text.clone());
    }

    if let Some(placeholder) = &field.placeholder {
        let help = arg
            .get_help()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let help = if help.is_empty() {
            format!("e.g. {placeholder}")
        } else {
            format!("{help} (e.g. {placeholder})")
        };
        arg = arg.help(help);
    }

    match typed_field_kind(field).expect("build_field_arg only called for supported fields") {
        TypedFieldKind::Toggle => arg.action(clap::ArgAction::SetTrue),
        TypedFieldKind::Number => arg.value_parser(clap::value_parser!(String)),
        TypedFieldKind::Select => {
            if !field.options.is_empty() && !field.list {
                let values = field
                    .options
                    .iter()
                    .map(|option| clap::builder::PossibleValue::new(option.value.clone()))
                    .collect::<Vec<_>>();
                arg.value_parser(clap::builder::PossibleValuesParser::new(values))
            } else {
                arg.value_parser(clap::value_parser!(String))
            }
        }
        TypedFieldKind::Text => arg.value_parser(clap::value_parser!(String)),
        TypedFieldKind::SshPrivateKey => arg.value_parser(clap::value_parser!(PathBuf)),
    }
}

fn extract_invocation_params(
    interaction: &InteractionDescriptor,
    render_mode: &InteractionRenderMode<'_>,
    matches: &clap::ArgMatches,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    let mut params = parse_params(
        matches
            .get_one::<String>("params")
            .map(String::as_str)
            .unwrap_or("{}"),
    )?;

    let InteractionRenderMode::Typed { fields, .. } = render_mode else {
        return Ok(params);
    };

    if interaction.form_ui.is_some() {
        for &field in fields {
            let value = extract_field_value(field, matches)?;
            if let Some(value) = value {
                params.insert(field.key.clone(), value);
            }
        }
    }

    Ok(params)
}

fn extract_field_value(
    field: &FormFieldDescriptor,
    matches: &clap::ArgMatches,
) -> Result<Option<serde_json::Value>> {
    use crate::error::CliError;

    match typed_field_kind(field).map_err(|reason| report!(CliError::Other(reason)))? {
        TypedFieldKind::Toggle => Ok(Some(serde_json::Value::Bool(
            matches.get_flag(field.key.as_str()),
        ))),
        TypedFieldKind::Number => {
            let Some(value) = matches.get_one::<String>(field.key.as_str()) else {
                return Ok(None);
            };
            if field.list {
                let mut values = Vec::new();
                for item in split_list_values(value) {
                    let value =
                        serde_json::Number::from_f64(item.parse::<f64>().map_err(|err| {
                            report!(CliError::Other(format!(
                                "invalid number for --{}: {err}",
                                field.key
                            )))
                        })?)
                        .ok_or_else(|| {
                            report!(CliError::Other(format!(
                                "invalid number for --{}",
                                field.key
                            )))
                        })?;
                    values.push(serde_json::Value::Number(value));
                }
                Ok(Some(serde_json::Value::Array(values)))
            } else {
                let value = serde_json::Number::from_f64(value.parse::<f64>().map_err(|err| {
                    report!(CliError::Other(format!(
                        "invalid number for --{}: {err}",
                        field.key
                    )))
                })?)
                .ok_or_else(|| {
                    report!(CliError::Other(format!(
                        "invalid number for --{}",
                        field.key
                    )))
                })?;
                Ok(Some(serde_json::Value::Number(value)))
            }
        }
        TypedFieldKind::Select => {
            let Some(raw) = matches.get_one::<String>(field.key.as_str()) else {
                return Ok(None);
            };
            if field.list {
                let values = split_list_values(raw)
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect::<Vec<_>>();
                Ok(Some(serde_json::Value::Array(values)))
            } else {
                Ok(Some(serde_json::Value::String(raw.clone())))
            }
        }
        TypedFieldKind::Text => {
            let Some(raw) = matches.get_one::<String>(field.key.as_str()) else {
                return Ok(None);
            };
            if field.list {
                let values = split_list_values(raw)
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect::<Vec<_>>();
                Ok(Some(serde_json::Value::Array(values)))
            } else {
                Ok(Some(serde_json::Value::String(raw.clone())))
            }
        }
        TypedFieldKind::SshPrivateKey => {
            let Some(path) = matches.get_one::<PathBuf>(field.key.as_str()) else {
                return Ok(None);
            };
            let contents = std::fs::read_to_string(path).map_err(|err| {
                report!(CliError::Other(format!(
                    "failed to read SSH private key file '{}': {err}",
                    path.display()
                )))
            })?;
            Ok(Some(serde_json::Value::String(contents)))
        }
    }
}

fn split_list_values(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect()
}

fn parse_params(params: &str) -> Result<serde_json::Map<String, serde_json::Value>> {
    use crate::error::CliError;

    let value: serde_json::Value = serde_json::from_str(params)
        .map_err(|err| report!(CliError::Other(format!("invalid JSON for --params: {err}"))))?;
    let Some(map) = value.as_object() else {
        return Err(report!(CliError::Other(
            "--params must be a JSON object".to_string()
        )));
    };
    Ok(map.clone())
}

async fn build_invoke_request(
    client: &uptrakit_openapi_client::UptrakitClient,
    surface_id: &str,
    interaction: &InteractionDescriptor,
    params: serde_json::Map<String, serde_json::Value>,
    target_provider_id: Option<String>,
    timeout_seconds: Option<u16>,
) -> Result<InvokeSurfaceInteractionRequest> {
    let (mut clear_params, sensitive_params) =
        partition_sensitive_params(params, &interaction.sensitive_fields);

    let encrypted_sensitive_params = if matches!(
        interaction.transport,
        InteractionTransport::ProviderProxied
    ) && !interaction.sensitive_fields.is_empty()
    {
        let provider_id = target_provider_id.as_deref().ok_or_else(|| {
            report!(crate::error::CliError::Other(
                "provider-proxied interactions with sensitive fields require --target-provider-id <PROVIDER_ID>".to_string(),
            ))
        })?;
        let provider = provider_encryption_metadata(client, surface_id, provider_id).await?;
        let encrypted = encrypt_sensitive_params(&provider, sensitive_params)?;
        Some(encrypted)
    } else {
        clear_params.extend(sensitive_params);
        None
    };

    Ok(InvokeSurfaceInteractionRequest {
        params: clear_params,
        encrypted_sensitive_params,
        target_provider_id,
        idempotency_key: None,
        timeout_seconds,
    })
}

fn partition_sensitive_params(
    params: serde_json::Map<String, serde_json::Value>,
    sensitive_fields: &[String],
) -> (
    serde_json::Map<String, serde_json::Value>,
    serde_json::Map<String, serde_json::Value>,
) {
    if sensitive_fields.is_empty() {
        return (params, serde_json::Map::new());
    }

    let sensitive_fields = sensitive_fields
        .iter()
        .map(|field| field.as_str())
        .collect::<HashSet<_>>();

    let mut clear_params = serde_json::Map::new();
    let mut sensitive_params = serde_json::Map::new();
    for (key, value) in params {
        if sensitive_fields.contains(key.as_str()) {
            sensitive_params.insert(key, value);
        } else {
            clear_params.insert(key, value);
        }
    }

    (clear_params, sensitive_params)
}

fn encrypt_sensitive_params(
    provider: &SurfaceProviderInfo,
    sensitive_params: serde_json::Map<String, serde_json::Value>,
) -> Result<uptrakit_internal_wire::surfaces::EncryptedSensitiveParams> {
    use crate::error::CliError;

    let metadata = provider.encryption_metadata.as_ref().ok_or_else(|| {
        report!(CliError::Other(format!(
            "provider `{}` does not advertise encryption metadata",
            provider.provider_id
        )))
    })?;

    let ProviderEncryptionMetadata {
        key_id,
        algorithm,
        public_key,
    } = metadata;

    if !matches!(algorithm, ProviderEncryptionAlgorithm::EciesP256) {
        return Err(report!(CliError::Other(format!(
            "provider `{}` advertises unsupported encryption algorithm {:?}",
            provider.provider_id, algorithm
        ))));
    }

    let plaintext = serde_json::to_string(&sensitive_params).map_err(|err| {
        report!(CliError::Other(format!(
            "failed to serialise sensitive params for encryption: {err}"
        )))
    })?;
    let ciphertext_b64 = sealed_box_encrypt_base64(&plaintext, public_key).map_err(|err| {
        report!(CliError::Other(format!(
            "failed to encrypt sensitive params for provider `{}`: {err}",
            provider.provider_id
        )))
    })?;

    Ok(uptrakit_internal_wire::surfaces::EncryptedSensitiveParams {
        key_id: key_id.clone(),
        algorithm: algorithm.clone(),
        ciphertext_b64,
    })
}

async fn provider_encryption_metadata(
    client: &uptrakit_openapi_client::UptrakitClient,
    surface_id: &str,
    provider_id: &str,
) -> Result<SurfaceProviderInfo> {
    use crate::error::CliError;

    let providers = client
        .list_surface_providers(surface_id)
        .await
        .context_to()?;
    providers
        .into_iter()
        .find(|provider| provider.provider_id == provider_id)
        .ok_or_else(|| {
            report!(CliError::Other(format!(
                "provider `{provider_id}` was not returned for surface `{surface_id}`"
            )))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_field_type_forces_raw_json_mode() {
        let surface = SurfaceReadResponse {
            descriptor: uptrakit_internal_wire::surfaces::SurfaceDescriptor {
                surface_id: "surface.sample".parse().unwrap(),
                label: "Sample surface".to_string(),
                priority: 200,
                slot: uptrakit_internal_wire::surfaces::SLOT_SETTINGS_TABS.to_string(),
                scope: uptrakit_internal_wire::surfaces::Scope::Tenant,
                targeting: Targeting::Universal,
                required_permission: None,
                provider_kind: uptrakit_internal_wire::surfaces::ProviderKind::Plugin,
                required_capabilities: uptrakit_internal_wire::surfaces::CapabilitySet::default(),
                root_node: uptrakit_internal_wire::surfaces::SurfaceNode::Section {
                    title: Some("Sample surface".to_string()),
                    children: vec![],
                },
            },
            interactions: vec![InteractionDescriptor {
                interaction_id: "surface.sample.submit".parse().unwrap(),
                kind: InteractionKind::FormSubmit,
                label: "Submit".to_string(),
                required_permission: None,
                input_schema: Some(uptrakit_internal_wire::surfaces::SchemaContract::Object),
                result_schema: Some(uptrakit_internal_wire::surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: Some(30),
                confirmation: None,
                transport: InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: Some(uptrakit_internal_wire::surfaces::FormUiDescriptor {
                    fields: vec![FormFieldDescriptor {
                        key: "mystery".to_string(),
                        label: "Mystery".to_string(),
                        field_type: "mystery_type".to_string(),
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
                    pre_load_interaction_id: None,
                }),
            }],
            data_sources: vec![],
        };

        let cmd = build_surface_command(&surface);
        let subcmd = cmd
            .get_subcommands()
            .find(|candidate| candidate.get_name() == "surface.sample.submit")
            .expect("subcommand");
        let about = subcmd
            .get_about()
            .map(|value| value.to_string())
            .unwrap_or_default();

        assert!(
            about.contains("raw JSON only"),
            "unexpected about text: {about}"
        );
        assert!(
            about.contains("unsupported field_type `mystery_type`"),
            "unexpected about text: {about}"
        );
        assert!(
            subcmd
                .get_arguments()
                .all(|arg| arg.get_id().as_str() != "mystery"),
            "unsupported field type should not produce a typed argument"
        );
    }
}
