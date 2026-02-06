use std::fmt;
use std::time::Duration;

use rootcause::ReportConversion;
use rootcause::prelude::*;
use rumqttc::{AsyncClient, EventLoop, LastWill, MqttOptions, QoS, Transport};
use thiserror::Error;
use uptrakit_web_api_types::mqtt_transport::MqttTransport;

/// Configuration for connecting to an MQTT broker.
pub struct MqttConfig {
    /// Transport protocol.
    pub transport: MqttTransport,
    /// Broker hostname.
    pub host: String,
    /// Broker port.
    pub port: u16,
    /// MQTT client ID.
    pub client_id: String,
    /// Optional username for authentication.
    pub username: Option<String>,
    /// Optional password for authentication.
    pub password: Option<String>,
    /// Topic prefix (e.g. `"uptrakit"`).
    pub topic_prefix: String,
}

impl fmt::Debug for MqttConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttConfig")
            .field("transport", &self.transport)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("client_id", &self.client_id)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("topic_prefix", &self.topic_prefix)
            .finish()
    }
}

/// Handle to a running MQTT connection.
///
/// Dropping without calling [`shutdown`](MqttHandle::shutdown) will abort the
/// event-loop task (the broker will then publish the LWT).
pub struct MqttHandle {
    client: AsyncClient,
    topic: String,
    task: tokio::task::JoinHandle<()>,
}

impl MqttHandle {
    /// Publish a retained `offline` message, disconnect, and wait for the
    /// event-loop task to finish.
    pub async fn shutdown(self) {
        let _ = self
            .client
            .publish(&self.topic, QoS::AtLeastOnce, true, "offline")
            .await;
        let _ = self.client.disconnect().await;
        let _ = self.task.await;
    }
}

/// Errors returned by [`start`].
#[derive(Debug, Error)]
pub enum MqttError {
    /// Wraps [`rumqttc::ClientError`].
    #[error("MQTT client error: {0}")]
    Client(#[from] rumqttc::ClientError),
}

pub type Result<T> = std::result::Result<T, Report<MqttError>>;

impl<T> ReportConversion<rumqttc::ClientError, markers::Mutable, T> for MqttError
where
    MqttError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<rumqttc::ClientError, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(MqttError::Client)
    }
}

/// Connect to the MQTT broker described by `config`.
///
/// Publishes a retained `online` message on every successful connection and
/// registers an LWT so the broker publishes `offline` on unexpected disconnect.
pub async fn start(config: MqttConfig) -> Result<MqttHandle> {
    let topic = status_topic(&config.topic_prefix);
    let options = build_mqtt_options(&config);

    let (client, event_loop) = AsyncClient::new(options, 10);

    // Publish initial online message.
    client
        .publish(&topic, QoS::AtLeastOnce, true, "online")
        .await
        .context_to::<MqttError>()?;

    let task_topic = topic.clone();
    let task_client = client.clone();
    let task = tokio::spawn(run_event_loop(event_loop, task_client, task_topic));

    Ok(MqttHandle {
        client,
        topic,
        task,
    })
}

fn status_topic(prefix: &str) -> String {
    format!("{prefix}/status")
}

fn build_mqtt_options(config: &MqttConfig) -> MqttOptions {
    let mut opts = MqttOptions::new(&config.client_id, &config.host, config.port);
    opts.set_keep_alive(Duration::from_secs(30));
    opts.set_clean_session(true);

    let lwt = LastWill::new(
        status_topic(&config.topic_prefix),
        "offline",
        QoS::AtLeastOnce,
        true,
    );
    opts.set_last_will(lwt);

    if let (Some(username), password) = (&config.username, &config.password) {
        opts.set_credentials(username, password.as_deref().unwrap_or(""));
    }

    match config.transport {
        MqttTransport::Tcp => {}
        MqttTransport::Tls => {
            let tls_config = rumqttc::TlsConfiguration::Simple {
                ca: Vec::new(),
                alpn: None,
                client_auth: None,
            };
            opts.set_transport(Transport::Tls(tls_config));
        }
    }

    opts
}

async fn run_event_loop(mut event_loop: EventLoop, client: AsyncClient, topic: String) {
    loop {
        match event_loop.poll().await {
            Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_))) => {
                tracing::info!("MQTT connected");
                if let Err(e) = client
                    .publish(&topic, QoS::AtLeastOnce, true, "online")
                    .await
                {
                    tracing::warn!("failed to publish online status: {e}");
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("MQTT error: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tcp_config() -> MqttConfig {
        MqttConfig {
            transport: MqttTransport::Tcp,
            host: "localhost".into(),
            port: 1883,
            client_id: "test".into(),
            username: None,
            password: None,
            topic_prefix: "myprefix".into(),
        }
    }

    #[test]
    fn mqtt_options_sets_last_will() {
        let opts = build_mqtt_options(&tcp_config());
        let lwt = opts.last_will().expect("LWT should be set");

        assert_eq!(lwt.topic, "myprefix/status");
        assert_eq!(lwt.message, b"offline".as_slice());
        assert!(lwt.retain, "LWT should be retained");
    }

    #[test]
    fn mqtt_options_sets_credentials_when_provided() {
        let config = MqttConfig {
            username: Some("user".into()),
            password: Some("pass".into()),
            ..tcp_config()
        };

        let opts = build_mqtt_options(&config);
        let login = opts.credentials().expect("credentials should be set");

        assert_eq!(login.username, "user");
        assert_eq!(login.password, "pass");
    }

    #[test]
    fn mqtt_options_no_credentials_when_none() {
        let opts = build_mqtt_options(&tcp_config());
        assert!(opts.credentials().is_none());
    }

    #[test]
    fn status_topic_uses_prefix() {
        assert_eq!(status_topic("uptrakit"), "uptrakit/status");
        assert_eq!(status_topic("home/sensor"), "home/sensor/status");
    }

    #[test]
    fn password_redacted_in_debug() {
        let config = MqttConfig {
            password: Some("super-secret-password".into()),
            username: Some("user".into()),
            ..tcp_config()
        };

        let debug_str = format!("{config:?}");
        assert!(
            !debug_str.contains("super-secret-password"),
            "password should not appear in debug output"
        );
        assert!(
            debug_str.contains("[REDACTED]"),
            "debug output should show [REDACTED]"
        );
    }

    #[test]
    fn debug_includes_transport() {
        let config = MqttConfig {
            transport: MqttTransport::Tls,
            ..tcp_config()
        };

        let debug_str = format!("{config:?}");
        assert!(debug_str.contains("Tls"));
    }

    #[test]
    fn tls_transport_sets_tls() {
        let config = MqttConfig {
            transport: MqttTransport::Tls,
            port: 8883,
            ..tcp_config()
        };
        // Just verify it doesn't panic
        let _opts = build_mqtt_options(&config);
    }
}
