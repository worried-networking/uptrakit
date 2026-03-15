use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::SecretString;
use uptrakit_openapi_client::types::settings_mqtt::{
    CreateMqttClientRequest, MqttClientResponse, MqttLimitResponse, UpdateMqttClientRequest,
    UpdateMqttLimitRequest,
};

use super::DeletedOutput;

#[derive(Debug, Subcommand)]
pub enum MqttCommands {
    /// List MQTT client configurations
    List,
    /// Show MQTT client configuration details
    Show {
        /// MQTT configuration UUID
        id: Uuid,
    },
    /// Create a new MQTT client configuration
    Create {
        /// MQTT URL (e.g. mqtt://broker:1883)
        #[arg(long)]
        url: Option<String>,
        /// Transport type (tcp, tls)
        #[arg(long)]
        transport: Option<String>,
        /// Broker hostname
        #[arg(long)]
        host: Option<String>,
        /// Broker port
        #[arg(long)]
        port: Option<u16>,
        /// Enable or disable
        #[arg(long)]
        enabled: Option<bool>,
        /// MQTT client ID
        #[arg(long)]
        client_id: Option<String>,
        /// MQTT username
        #[arg(long)]
        username: Option<String>,
        /// MQTT password
        #[arg(long)]
        password: Option<String>,
        /// Custom CA certificate in PEM format (for private brokers)
        #[arg(long, conflicts_with = "ca_pem_file")]
        ca_pem: Option<String>,
        /// Path to a PEM file containing a custom CA certificate (for private brokers)
        #[arg(long, conflicts_with = "ca_pem")]
        ca_pem_file: Option<std::path::PathBuf>,
        /// Topic prefix (e.g. uptrakit)
        #[arg(long)]
        topic_prefix: Option<String>,
        /// Enable Home Assistant MQTT discovery
        #[arg(long = "ha-discovery", action = clap::ArgAction::SetTrue, default_value_t = false)]
        ha_discovery: bool,
        /// Disable Home Assistant MQTT discovery
        #[arg(long = "no-ha-discovery", action = clap::ArgAction::SetTrue, default_value_t = false, conflicts_with = "ha_discovery")]
        no_ha_discovery: bool,
        /// Home Assistant discovery topic prefix (default: homeassistant)
        #[arg(long)]
        ha_discovery_prefix: Option<String>,
    },
    /// Update an MQTT client configuration
    Update {
        /// MQTT configuration UUID
        id: Uuid,
        /// MQTT URL
        #[arg(long)]
        url: Option<String>,
        /// Transport type (tcp, tls)
        #[arg(long)]
        transport: Option<String>,
        /// Broker hostname
        #[arg(long)]
        host: Option<String>,
        /// Broker port
        #[arg(long)]
        port: Option<u16>,
        /// Enable or disable
        #[arg(long)]
        enabled: Option<bool>,
        /// MQTT client ID
        #[arg(long)]
        client_id: Option<String>,
        /// MQTT username
        #[arg(long)]
        username: Option<String>,
        /// MQTT password
        #[arg(long)]
        password: Option<String>,
        /// Custom CA certificate in PEM format (for private brokers)
        #[arg(long, conflicts_with = "ca_pem_file")]
        ca_pem: Option<String>,
        /// Path to a PEM file containing a custom CA certificate (for private brokers)
        #[arg(long, conflicts_with = "ca_pem")]
        ca_pem_file: Option<std::path::PathBuf>,
        /// Topic prefix
        #[arg(long)]
        topic_prefix: Option<String>,
        /// Enable Home Assistant MQTT discovery
        #[arg(long = "ha-discovery", action = clap::ArgAction::SetTrue, default_value_t = false)]
        ha_discovery: bool,
        /// Disable Home Assistant MQTT discovery
        #[arg(long = "no-ha-discovery", action = clap::ArgAction::SetTrue, default_value_t = false, conflicts_with = "ha_discovery")]
        no_ha_discovery: bool,
        /// Home Assistant discovery topic prefix (default: homeassistant)
        #[arg(long)]
        ha_discovery_prefix: Option<String>,
    },
    /// Delete an MQTT client configuration
    Delete {
        /// MQTT configuration UUID
        id: Uuid,
    },
    /// MQTT client limit management
    Limit {
        #[command(subcommand)]
        command: MqttLimitCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum MqttLimitCommands {
    /// Show MQTT client limit
    Show,
    /// Update MQTT client limit
    Update {
        /// Maximum MQTT clients per tenant
        #[arg(long)]
        max: u16,
    },
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for creating an MQTT client configuration.
pub struct MqttCreateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub url: Option<String>,
    pub transport: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub enabled: Option<bool>,
    pub client_id: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub ca_pem: Option<String>,
    pub topic_prefix: Option<String>,
    pub ha_discovery: Option<bool>,
    pub ha_discovery_prefix: Option<String>,
}

/// Parameters for updating an MQTT client configuration.
pub struct MqttUpdateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub id: Uuid,
    pub url: Option<String>,
    pub transport: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub enabled: Option<bool>,
    pub client_id: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub ca_pem: Option<String>,
    pub topic_prefix: Option<String>,
    pub ha_discovery: Option<bool>,
    pub ha_discovery_prefix: Option<String>,
}

// ── Human output ─────────────────────────────────────────────────────────────

impl HumanOutput for Vec<MqttClientResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No MQTT configurations found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<8} {:<10} {:<25} {:<6} STATUS\n",
            "ID", "ENABLED", "TRANSPORT", "HOST", "PORT"
        );
        for m in self {
            out.push_str(&format!(
                "{:<38} {:<8} {:<10} {:<25} {:<6} {}\n",
                m.id, m.enabled, m.transport, m.host, m.port, m.connection_status
            ));
        }
        out
    }
}

