use std::fmt;
use std::time::Duration;

use rootcause::prelude::*;
use rumqttc::{AsyncClient, EventLoop, LastWill, MqttOptions, Packet, QoS, Transport};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uptrakit_internal_wire::SecretString;
use uptrakit_service_sdk::Backoff;
use uptrakit_shared_macros::impl_report_conversion;

use crate::types::{MqttClientConnectionStatus, MqttTransport};

/// Configuration for connecting to an MQTT broker.
pub(crate) struct MqttConfig {
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

/// Status event emitted by a running MQTT client.
#[derive(Debug, Clone)]
pub struct MqttClientStatusEvent {
    pub mqtt_client_id: uuid::Uuid,
    pub status: MqttClientConnectionStatus,
}

/// Events emitted by a running MQTT client connection.
#[derive(Debug, Clone)]
pub enum MqttServiceEvent {
    /// Connection-status change (Online/Offline/Connecting).
    Status(MqttClientStatusEvent),
    /// Broker reconnect — discovery configs and state topics must be republished.
    Reconnected(uuid::Uuid),
    /// HA published its birth message (`online`) to the HA status topic —
    /// republish all discovery configs.
    HaOnline(uuid::Uuid),
    /// Inbound MQTT publish on a subscribed topic (used for update commands).
    Command {
        mqtt_client_id: uuid::Uuid,
        topic: String,
    },
    /// Completion of a controller-side service-config mutation initiated from
    /// a surface action. The runtime handles this on its internal event loop
    /// so it can keep receiving controller messages while the ACK is pending.
    SurfaceConfigRequestCompleted {
        request_id: uuid::Uuid,
        local_update: Option<uptrakit_internal_wire::payloads::ServiceConfigUpdatedPayload>,
        result: Option<serde_json::Value>,
        error: Option<String>,
    },
}

/// Handle to a running MQTT connection.
///
/// Dropping without calling [`shutdown`](MqttHandle::shutdown) will abort the
/// event-loop task (the broker will then publish the LWT).
pub(crate) struct MqttHandle {
    client: AsyncClient,
    topic: String,
    task: tokio::task::JoinHandle<()>,
    shutdown_token: CancellationToken,
}

impl MqttHandle {
    /// Publish a retained message.
    ///
    /// Uses a non-blocking try-send: returns [`MqttError::ChannelFull`]
    /// immediately if the internal request channel is full rather than
    /// blocking up to [`OPERATION_TIMEOUT`].  A full channel means the event
    /// loop is unable to drain requests (e.g. during a rumqttc collision-wait
    /// or reconnect backoff).  Blocking here would prevent [`poll_event`] from
    /// running in the parent select loop, causing the 512-slot MQTT service
    /// event channel to saturate and subsequent `Reconnected` events to be
    /// dropped — making recovery impossible until the controller restarts.
    ///
    /// Callers use [`publish_or_abort!`] so an immediate error simply aborts
    /// the current batch; the state will be republished on the next successful
    /// broker reconnect.
    pub(crate) fn publish_retained(&self, topic: &str, payload: impl Into<Vec<u8>>) -> Result<()> {
        let payload = payload.into();
        self.client
            .try_publish(topic, QoS::AtLeastOnce, true, payload)
            .context_to::<MqttError>()
    }

    /// Subscribe to a topic with QoS `AtLeastOnce`.
    ///
    /// Uses a non-blocking try-send for the same reason as [`publish_retained`]:
    /// blocking here would prevent the parent event loop from draining the MQTT
    /// service event channel, causing `Reconnected` events to be dropped.
    pub(crate) fn subscribe_topic(&self, topic: &str) -> Result<()> {
        self.client
            .try_subscribe(topic, QoS::AtLeastOnce)
            .context_to::<MqttError>()
    }

    /// Unsubscribe from a topic.
    ///
    /// Uses a non-blocking try-send for the same reason as [`publish_retained`].
    pub(crate) fn unsubscribe_topic(&self, topic: &str) -> Result<()> {
        self.client.try_unsubscribe(topic).context_to::<MqttError>()
    }

