use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::Result;
use crate::output::HumanOutput;

use clap::Subcommand;
use rootcause::prelude::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::PathBuf;
use uptrakit_crypto::ecies::sealed_box_encrypt_base64;
use uptrakit_openapi_client::types::surfaces::{
    InvokeSurfaceInteractionRequest, SurfaceProviderInfo, SurfaceReadResponse, SurfaceResponse,
};
use uptrakit_wire::surfaces::{
    FormFieldDescriptor, FormSelectSource, InteractionDescriptor, InteractionHttpMethod,
    InteractionKind, InteractionTransport, ParamFieldDescriptor, ProviderEncryptionAlgorithm,
    ProviderEncryptionMetadata, SchemaContract, Targeting,
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
        /// HTTP method to select when the interaction ID is registered
        /// under multiple methods (e.g. `get`, `post`, `put`, `delete`).
        #[arg(long)]
        method: Option<String>,
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
            method,
            target_provider_id,
            timeout_seconds,
        } => {
            let params = parse_params(&params)?;
            let method = parse_http_method(method.as_deref())?;
            let resp = invoke(InvokeParams {
                surface_id,
                interaction_id,
                params,
                method,
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
    /// An interaction with no `form_ui` but opt-in `params` declarations
    /// (`ParamFieldDescriptor`s) -- rendered as typed clap args derived from
    /// each field's `SchemaContract`, distinct from the `form_ui`-keyed
    /// `Typed` variant above.
    DeclaredParams {
        about: String,
        params: Vec<&'a ParamFieldDescriptor>,
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
    let label = interaction.label.as_str();

    if interaction.kind == InteractionKind::Workflow {
        return InteractionRenderMode::RawOnly {
            about: format!(
                "{label} (raw JSON only: workflow interactions are not rendered dynamically)"
            ),
        };
    }

    let Some(form_ui) = interaction.form_ui.as_ref() else {
        if !interaction.params.is_empty() {
            return InteractionRenderMode::DeclaredParams {
                about: label.to_string(),
                params: interaction.params.iter().collect(),
            };
        }
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
    /// Disambiguates `interaction_id` when the surface registers it under
    /// more than one HTTP method (see `select_interaction`). `None` is the
    /// zero-UX-change path for single-method IDs.
    pub method: Option<InteractionHttpMethod>,
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
    let interaction = select_interaction(&surface, &params.interaction_id, params.method)?;
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
    let result = dispatch_interaction(
        &client,
        &params.surface_id,
        &params.interaction_id,
        interaction,
        request,
    )
    .await?;
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

    let Some(first_arg) = args.first() else {
        return Err(report!(CliError::Other(
            "surface ID is required (e.g. `surfaces ssh-agent.hosts list-hosts`)".to_string()
        )));
    };

    let surface_id = first_arg
        .to_str()
        .ok_or_else(|| {
            report!(CliError::Other(
                "surface ID must be valid UTF-8".to_string()
            ))
        })?
        .to_string();

    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let surface = client.read_surface(&surface_id).await.context_to()?;

    // Pre-resolve (not arg-union, see Task 1b): clap subcommands must be
    // built *before* parsing, but a multi-method interaction ID's
    // subcommand shape depends on which method the caller asked for. Scan
    // the raw args once, before clap ever sees them, for the target
    // subcommand name and an explicit `--method` token.
    let (target_interaction, method_token) = prescan_target_and_method(&args);
    let prescanned_method = parse_http_method(method_token.as_deref())?;
    let resolved_target = target_interaction
        .as_deref()
        .zip(prescanned_method.as_ref())
        .map(|(id, method)| (id.to_string(), method.clone()));

    let cmd = build_surface_command(
        &surface,
        resolved_target
            .as_ref()
            .map(|(id, method)| (id.as_str(), method)),
    );

    let matches = match cmd.try_get_matches_from(
        std::iter::once(OsString::from("surfaces")).chain(args.into_iter().skip(1)),
    ) {
        Ok(matches) => matches,
        Err(err) => err.exit(),
    };

    let target_provider_id = if surface.descriptor.targeting
        == uptrakit_wire::surfaces::Targeting::Targeted
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

    // The subcommand's own parsed `--method` (only present when the built
    // shape declared one -- see `build_interaction_subcommand`) is the
    // source of truth for dispatch method; the pre-scan above only shaped
    // which args clap would accept.
    let selected_method = parse_http_method(
        interaction_matches
            .try_get_one::<String>("method")
            .ok()
            .flatten()
            .map(String::as_str),
    )?;

    let interaction = select_interaction(&surface, interaction_id, selected_method)?;

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

    let result =
        dispatch_interaction(&client, &surface_id, interaction_id, interaction, request).await?;
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

/// Selects the concrete [`InteractionDescriptor`] a caller means by
/// `interaction_id`, disambiguating by `method` when the surface registers
/// that ID under more than one HTTP method (see ADR-0030's `(id, method)`
/// registry uniqueness key).
///
/// - Zero matches: the existing "not part of surface" error (unchanged
///   behavior).
/// - Exactly one match: used unconditionally, regardless of `method` --
///   zero UX change for single-method IDs.
/// - More than one match: `method` must be `Some` and match exactly one
///   candidate's [`InteractionDescriptor::effective_http_method`]; otherwise
///   a typed error listing the available methods.
fn select_interaction<'a>(
    surface: &'a SurfaceReadResponse,
    interaction_id: &str,
    method: Option<InteractionHttpMethod>,
) -> Result<&'a InteractionDescriptor> {
    use crate::error::CliError;

    let candidates: Vec<&InteractionDescriptor> = surface
        .interactions
        .iter()
        .filter(|candidate| candidate.interaction_id.as_str() == interaction_id)
        .collect();

    match candidates.as_slice() {
        [] => Err(report!(CliError::Other(format!(
            "interaction '{interaction_id}' is not part of surface '{}'",
            surface.descriptor.surface_id
        )))),
        [only] => Ok(*only),
        multiple => match method {
            Some(method) => multiple
                .iter()
                .find(|candidate| candidate.effective_http_method() == method)
                .copied()
                .ok_or_else(|| {
                    report!(CliError::Other(format!(
                        "interaction '{interaction_id}' has no variant registered under method '{method}'"
                    )))
                }),
            None => {
                let methods = multiple
                    .iter()
                    .map(|candidate| candidate.effective_http_method().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(report!(CliError::Other(format!(
                    "interaction '{interaction_id}' is registered under multiple methods ({methods}); pass --method"
                ))))
            }
        },
    }
}

/// Parses a raw `--method` value using [`InteractionHttpMethod`]'s strict
/// `FromStr` (generated by `wire_safe_enum!`). `None` in, `None` out --
/// this is the zero-UX-change path when the caller didn't pass `--method`.
fn parse_http_method(raw: Option<&str>) -> Result<Option<InteractionHttpMethod>> {
    use crate::error::CliError;

    raw.map(|value| {
        value.parse::<InteractionHttpMethod>().map_err(|_err| {
            report!(CliError::Other(format!(
                "invalid --method value '{value}'; expected one of: get, post, put, delete"
            )))
        })
    })
    .transpose()
}

/// Scans the raw `Dynamic` subcommand args -- *before* clap parses them --
/// for the target interaction subcommand name (the first non-flag token)
/// and an explicit `--method`/`--method=value` token anywhere after it.
///
/// This is a lightweight pre-resolve step only: it decides which shape to
/// build the target subcommand in (see `build_interaction_subcommand`). The
/// authoritative dispatch method is re-derived *after* clap parses, by
/// reading the `"method"` arg back out of the matched subcommand.
fn prescan_target_and_method(args: &[OsString]) -> (Option<String>, Option<String>) {
    let rest: Vec<&OsString> = args.iter().skip(1).collect();

    let mut target = None;
    let mut index = 0;
    while let Some(current) = rest.get(index) {
        if let Some(value) = current.to_str()
            && !value.starts_with('-')
        {
            target = Some(value.to_string());
            index += 1;
            break;
        }
        index += 1;
    }

    let mut method = None;
    while let Some(current) = rest.get(index) {
        if let Some(value) = current.to_str() {
            if let Some(inline) = value.strip_prefix("--method=") {
                method = Some(inline.to_string());
                break;
            }
            if value == "--method" {
                if let Some(next) = rest.get(index + 1).and_then(|arg| arg.to_str()) {
                    method = Some(next.to_string());
                }
                break;
            }
        }
        index += 1;
    }

    (target, method)
}

/// Groups `surface.interactions` by `interaction_id`, preserving first-seen
/// order, so `build_surface_command` builds exactly one clap subcommand per
/// unique ID (never one per descriptor -- that produced duplicate
/// subcommand names for multi-method IDs).
fn group_interactions_by_id(
    surface: &SurfaceReadResponse,
) -> Vec<(String, Vec<&InteractionDescriptor>)> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<&InteractionDescriptor>> = HashMap::new();

    for interaction in &surface.interactions {
        let id = interaction.interaction_id.to_string();
        if !groups.contains_key(&id) {
            order.push(id.clone());
        }
        groups.entry(id).or_default().push(interaction);
    }

    order
        .into_iter()
        .map(|id| {
            let descriptors = groups.remove(&id).unwrap_or_default();
            (id, descriptors)
        })
        .collect()
}

fn build_surface_command(
    surface: &SurfaceReadResponse,
    resolved: Option<(&str, &InteractionHttpMethod)>,
) -> clap::Command {
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

    for (interaction_id, descriptors) in group_interactions_by_id(surface) {
        cmd = cmd.subcommand(build_interaction_subcommand(
            &interaction_id,
            &descriptors,
            resolved,
        ));
    }

    cmd
}

/// Builds the single clap subcommand for one interaction ID.
///
/// - Single-method ID: today's shape, built from its sole descriptor.
/// - Multi-method ID with a resolved match (`resolved` names this ID and
///   its `effective_http_method` matches one of `descriptors`): the
///   resolved descriptor's shape, plus a `--method` arg so clap accepts the
///   already-known token.
/// - Multi-method ID without a resolution: a fallback shape with only a
///   required `--method` plus raw `--params`, so parsing yields a clear
///   missing-`--method` error and `--help` documents it.
fn build_interaction_subcommand(
    interaction_id: &str,
    descriptors: &[&InteractionDescriptor],
    resolved: Option<(&str, &InteractionHttpMethod)>,
) -> clap::Command {
    match descriptors {
        [only] => build_subcommand_for_descriptor(interaction_id, only, false),
        multiple => {
            if let Some((target_id, target_method)) = resolved
                && target_id == interaction_id
                && let Some(descriptor) = multiple
                    .iter()
                    .find(|candidate| candidate.effective_http_method() == *target_method)
            {
                return build_subcommand_for_descriptor(interaction_id, descriptor, true);
            }

            let methods = multiple
                .iter()
                .map(|candidate| candidate.effective_http_method().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            clap::Command::new(interaction_id.to_string())
                .about(format!(
                    "registered under multiple methods ({methods}); pass --method"
                ))
                .arg(raw_params_arg())
                .arg(
                    clap::Arg::new("method")
                        .long("method")
                        .required(true)
                        .help(format!("HTTP method to dispatch (one of: {methods})")),
                )
        }
    }
}

/// Builds a clap subcommand from a single resolved [`InteractionDescriptor`],
/// optionally adding a non-required `--method` arg (multi-method IDs need
/// clap to accept the already-pre-resolved `--method` token).
fn build_subcommand_for_descriptor(
    interaction_id: &str,
    interaction: &InteractionDescriptor,
    with_method_arg: bool,
) -> clap::Command {
    let render_mode = render_mode_for_interaction(interaction);
    let mut subcmd = clap::Command::new(interaction_id.to_string())
        .about(render_mode_about(&render_mode))
        .arg(raw_params_arg());

    if with_method_arg {
        subcmd = subcmd.arg(
            clap::Arg::new("method")
                .long("method")
                .help("HTTP method for this interaction (already resolved)"),
        );
    }

    match &render_mode {
        InteractionRenderMode::Typed { fields, .. } => {
            for field in fields {
                subcmd = subcmd.arg(build_field_arg(field));
            }
        }
        InteractionRenderMode::DeclaredParams { params, .. } => {
            for field in params {
                subcmd = subcmd.arg(build_param_arg(field));
            }
        }
        InteractionRenderMode::RawOnly { .. } => {}
    }

    subcmd
}

fn render_mode_about(render_mode: &InteractionRenderMode<'_>) -> String {
    match render_mode {
        InteractionRenderMode::Typed { about, .. }
        | InteractionRenderMode::DeclaredParams { about, .. }
        | InteractionRenderMode::RawOnly { about, .. } => about.clone(),
    }
}

fn raw_params_arg() -> clap::Arg {
    clap::Arg::new("params")
        .long("params")
        .help("Raw JSON params merged into the request body")
        .default_value("{}")
}

#[expect(
    clippy::expect_used,
    reason = "caller contract: only called for fields where typed_field_kind returns Some"
)]
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

/// Builds a clap arg for a declared `params` field (opt-in per-field
/// declaration, distinct from `form_ui` fields). Only `Integer`/`Number`/
/// `Boolean`/`String` are exercised by `DataLoad` admission (scalar-only,
/// see `validate_interaction_params`); other schema kinds fall back to a
/// plain string arg for convenience on non-`DataLoad` interactions.
fn build_param_arg(field: &ParamFieldDescriptor) -> clap::Arg {
    let arg = clap::Arg::new(field.key.clone())
        .long(field.key.clone())
        .required(field.required);

    match field.schema {
        SchemaContract::Integer => arg.value_parser(clap::value_parser!(i64)),
        SchemaContract::Number => arg.value_parser(clap::value_parser!(f64)),
        SchemaContract::Boolean => arg.value_parser(clap::value_parser!(bool)),
        SchemaContract::String
        | SchemaContract::Any
        | SchemaContract::Object
        | SchemaContract::Array
        | SchemaContract::Null => arg.value_parser(clap::value_parser!(String)),
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

    match render_mode {
        InteractionRenderMode::Typed { fields, .. } => {
            if interaction.form_ui.is_some() {
                for &field in fields {
                    if let Some(value) = extract_field_value(field, matches)? {
                        params.insert(field.key.clone(), value);
                    }
                }
            }
        }
        InteractionRenderMode::DeclaredParams {
            params: declared_params,
            ..
        } => {
            for &field in declared_params {
                if let Some(value) = extract_param_value(field, matches)? {
                    params.insert(field.key.clone(), value);
                }
            }
        }
        InteractionRenderMode::RawOnly { .. } => {}
    }

    Ok(params)
}

/// Extracts a typed value for a declared `params` field, mirroring
/// [`extract_field_value`] but sourced from `build_param_arg`'s
/// `SchemaContract`-derived value parsers rather than `FormFieldDescriptor`.
fn extract_param_value(
    field: &ParamFieldDescriptor,
    matches: &clap::ArgMatches,
) -> Result<Option<serde_json::Value>> {
    use crate::error::CliError;

    match field.schema {
        SchemaContract::Integer => Ok(matches
            .get_one::<i64>(field.key.as_str())
            .map(|value| serde_json::Value::Number((*value).into()))),
        SchemaContract::Number => {
            let Some(value) = matches.get_one::<f64>(field.key.as_str()) else {
                return Ok(None);
            };
            let number = serde_json::Number::from_f64(*value).ok_or_else(|| {
                report!(CliError::Other(format!(
                    "invalid number for --{}",
                    field.key
                )))
            })?;
            Ok(Some(serde_json::Value::Number(number)))
        }
        SchemaContract::Boolean => Ok(matches
            .get_one::<bool>(field.key.as_str())
            .map(|value| serde_json::Value::Bool(*value))),
        SchemaContract::String
        | SchemaContract::Any
        | SchemaContract::Object
        | SchemaContract::Array
        | SchemaContract::Null => Ok(matches
            .get_one::<String>(field.key.as_str())
            .map(|value| serde_json::Value::String(value.clone()))),
    }
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

/// Method-only dispatch decision for a surface interaction, derived from
/// [`InteractionDescriptor::effective_http_method`]. Kept separate from the
/// wire-level `InteractionHttpMethod` (which carries a forward-compat
/// `Other(String)` variant) so the CLI has one closed set of dispatch
/// outcomes to test against and fold unknown/forward-compat methods into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispatchVerb {
    Get,
    Put,
    Delete,
    Post,
}

fn dispatch_verb_for(interaction: &InteractionDescriptor) -> DispatchVerb {
    match interaction.effective_http_method() {
        InteractionHttpMethod::Get => DispatchVerb::Get,
        InteractionHttpMethod::Put => DispatchVerb::Put,
        InteractionHttpMethod::Delete => DispatchVerb::Delete,
        // Post and any forward-compat `Other(_)` method both invoke.
        _ => DispatchVerb::Post,
    }
}

/// Renders a scalar JSON value as a query-string value: strings pass through
/// unquoted, other scalars (and, as a fallback, non-scalars) use `Value`'s
/// own `Display` (`to_string`).
fn query_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Flattens a built request into `GET` query pairs. Mirrors the envelope the
/// `read_surface_interaction` handler splits back out server-side
/// (`target_provider_id` / `timeout_seconds` are envelope keys alongside the
/// provider `params`, see `split_get_envelope` in
/// `crates/ui/web-api/src/routes/surfaces.rs`).
fn build_query_pairs(request: &InvokeSurfaceInteractionRequest) -> Vec<(&str, String)> {
    let mut pairs: Vec<(&str, String)> = request
        .params
        .iter()
        .map(|(key, value)| (key.as_str(), query_value(value)))
        .collect();
    if let Some(target_provider_id) = &request.target_provider_id {
        pairs.push(("target_provider_id", target_provider_id.clone()));
    }
    if let Some(timeout_seconds) = request.timeout_seconds {
        pairs.push(("timeout_seconds", timeout_seconds.to_string()));
    }
    pairs
}

/// Dispatches a built request to the client method matching the
/// interaction's declared HTTP method (`DataLoad` interactions always
/// normalize to `GET` via `effective_http_method`).
async fn dispatch_interaction(
    client: &uptrakit_openapi_client::UptrakitClient,
    surface_id: &str,
    interaction_id: &str,
    interaction: &InteractionDescriptor,
    request: InvokeSurfaceInteractionRequest,
) -> Result<serde_json::Value> {
    match dispatch_verb_for(interaction) {
        DispatchVerb::Get => {
            let query = build_query_pairs(&request);
            client
                .read_surface_interaction(surface_id, interaction_id, &query)
                .await
                .context_to()
        }
        DispatchVerb::Put => client
            .update_surface_interaction(surface_id, interaction_id, &request)
            .await
            .context_to(),
        DispatchVerb::Delete => client
            .delete_surface_interaction(surface_id, interaction_id, &request)
            .await
            .context_to(),
        DispatchVerb::Post => client
            .invoke_surface_interaction(surface_id, interaction_id, &request)
            .await
            .context_to(),
    }
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
) -> Result<uptrakit_wire::surfaces::EncryptedSensitiveParams> {
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

    Ok(uptrakit_wire::surfaces::EncryptedSensitiveParams {
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
            descriptor: uptrakit_wire::surfaces::SurfaceDescriptor::builder()
                .surface_id("surface.sample".parse().unwrap())
                .label("Sample surface")
                .priority(200)
                .slot(uptrakit_wire::surfaces::SLOT_SETTINGS_TABS)
                .scope(uptrakit_wire::surfaces::Scope::Tenant)
                .targeting(Targeting::Universal)
                .provider_kind(uptrakit_wire::surfaces::ProviderKind::Plugin)
                .required_capabilities(uptrakit_wire::surfaces::CapabilitySet::default())
                .root_node(uptrakit_wire::surfaces::SurfaceNode::section(
                    Some("Sample surface".to_string()),
                    vec![],
                ))
                .build(),
            interactions: vec![{
                let mut i = InteractionDescriptor::new(
                    "surface.sample.submit".parse().unwrap(),
                    InteractionKind::FormSubmit,
                    "Submit",
                    InteractionTransport::ControllerLocal,
                );
                i.input_schema = Some(uptrakit_wire::surfaces::SchemaContract::Object);
                i.result_schema = Some(uptrakit_wire::surfaces::SchemaContract::Any);
                i.timeout_seconds = Some(30);
                i.form_ui = Some(uptrakit_wire::surfaces::FormUiDescriptor {
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
                });
                i
            }],
            data_sources: vec![],
        };

        let cmd = build_surface_command(&surface, None);
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

    #[test]
    fn query_value_strips_quotes_from_strings_but_not_other_scalars() {
        assert_eq!(query_value(&serde_json::json!("hello")), "hello");
        assert_eq!(query_value(&serde_json::json!(42)), "42");
        assert_eq!(query_value(&serde_json::json!(3.5)), "3.5");
        assert_eq!(query_value(&serde_json::json!(true)), "true");
        assert_eq!(query_value(&serde_json::json!(null)), "null");
    }

    #[test]
    fn data_load_descriptor_dispatches_get() {
        let interaction = InteractionDescriptor::new(
            "surface.sample.list".parse().unwrap(),
            InteractionKind::DataLoad,
            "List",
            InteractionTransport::ControllerLocal,
        );

        // DataLoad interactions normalize to GET regardless of the raw
        // (default-POST) `http_method` field -- see `effective_http_method`.
        assert_eq!(interaction.http_method, InteractionHttpMethod::Post);
        assert_eq!(dispatch_verb_for(&interaction), DispatchVerb::Get);
    }

    #[test]
    fn put_declared_descriptor_dispatches_put() {
        let mut interaction = InteractionDescriptor::new(
            "surface.sample.update".parse().unwrap(),
            InteractionKind::MutationAction,
            "Update",
            InteractionTransport::ControllerLocal,
        );
        interaction.http_method = InteractionHttpMethod::Put;

        assert_eq!(dispatch_verb_for(&interaction), DispatchVerb::Put);
    }

    /// Factored from the descriptor literal above; shared by the
    /// method-disambiguation fixtures below.
    fn sample_surface_descriptor() -> uptrakit_wire::surfaces::SurfaceDescriptor {
        uptrakit_wire::surfaces::SurfaceDescriptor::builder()
            .surface_id("surface.sample".parse().unwrap())
            .label("Sample surface")
            .priority(200)
            .slot(uptrakit_wire::surfaces::SLOT_SETTINGS_TABS)
            .scope(uptrakit_wire::surfaces::Scope::Tenant)
            .targeting(Targeting::Universal)
            .provider_kind(uptrakit_wire::surfaces::ProviderKind::Plugin)
            .required_capabilities(uptrakit_wire::surfaces::CapabilitySet::default())
            .root_node(uptrakit_wire::surfaces::SurfaceNode::section(
                Some("Sample surface".to_string()),
                vec![],
            ))
            .build()
    }

    fn sample_surface(interactions: Vec<InteractionDescriptor>) -> SurfaceReadResponse {
        SurfaceReadResponse {
            descriptor: sample_surface_descriptor(),
            interactions,
            data_sources: vec![],
        }
    }

    /// A `channels`-shaped fixture: GET `DataLoad` + POST `FormSubmit` (with
    /// a required `name` field) + PUT `MutationAction` + DELETE
    /// `MutationAction`, all sharing `interaction_id: "channels"` -- the
    /// first real multi-method interaction (Task 2).
    fn build_channels_surface() -> SurfaceReadResponse {
        let get_descriptor = InteractionDescriptor::new(
            "channels".parse().unwrap(),
            InteractionKind::DataLoad,
            "List channels",
            InteractionTransport::ControllerLocal,
        );

        let mut post_descriptor = InteractionDescriptor::new(
            "channels".parse().unwrap(),
            InteractionKind::FormSubmit,
            "Create channel",
            InteractionTransport::ControllerLocal,
        );
        post_descriptor.form_ui = Some(uptrakit_wire::surfaces::FormUiDescriptor {
            fields: vec![FormFieldDescriptor {
                key: "name".to_string(),
                label: "Name".to_string(),
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
            pre_load_interaction_id: None,
        });

        let put_descriptor = InteractionDescriptor::new(
            "channels".parse().unwrap(),
            InteractionKind::MutationAction,
            "Update channel",
            InteractionTransport::ControllerLocal,
        )
        .with_http_method(InteractionHttpMethod::Put);

        let delete_descriptor = InteractionDescriptor::new(
            "channels".parse().unwrap(),
            InteractionKind::MutationAction,
            "Delete channel",
            InteractionTransport::ControllerLocal,
        )
        .with_http_method(InteractionHttpMethod::Delete);

        sample_surface(vec![
            get_descriptor,
            post_descriptor,
            put_descriptor,
            delete_descriptor,
        ])
    }

    #[test]
    fn select_interaction_multi_method_resolves_by_explicit_method() {
        let get = InteractionDescriptor::new(
            "widget".parse().unwrap(),
            InteractionKind::DataLoad,
            "Get widget",
            InteractionTransport::ControllerLocal,
        );
        let put = InteractionDescriptor::new(
            "widget".parse().unwrap(),
            InteractionKind::MutationAction,
            "Update widget",
            InteractionTransport::ControllerLocal,
        )
        .with_http_method(InteractionHttpMethod::Put);

        let surface = sample_surface(vec![get, put]);

        let resolved = select_interaction(&surface, "widget", Some(InteractionHttpMethod::Put))
            .expect("Some(Put) should resolve the PUT descriptor");

        assert_eq!(resolved.effective_http_method(), InteractionHttpMethod::Put);
    }

    #[test]
    fn select_interaction_multi_method_without_method_lists_choices() {
        let get = InteractionDescriptor::new(
            "widget".parse().unwrap(),
            InteractionKind::DataLoad,
            "Get widget",
            InteractionTransport::ControllerLocal,
        );
        let put = InteractionDescriptor::new(
            "widget".parse().unwrap(),
            InteractionKind::MutationAction,
            "Update widget",
            InteractionTransport::ControllerLocal,
        )
        .with_http_method(InteractionHttpMethod::Put);

        let surface = sample_surface(vec![get, put]);

        let err = select_interaction(&surface, "widget", None)
            .expect_err("None must fail when the ID is registered under multiple methods");

        match err.current_context() {
            crate::error::CliError::Other(message) => {
                assert!(
                    message.contains("multiple methods"),
                    "unexpected message: {message}"
                );
                assert!(message.contains("get"), "unexpected message: {message}");
                assert!(message.contains("put"), "unexpected message: {message}");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn select_interaction_single_method_ignores_missing_method() {
        let get = InteractionDescriptor::new(
            "widget".parse().unwrap(),
            InteractionKind::DataLoad,
            "Get widget",
            InteractionTransport::ControllerLocal,
        );
        let surface = sample_surface(vec![get]);

        // Regression guard: today's UX for single-method IDs is unaffected
        // by the new disambiguation -- `None` still resolves.
        let resolved = select_interaction(&surface, "widget", None)
            .expect("single-method ID resolves regardless of --method");
        assert_eq!(resolved.interaction_id.as_str(), "widget");
    }

    #[test]
    fn multi_method_channels_get_reaches_dataload_descriptor_without_post_fields() {
        let surface = build_channels_surface();
        let method = InteractionHttpMethod::Get;
        let cmd = build_surface_command(&surface, Some(("channels", &method)));

        let matches = cmd
            .try_get_matches_from(["surfaces", "channels", "--method", "get"])
            .expect("GET variant must parse without POST's required `name` field");

        let (interaction_id, interaction_matches) =
            matches.subcommand().expect("subcommand present");
        assert_eq!(interaction_id, "channels");

        let selected_method = parse_http_method(
            interaction_matches
                .try_get_one::<String>("method")
                .ok()
                .flatten()
                .map(String::as_str),
        )
        .expect("valid --method value");

        let resolved = select_interaction(&surface, interaction_id, selected_method)
            .expect("resolves to the GET descriptor");
        assert_eq!(resolved.kind, InteractionKind::DataLoad);
    }

    #[test]
    fn multi_method_channels_without_method_flag_requires_method() {
        let surface = build_channels_surface();
        let cmd = build_surface_command(&surface, None);

        let result = cmd.try_get_matches_from(["surfaces", "channels"]);
        assert!(
            result.is_err(),
            "multi-method ID without --method must fail to parse, not silently pick one"
        );
    }

    #[test]
    fn build_surface_command_has_no_duplicate_subcommand_names() {
        let surface = build_channels_surface();
        let cmd = build_surface_command(&surface, None);

        let names: Vec<&str> = cmd.get_subcommands().map(clap::Command::get_name).collect();
        let unique: HashSet<&str> = names.iter().copied().collect();
        assert_eq!(
            names.len(),
            unique.len(),
            "duplicate subcommand names: {names:?}"
        );
    }
}
