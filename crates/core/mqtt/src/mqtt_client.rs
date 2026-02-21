use std::fmt;
use std::time::Duration;

use rootcause::prelude::*;
use rumqttc::{AsyncClient, EventLoop, LastWill, MqttOptions, QoS, Transport};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uptrakit_internal_wire::MqttClientConnectionStatus;
use uptrakit_internal_wire::MqttTransport;
use uptrakit_internal_wire::SecretString;
use uptrakit_shared_macros::impl_report_conversion;

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
    pub username: Option<SecretString>,
    /// Optional password for authentication.
    pub password: Option<SecretString>,
    /// Optional custom CA certificate in PEM format (for private brokers).
    pub ca_pem: Option<SecretString>,
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
            .field("username", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .field("ca_pem", &self.ca_pem.as_ref().map(|_| "[REDACTED]"))
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
    shutdown_token: CancellationToken,
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
        let outcome = shutdown_task(self.shutdown_token, self.task).await;
        match outcome {
            ShutdownOutcome::Completed => {}
            ShutdownOutcome::TimedOut => {
                tracing::warn!("MQTT event loop shutdown timed out; task aborted");
            }
            ShutdownOutcome::JoinError => {
                tracing::warn!("MQTT event loop task ended with join error");
            }
        }
    }
}

#[derive(Debug, Copy, Clone)]
enum ShutdownOutcome {
    Completed,
    TimedOut,
    JoinError,
}

/// Errors returned by [`start`].
#[derive(Debug, Error)]
pub enum MqttError {
    /// Wraps [`rumqttc::ClientError`].
    #[error("MQTT client error: {0}")]
    Client(#[from] rumqttc::ClientError),
}

pub type Result<T> = std::result::Result<T, Report<MqttError>>;

impl_report_conversion!(rumqttc::ClientError => MqttError::Client);

/// Status event emitted by a running MQTT client.
#[derive(Debug, Clone)]
pub struct MqttClientStatusEvent {
    pub mqtt_client_id: uuid::Uuid,
    pub status: MqttClientConnectionStatus,
}

#[derive(Clone)]
struct MqttClientStatusReporter {
    mqtt_client_id: uuid::Uuid,
    sender: mpsc::UnboundedSender<MqttClientStatusEvent>,
}

impl MqttClientStatusReporter {
    fn new(
        mqtt_client_id: uuid::Uuid,
        sender: mpsc::UnboundedSender<MqttClientStatusEvent>,
    ) -> Self {
        Self {
            mqtt_client_id,
            sender,
        }
    }

    fn report(&self, status: MqttClientConnectionStatus) {
        let _ = self.sender.send(MqttClientStatusEvent {
            mqtt_client_id: self.mqtt_client_id,
            status,
        });
    }
}

/// Connect to the MQTT broker described by `config`.
///
/// Publishes a retained `online` message on every successful connection and
/// registers an LWT so the broker publishes `offline` on unexpected disconnect.
pub async fn start(
    config: MqttConfig,
    status_sender: Option<mpsc::UnboundedSender<MqttClientStatusEvent>>,
    mqtt_client_id: uuid::Uuid,
) -> Result<MqttHandle> {
    let topic = status_topic(&config.topic_prefix);
    let options = build_mqtt_options(&config);
    let reporter =
        status_sender.map(|sender| MqttClientStatusReporter::new(mqtt_client_id, sender));
    report_status(&reporter, MqttClientConnectionStatus::Connecting);

    let (client, event_loop) = AsyncClient::new(options, 10);
    let shutdown_token = CancellationToken::new();

    // Publish initial online message.
    client
        .publish(&topic, QoS::AtLeastOnce, true, "online")
        .await
        .context_to::<MqttError>()?;

    let task_topic = topic.clone();
    let task_client = client.clone();
    let task_token = shutdown_token.clone();
    let task = tokio::spawn(run_event_loop(
        event_loop,
        task_client,
        task_topic,
        task_token,
        reporter,
    ));

    Ok(MqttHandle {
        client,
        topic,
        task,
        shutdown_token,
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
        opts.set_credentials(
            username.expose_secret(),
            password.as_ref().map(|p| p.expose_secret()).unwrap_or(""),
        );
    }

    match config.transport {
        MqttTransport::Tcp => {}
        MqttTransport::Tls => {
            let ca = config
                .ca_pem
                .as_ref()
                .map(|pem| pem.expose_secret().as_bytes().to_vec())
                .unwrap_or_default();
            let tls_config = rumqttc::TlsConfiguration::Simple {
                ca,
                alpn: None,
                client_auth: None,
            };
            opts.set_transport(Transport::Tls(tls_config));
        }
    }

    opts
}

async fn run_event_loop(
    mut event_loop: EventLoop,
    client: AsyncClient,
    topic: String,
    shutdown_token: CancellationToken,
    reporter: Option<MqttClientStatusReporter>,
) {
    loop {
        tokio::select! {
            _ = shutdown_token.cancelled() => {
                tracing::debug!("MQTT event loop shutdown requested");
                break;
            }
            poll = event_loop.poll() => {
                match poll {
                    Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_))) => {
                        tracing::info!("MQTT connected");
                        report_status(&reporter, MqttClientConnectionStatus::Online);
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
                        report_status(&reporter, MqttClientConnectionStatus::Offline);
                        tokio::select! {
                            _ = shutdown_token.cancelled() => {
                                tracing::debug!("MQTT event loop shutdown requested");
                                break;
                            }
                            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                                report_status(&reporter, MqttClientConnectionStatus::Connecting);
                            }
                        }
                    }
                }
            }
        }
    }
}

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

