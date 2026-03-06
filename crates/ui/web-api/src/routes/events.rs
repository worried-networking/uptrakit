use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::middleware::permission::CanViewAgents;
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
pub async fn stream_events(
    State(state): State<Arc<AppState>>,
    CanViewAgents(_auth_user): CanViewAgents,
    tenant: TenantContext,
) -> Response {
    let tenant_id = tenant.tenant_id;
    let shutdown_token = state.shutdown_token.clone();

    let rx = state.event_broadcaster.subscribe(tenant_id).await;

    let stream = async_stream::stream! {
        let mut rx = rx;

        loop {
            tokio::select! {
                event = rx.recv() => {
                    match event {
                        Ok(admin_event) => {
                            let event_name = admin_event.event_name();
                            if let Ok(json) = serde_json::to_string(&admin_event) {
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

        state.event_broadcaster.unsubscribe(tenant_id).await;
    };

    Sse::new(stream)
        .keep_alive(KeepAlive::default().interval(std::time::Duration::from_secs(15)))
        .into_response()
}
