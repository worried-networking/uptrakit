//! Single source of truth for MQTT service deployment facts.
//!
//! Both the standalone binary (`crates/core/mqtt/src/main.rs`) and the
//! controller's embedded-service host
//! (`crates/core/controller-runtime/src/service_host/builtins.rs`) build the
//! same [`MqttHandler`] and must agree on the same app name, capability set,
//! scope, yield policy, and shutdown timeout. This module is the one place
//! those facts are defined; consumers read them from here rather than
//! re-declaring their own copies.

use std::collections::BTreeSet;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use uptrakit_service_platform::{ServiceScope, YieldPolicy};
use uptrakit_wire::{Capability, ServiceTransport};
use uuid::Uuid;

use crate::MqttHandler;

/// App name used across enrollment, config-store keys, and provider ids.
pub const MQTT_SERVICE_APP_NAME: &str = "uptrakit-mqtt";

/// MQTT is global infrastructure, not bound to any tenant.
pub const SCOPE: ServiceScope = ServiceScope::System;

/// Yield to any external MQTT service claiming the same app name, regardless
/// of which host it runs on — MQTT holds no host-local state that would
/// require host affinity.
pub const YIELD_POLICY: YieldPolicy = YieldPolicy::SameServiceAnywhere;

/// Shutdown timeout for the embedded MQTT task.
///
/// Each MQTT client may take up to `OPERATION_TIMEOUT (5s) + SHUTDOWN_TIMEOUT
/// (5s) = 10s` to disconnect cleanly. Clients shut down in parallel via
/// `FuturesUnordered`, so N clients ≈ 10s worst-case regardless of count. 5s
/// of safety margin added.
pub const EMBEDDED_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);

/// Capabilities advertised by the MQTT service.
///
/// `SystemService` marks this service as global infrastructure (routed to
/// the `system_services` table instead of the per-tenant `services` table).
/// `UiSurfaces` enables the MQTT clients settings page.
#[must_use]
pub fn capabilities() -> BTreeSet<Capability> {
    [
        Capability::SystemService,
        Capability::UpdateTracking,
        Capability::GracefulShutdown,
        Capability::UiSurfaces,
        Capability::WorkloadClaims,
    ]
    .into_iter()
    .collect()
}

/// Construct a fresh [`MqttHandler`].
#[must_use]
pub fn new_handler() -> MqttHandler {
    MqttHandler::new()
}

/// Run the MQTT service handler in embedded mode.
///
/// Thin wrapper over [`uptrakit_service_sdk::run_embedded_service`] so every
/// embedded consumer boots the same handler the same way.
pub async fn run_embedded(
    service_id: Uuid,
    transport: impl ServiceTransport,
    drain: CancellationToken,
    abort: CancellationToken,
) {
    uptrakit_service_sdk::run_embedded_service(service_id, new_handler(), transport, drain, abort)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_includes_expected_set() {
        let caps = capabilities();
        assert!(caps.contains(&Capability::SystemService));
        assert!(caps.contains(&Capability::UpdateTracking));
        assert!(caps.contains(&Capability::GracefulShutdown));
        assert!(caps.contains(&Capability::UiSurfaces));
        assert!(caps.contains(&Capability::WorkloadClaims));
    }
}