    /// Publish a retained `offline` message, disconnect, and wait for the
    /// event-loop task to finish.
    pub(crate) async fn shutdown(self) {
        // Use a timeout for the offline publish and disconnect so that
        // shutdown is not blocked indefinitely when the broker connection is
        // already down and the request channel is full.
        let _ = tokio::time::timeout(OPERATION_TIMEOUT, async {
            let _ = self
                .client
                .publish(&self.topic, QoS::AtLeastOnce, true, "offline")
                .await;
            let _ = self.client.disconnect().await;
        })
        .await;
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

/// Timeout for the shutdown sequence: publish retained `offline` + disconnect.
///
/// Only used by [`MqttHandle::shutdown`].  Normal publish/subscribe operations
/// use non-blocking `try_*` calls and never wait.
const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors returned by [`start`].
#[derive(Debug, Error)]
pub(crate) enum MqttError {
    /// Wraps [`rumqttc::ClientError`].
    #[error("MQTT client error: {0}")]
    Client(#[from] rumqttc::ClientError),
}

pub(crate) type Result<T> = std::result::Result<T, Report<MqttError>>;

impl_report_conversion!(rumqttc::ClientError => MqttError::Client);

#[derive(Clone)]
struct MqttEventReporter {
    mqtt_client_id: uuid::Uuid,
    sender: mpsc::Sender<MqttServiceEvent>,
}

impl MqttEventReporter {
    fn new(mqtt_client_id: uuid::Uuid, sender: mpsc::Sender<MqttServiceEvent>) -> Self {
        Self {
            mqtt_client_id,
            sender,
        }
    }

    fn report_status(&self, status: MqttClientConnectionStatus) {
        if let Err(e) = self
            .sender
            .try_send(MqttServiceEvent::Status(MqttClientStatusEvent {
                mqtt_client_id: self.mqtt_client_id,
                status,
            }))
        {
            tracing::warn!(error = %e, "MQTT event channel full, dropping status event");
        }
    }

    fn report_reconnected(&self) {
        if let Err(e) = self
            .sender
            .try_send(MqttServiceEvent::Reconnected(self.mqtt_client_id))
        {
            tracing::warn!(error = %e, "MQTT event channel full, dropping reconnected event");
        }
    }

    fn report_ha_online(&self) {
        if let Err(e) = self
            .sender
            .try_send(MqttServiceEvent::HaOnline(self.mqtt_client_id))
        {
            tracing::warn!(error = %e, "MQTT event channel full, dropping ha_online event");
        }
    }

    fn report_command(&self, topic: String) {
        if let Err(e) = self.sender.try_send(MqttServiceEvent::Command {
            mqtt_client_id: self.mqtt_client_id,
            topic,
        }) {
            tracing::warn!(error = %e, "MQTT event channel full, dropping command event");
        }
    }
}

/// Connect to the MQTT broker described by `config`.
///
/// On every successful connection the event loop publishes a retained `online`
/// status message and subscribes to the optional `ha_status_topic`.  The broker
/// will publish the LWT `offline` on unexpected disconnect.
///
/// The `online` publish and HA subscription are deferred to the event loop and
/// use non-blocking sends so that the event loop can never deadlock against its
/// own request channel (see `run_event_loop` for details).
///
/// When `ha_status_topic` is `Some`, the event loop subscribes to that topic
/// after every `ConnAck` and emits [`MqttServiceEvent::HaOnline`] whenever HA
/// publishes `"online"` to it (HA birth message).
pub(crate) async fn start(
    config: MqttConfig,
    event_sender: Option<mpsc::Sender<MqttServiceEvent>>,
    mqtt_client_id: uuid::Uuid,
    ha_status_topic: Option<String>,
) -> MqttHandle {
    let topic = status_topic(&config.topic_prefix);
    let options = build_mqtt_options(&config);
    let reporter = event_sender.map(|sender| MqttEventReporter::new(mqtt_client_id, sender));
    if let Some(ref r) = reporter {
        r.report_status(MqttClientConnectionStatus::Connecting);
    }

    // Channel capacity of 128 accommodates a full publish_software_states()
    // batch (≈68 messages for 17 items × 2 hosts × 2 topics) without blocking
    // the service-SDK task while the event loop is draining.
    let (client, event_loop) = AsyncClient::new(options, 128);
    let shutdown_token = CancellationToken::new();

    let task_topic = topic.clone();
    let task_client = client.clone();
    let task_token = shutdown_token.clone();
    let reconnect_backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
    let task = tokio::spawn(run_event_loop(
        event_loop,
        task_client,
        task_topic,
        task_token,
        reporter,
        ha_status_topic,
        reconnect_backoff,
    ));

    MqttHandle {
        client,
        topic,
        task,
        shutdown_token,
    }
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
            // `alpn` is intentionally `None`: MQTT over TLS does not require
            // ALPN negotiation and no broker in the supported set mandates it.
            //
            // `client_auth` is intentionally `None`: mTLS for broker
            // authentication is not supported by this client. Server identity
            // is verified via the CA certificate; the broker authenticates
            // clients via MQTT username/password credentials instead.
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
    reporter: Option<MqttEventReporter>,
    ha_status_topic: Option<String>,
    mut reconnect_backoff: Backoff,
) {
    // Tracks whether the "online" retained publish and the HA-status
    // subscription still need to be sent after the most recent ConnAck.
    //
    // # Why not `.await` inside the poll() callback?
    //
    // `AsyncClient::publish/subscribe` send to the bounded flume channel that
    // `EventLoop::poll()` drains.  Calling `.await` on those sends *inside* a
    // `poll()` callback creates a self-deadlock: the channel may already be
    // full (e.g. a large `publish_software_states` batch was in flight when the
    // connection was re-established), so the `.await` blocks — but the only
    // entity that drains the channel is this very task, which is now blocked.
    // Neither task can make progress; the service loop freezes.
    //
    // The fix: use `try_publish`/`try_subscribe` (non-blocking, returns
    // `Err` when full instead of blocking).  If the channel is full on the
    // first attempt we set a flag and retry at the top of every subsequent
    // iteration; `poll()` will have drained at least one slot by then.
    let mut pending_online = false;
    let mut pending_ha_subscribe = false;

    loop {
        // Retry the "online" publish / HA subscription that were deferred
        // from the ConnAck handler.  `try_publish`/`try_subscribe` are O(1)
        // and never block, so spinning here costs nothing.
        if pending_online
            && client
                .try_publish(&topic, QoS::AtLeastOnce, true, "online")
                .is_ok()
        {
            pending_online = false;
        }
        if pending_ha_subscribe {
            if let Some(ref ha_topic) = ha_status_topic {
                if client.try_subscribe(ha_topic, QoS::AtLeastOnce).is_ok() {
                    pending_ha_subscribe = false;
                }
            } else {
                pending_ha_subscribe = false;
            }
        }

        tokio::select! {
            _ = shutdown_token.cancelled() => {
                tracing::debug!("MQTT event loop shutdown requested");
                break;
            }
            poll = event_loop.poll() => {
                match poll {
                    Ok(rumqttc::Event::Incoming(Packet::ConnAck(_))) => {
                        reconnect_backoff.reset();
                        tracing::info!("MQTT connected");
                        if let Some(ref r) = reporter {
                            r.report_status(MqttClientConnectionStatus::Online);
                        }
                        // Schedule non-blocking publish/subscribe for the
                        // next loop iteration.  We cannot .await here — see
                        // the comment at the top of this function.
                        pending_online = true;
                        pending_ha_subscribe = ha_status_topic.is_some();
                        if let Some(ref r) = reporter {
                            r.report_reconnected();
                        }
                    }
                    Ok(rumqttc::Event::Incoming(Packet::Publish(publish))) => {
                        if ha_status_topic.as_deref() == Some(publish.topic.as_str())
                            && publish.payload.as_ref() == b"online"
                        {
                            if let Some(ref r) = reporter {
                                r.report_ha_online();
                            }
                        } else if let Some(ref r) = reporter {
                            r.report_command(publish.topic.clone());
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        // Clear the pending flags: we are disconnected so
                        // there is nothing to retry until the next ConnAck.
                        pending_online = false;
                        pending_ha_subscribe = false;
                        let delay = reconnect_backoff.next_delay();
                        tracing::warn!("MQTT error: {e}; retrying in {delay:?}");
                        if let Some(ref r) = reporter {
                            r.report_status(MqttClientConnectionStatus::Offline);
                        }
                        tokio::select! {
                            _ = shutdown_token.cancelled() => {
                                tracing::debug!("MQTT event loop shutdown requested");
                                break;
                            }
                            _ = tokio::time::sleep(delay) => {
                                if let Some(ref r) = reporter {
                                    r.report_status(MqttClientConnectionStatus::Connecting);
                                }
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
            username: Some(SecretString::new("user")),
            password: Some(SecretString::new("pass")),
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
            password: Some(SecretString::new("super-secret-password")),
            username: Some(SecretString::new("user")),
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
                "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----",
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

    #[tokio::test(start_paused = true)]
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
