use std::sync::Arc;

use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_notification_plugin_core::{DeliveryMessage, NotificationPluginError};
use uptrakit_plugin_infrastructure_core::NotificationTransport;
use uptrakit_shared_macros::impl_report_conversion;

/// Error returned by [`deliver`] and [`build_delivery_message`].
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum NotificationDeliveryError {
    #[error("{0}")]
    DeliveryFailed(NotificationPluginError),
    #[error("failed to serialize notification event details: {0}")]
    SerializationFailed(serde_json::Error),
}

impl_report_conversion!(NotificationPluginError => NotificationDeliveryError::DeliveryFailed);

pub type Result<T> = std::result::Result<T, rootcause::Report<NotificationDeliveryError>>;

/// Invoke a transport for a single channel delivery.
///
/// The caller is responsible for looking up the transport and handling
/// `TransportNotFound` before calling this function.
///
/// # Errors
///
/// Returns [`NotificationDeliveryError::DeliveryFailed`] if the transport
/// layer returns an error.
pub async fn deliver(
    transport: Arc<dyn NotificationTransport>,
    channel_config: &serde_json::Value,
    settings_bag: &serde_json::Value,
    message: &DeliveryMessage,
) -> Result<()> {
    transport
        .deliver(channel_config, settings_bag, message)
        .await
        .context_to()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — assert!(result.is_ok/is_err()) is idiomatic in tests"
    )]

    use std::sync::Arc;

    use async_trait::async_trait;
    use rootcause::report;
    use uptrakit_notification_plugin_core::{DeliveryMessage, NotificationPluginError};
    use uptrakit_plugin_infrastructure_core::{NotificationTransport, PluginMeta, PluginTypeId};

    use super::*;

    struct StubTransport {
        should_fail: bool,
    }

    impl PluginMeta for StubTransport {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::new("stub")
        }
    }

    #[async_trait]
    impl NotificationTransport for StubTransport {
        async fn deliver(
            &self,
            _config: &serde_json::Value,
            _settings: &serde_json::Value,
            _message: &DeliveryMessage,
        ) -> uptrakit_notification_plugin_core::Result<()> {
            if self.should_fail {
                Err(report!(NotificationPluginError::DeliveryFailed(
                    "stub error".to_string()
                )))
            } else {
                Ok(())
            }
        }
    }

    fn stub_message() -> DeliveryMessage {
        DeliveryMessage::new("title", "body", None, serde_json::json!({}), vec![])
    }

    #[tokio::test]
    async fn deliver_success_path() {
        let transport = Arc::new(StubTransport { should_fail: false });
        let result = deliver(
            transport,
            &serde_json::json!({}),
            &serde_json::json!({}),
            &stub_message(),
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn deliver_wraps_transport_error_as_delivery_failed() {
        let transport = Arc::new(StubTransport { should_fail: true });
        let result = deliver(
            transport,
            &serde_json::json!({}),
            &serde_json::json!({}),
            &stub_message(),
        )
        .await;
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("stub error"),
            "expected error containing 'stub error', got: {err}",
        );
    }
}
