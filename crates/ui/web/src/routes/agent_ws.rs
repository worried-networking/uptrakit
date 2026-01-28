use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{Message, WebSocket};
use axum::response::IntoResponse;
use uptrakit_internal_wire::{
    AgentMessage, ControllerMessage, PingPayload, PongPayload, now_millis,
};

pub async fn agent_ws(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_agent_socket)
}

async fn handle_agent_socket(mut socket: WebSocket) {
    tracing::debug!("agent connected");

    while let Some(msg) = socket.recv().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(error = %e, "websocket receive error");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                if let Err(e) = handle_text_message(&mut socket, &text).await {
                    tracing::debug!(error = %e, "error handling message");
                    break;
                }
            }
            Message::Close(_) => {
                tracing::debug!("agent sent close frame");
                break;
            }
            _ => {
                // Ignore binary, ping, pong frames
            }
        }
    }

    tracing::debug!("agent disconnected");
}

async fn handle_text_message(
    socket: &mut WebSocket,
    text: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let agent_msg: AgentMessage = serde_json::from_str(text)?;

    match agent_msg {
        AgentMessage::Ping(PingPayload { agent_ts }) => {
            let controller_ts = now_millis();
            let response = ControllerMessage::Pong(PongPayload {
                agent_ts,
                controller_ts,
            });
            let response_json = serde_json::to_string(&response)?;
            socket.send(Message::Text(response_json.into())).await?;
            tracing::trace!(agent_ts, controller_ts, "ping/pong");
        }
    }

    Ok(())
}