async fn shutdown_task(
    shutdown_token: CancellationToken,
    mut task: tokio::task::JoinHandle<()>,
) -> ShutdownOutcome {
    shutdown_token.cancel();
    match tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut task).await {
        Ok(Ok(())) => ShutdownOutcome::Completed,
        Ok(Err(e)) => {
            tracing::warn!("MQTT event loop task join error: {e}");
            ShutdownOutcome::JoinError
        }
        Err(_) => {
            task.abort();
            match task.await {
                Ok(()) => ShutdownOutcome::TimedOut,
                Err(e) => {
                    tracing::warn!("MQTT event loop task abort error: {e}");
                    ShutdownOutcome::JoinError
                }
            }
        }
    }
}

fn report_status(reporter: &Option<MqttClientStatusReporter>, status: MqttClientConnectionStatus) {
    if let Some(reporter) = reporter {
        reporter.report(status);
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
            ca_pem: None,
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
            username: Some(SecretString::new("user".into())),
            password: Some(SecretString::new("pass".into())),
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
    fn credentials_redacted_in_debug() {
        let config = MqttConfig {
            password: Some(SecretString::new("super-secret-password".into())),
            username: Some(SecretString::new("user".into())),
            ..tcp_config()
        };

        let debug_str = format!("{config:?}");
        assert!(
            !debug_str.contains("password: \"super-secret-password\""),
            "password should not appear in debug output"
        );
        assert!(
            !debug_str.contains("username: \"user\""),
            "username should not appear in debug output"
        );
        assert!(
            debug_str.contains("username: \"[REDACTED]\""),
            "debug output should show redacted username"
        );
        assert!(
            debug_str.contains("password: \"[REDACTED]\""),
            "debug output should show redacted password"
        );
        assert!(
            !debug_str.contains("username: None"),
            "username should not show None in debug output"
        );
        assert!(
            !debug_str.contains("password: None"),
            "password should not show None in debug output"
        );

        let none_config = MqttConfig {
            password: None,
            username: None,
            ..tcp_config()
        };

        let none_debug = format!("{none_config:?}");
        assert!(
            none_debug.contains("username: \"[REDACTED]\""),
            "debug output should show redacted username when None"
        );
        assert!(
            none_debug.contains("password: \"[REDACTED]\""),
            "debug output should show redacted password when None"
        );
        assert!(
            !none_debug.contains("username: None"),
            "username should not show None in debug output"
        );
        assert!(
            !none_debug.contains("password: None"),
            "password should not show None in debug output"
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

    #[test]
    fn tls_with_custom_ca_pem_does_not_panic() {
        let config = MqttConfig {
            transport: MqttTransport::Tls,
            port: 8883,
            ca_pem: Some(SecretString::new(
                "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----".into(),
            )),
            ..tcp_config()
        };
        let _opts = build_mqtt_options(&config);
    }

    #[tokio::test]
    async fn shutdown_task_completes_before_timeout() {
        let token = CancellationToken::new();
        let handle = tokio::spawn(async {});
        let outcome = shutdown_task(token, handle).await;
        assert!(matches!(outcome, ShutdownOutcome::Completed));
    }

    #[tokio::test]
    async fn shutdown_task_aborts_on_timeout() {
        let token = CancellationToken::new();
        let handle = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        let outcome = shutdown_task(token, handle).await;
        assert!(matches!(
            outcome,
            ShutdownOutcome::TimedOut | ShutdownOutcome::JoinError
        ));
    }
}