impl HumanOutput for MqttClientResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:            {}\n", self.id));
        out.push_str(&format!("Enabled:       {}\n", self.enabled));
        out.push_str(&format!("Transport:     {}\n", self.transport));
        out.push_str(&format!("Host:          {}\n", self.host));
        out.push_str(&format!("Port:          {}\n", self.port));
        out.push_str(&format!("URL:           {}\n", self.url));
        out.push_str(&format!("Client ID:     {}\n", self.client_id));
        out.push_str(&format!(
            "Username:      {}\n",
            self.username.as_deref().unwrap_or("-")
        ));
        out.push_str(&format!("Has Password:  {}\n", self.has_password));
        out.push_str(&format!("Topic Prefix:  {}\n", self.topic_prefix));
        out.push_str(&format!("HA Discovery:  {}\n", self.ha_discovery));
        if self.ha_discovery {
            out.push_str(&format!("HA Prefix:     {}\n", self.ha_discovery_prefix));
        }
        out.push_str(&format!("Status:        {}\n", self.connection_status));
        out
    }
}

impl HumanOutput for MqttLimitResponse {
    fn to_human_string(&self) -> String {
        format!("Max Clients Per Tenant:  {}\n", self.max_clients_per_tenant)
    }
}

// ── Dispatch ─────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)] // Large dispatch is inherent to MQTT subcommands
pub async fn dispatch_mqtt(command: MqttCommands, ctx: &CliContext) -> Result<()> {
    match command {
        MqttCommands::List => {
            let resp = mqtt_list(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        MqttCommands::Show { id } => {
            let resp = mqtt_show(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        MqttCommands::Create {
            url,
            transport,
            host,
            port,
            enabled,
            client_id,
            username,
            password,
            ca_pem,
            ca_pem_file,
            topic_prefix,
            ha_discovery,
            no_ha_discovery,
            ha_discovery_prefix,
        } => {
            let ca_pem = crate::commands::resolve_ca_pem(ca_pem, ca_pem_file)?;
            let ha_discovery_flag = if ha_discovery {
                Some(true)
            } else if no_ha_discovery {
                Some(false)
            } else {
                None
            };
            let resp = mqtt_create(MqttCreateParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                url,
                transport,
                host,
                port,
                enabled,
                client_id,
                username,
                password,
                ca_pem,
                topic_prefix,
                ha_discovery: ha_discovery_flag,
                ha_discovery_prefix,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        MqttCommands::Update {
            id,
            url,
            transport,
            host,
            port,
            enabled,
            client_id,
            username,
            password,
            ca_pem,
            ca_pem_file,
            topic_prefix,
            ha_discovery,
            no_ha_discovery,
            ha_discovery_prefix,
        } => {
            let ca_pem = crate::commands::resolve_ca_pem(ca_pem, ca_pem_file)?;
            let ha_discovery_flag = if ha_discovery {
                Some(true)
            } else if no_ha_discovery {
                Some(false)
            } else {
                None
            };
            let resp = mqtt_update(MqttUpdateParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                id,
                url,
                transport,
                host,
                port,
                enabled,
                client_id,
                username,
                password,
                ca_pem,
                topic_prefix,
                ha_discovery: ha_discovery_flag,
                ha_discovery_prefix,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        MqttCommands::Delete { id } => {
            let resp = mqtt_delete(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        MqttCommands::Limit { command } => match command {
            MqttLimitCommands::Show => {
                let resp = mqtt_limit_show(
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
            MqttLimitCommands::Update { max } => {
                let resp = mqtt_limit_update(
                    max,
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
        },
    }
    Ok(())
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// List all MQTT client configurations.
pub async fn mqtt_list(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<Vec<MqttClientResponse>> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.list_mqtt_settings().await.context_to()
}

/// Show a single MQTT client configuration.
pub async fn mqtt_show(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<MqttClientResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_mqtt_settings(id).await.context_to()
}

/// Create a new MQTT client configuration.
pub async fn mqtt_create(params: MqttCreateParams<'_>) -> Result<MqttClientResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let transport = params
        .transport
        .map(|t| t.parse())
        .transpose()
        .context_to()?;
    let req = CreateMqttClientRequest {
        url: params.url,
        transport,
        host: params.host,
        port: params.port,
        enabled: params.enabled,
        client_id: params.client_id,
        username: params.username,
        password: params.password.map(SecretString::new),
        ca_pem: params.ca_pem.map(SecretString::new),
        topic_prefix: params.topic_prefix,
        ha_discovery: params.ha_discovery,
        ha_discovery_prefix: params.ha_discovery_prefix,
    };
    client.create_mqtt_settings(&req).await.context_to()
}

/// Update an existing MQTT client configuration.
pub async fn mqtt_update(params: MqttUpdateParams<'_>) -> Result<MqttClientResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let transport = params
        .transport
        .map(|t| t.parse())
        .transpose()
        .context_to()?;
    let username = params.username.map(serde_json::Value::String);
    let password = params.password.map(serde_json::Value::String);
    let ca_pem = params.ca_pem.map(serde_json::Value::String);
    let req = UpdateMqttClientRequest {
        url: params.url,
        transport,
        host: params.host,
        port: params.port,
        enabled: params.enabled,
        client_id: params.client_id,
        username,
        password,
        ca_pem,
        topic_prefix: params.topic_prefix,
        ha_discovery: params.ha_discovery,
        ha_discovery_prefix: params.ha_discovery_prefix,
    };
    client
        .update_mqtt_settings(&params.id, &req)
        .await
        .context_to()
}

/// Delete an MQTT client configuration.
pub async fn mqtt_delete(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<DeletedOutput> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.delete_mqtt_settings(id).await.context_to()?;
    Ok(DeletedOutput {
        message: format!("MQTT configuration {id} deleted."),
    })
}

/// Show MQTT client limit.
pub async fn mqtt_limit_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<MqttLimitResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_mqtt_limit().await.context_to()
}

/// Update MQTT client limit.
pub async fn mqtt_limit_update(
    max: u16,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<MqttLimitResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateMqttLimitRequest {
        max_clients_per_tenant: max,
    };
    client.update_mqtt_limit(&req).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mqtt_list_empty() {
        let resp: Vec<MqttClientResponse> = vec![];
        assert!(resp.to_human_string().contains("No MQTT"));
    }

    #[test]
    fn mqtt_limit_human_output() {
        let resp = MqttLimitResponse {
            max_clients_per_tenant: 10,
        };
        assert!(resp.to_human_string().contains("10"));
    }
}
