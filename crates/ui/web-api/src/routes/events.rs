use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::middleware::permission::CanViewServices;
use crate::middleware::tenant_context::TenantContext;

/// SSE stream for real-time admin events.
///
/// Authenticated endpoint. Any authenticated user can subscribe.
/// Pushes lightweight invalidation signals for the user's tenant so the
/// frontend can fetch fresh data on demand.
///
/// # Events
///
/// Event names correspond to [`AdminEvent`](uptrakit_web_api_types::events::AdminEvent)
/// variant names (snake_case). The `data:` field contains the variant's inner
/// fields as JSON.
#[tracing::instrument(skip_all)]
pub async fn stream_events(
    State(state): State<Arc<AppState>>,
    CanViewServices(_auth_user): CanViewServices,
    tenant: TenantContext,
) -> Response {
    let tenant_id = tenant.tenant_id;
    let shutdown_token = state.shutdown_token.clone();

    let rx = state
        .notification
        .event_broadcaster
        .subscribe(tenant_id)
        .await;

    let stream = async_stream::stream! {
        let mut rx = rx;

        loop {
            tokio::select! {
                event = rx.recv() => {
                    match event {
                        Ok(admin_event) => {
                            let event_name = admin_event.event_name();
                            let inner = extract_sse_data(&admin_event);
                            if let Ok(json) = serde_json::to_string(&inner) {
                                yield Ok::<_, Infallible>(
                                    Event::default().event(event_name).data(json)
                                );
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::debug!(
                                tenant_id = %tenant_id,
                                missed = n,
                                "SSE subscriber lagged, continuing"
                            );
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
                _ = shutdown_token.cancelled() => {
                    break;
                }
            }
        }

        state.notification.event_broadcaster.unsubscribe(tenant_id).await;
    };

    Sse::new(stream)
        .keep_alive(KeepAlive::default().interval(std::time::Duration::from_secs(15)))
        .into_response()
}

/// Extract the inner JSON object from an externally-tagged `AdminEvent` value.
///
/// `AdminEvent` serialises as `{"variant_name": {...inner fields...}}` for struct
/// variants and as the bare string `"variant_name"` for unit variants (e.g. `DataReset`).
/// The SSE `event:` line already carries the variant name, so `data:` must hold only
/// the inner fields.  This function performs that extraction.
pub(crate) fn extract_sse_data(
    event: &uptrakit_web_api_types::events::AdminEvent,
) -> serde_json::Value {
    match serde_json::to_value(event) {
        Ok(serde_json::Value::Object(map)) => {
            let v = map
                .into_values()
                .next()
                .unwrap_or(serde_json::Value::Object(Default::default()));
            if v.is_null() {
                serde_json::Value::Object(Default::default())
            } else {
                v
            }
        }
        _ => serde_json::Value::Object(Default::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_web_api_types::events::AdminEvent;
    use uuid::Uuid;

    #[test]
    fn sse_data_struct_variant_exposes_inner_fields() {
        let id = Uuid::nil();
        let event = AdminEvent::UpdateStarted {
            update_history_id: id,
            host_id: id,
            software_item_id: id,
            interactive: true,
        };
        let data = extract_sse_data(&event);
        // Must NOT contain the outer "update_started" key.
        assert!(
            data.get("update_started").is_none(),
            "outer key must be absent: {data}"
        );
        // Must expose inner fields directly.
        assert!(
            data.get("update_history_id").is_some(),
            "update_history_id missing: {data}"
        );
        assert!(data.get("host_id").is_some(), "host_id missing: {data}");
        assert!(
            data.get("interactive").is_some(),
            "interactive missing: {data}"
        );
    }

    #[test]
    fn sse_data_unit_variant_emits_empty_object() {
        let event = AdminEvent::DataReset;
        let data = extract_sse_data(&event);
        assert!(data.is_object(), "expected object, got: {data}");
        assert_eq!(
            data.as_object().map(|m| m.len()).unwrap_or(1),
            0,
            "expected empty object: {data}"
        );
    }

    #[test]
    fn sse_data_surfaces_changed_emits_empty_object() {
        let event = AdminEvent::SurfacesChanged;
        let data = extract_sse_data(&event);
        assert!(data.is_object(), "expected object, got: {data}");
        assert_eq!(
            data.as_object().map(|m| m.len()).unwrap_or(1),
            0,
            "expected empty object: {data}"
        );
    }

    #[test]
    fn sse_data_update_triggered_exposes_inner_fields() {
        let id = Uuid::nil();
        let event = AdminEvent::UpdateTriggered {
            update_history_id: id,
            host_id: id,
            software_item_id: id,
        };
        let data = extract_sse_data(&event);
        assert!(
            data.get("update_triggered").is_none(),
            "outer key must be absent: {data}"
        );
        assert!(
            data.get("update_history_id").is_some(),
            "update_history_id missing: {data}"
        );
        assert!(data.get("host_id").is_some(), "host_id missing: {data}");
        assert!(
            data.get("software_item_id").is_some(),
            "software_item_id missing: {data}"
        );
    }
}
