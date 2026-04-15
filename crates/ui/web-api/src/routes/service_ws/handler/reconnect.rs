use std::sync::Arc;

use uptrakit_internal_wire::ControllerMessage;

use crate::AppState;

pub(super) struct PreparedReconnectReplay {
    pub(super) messages: Vec<ControllerMessage>,
    pub(super) replay_prepared: bool,
}

pub(super) async fn prepare_reconnect_replay(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    runtime_instance_id: Option<uuid::Uuid>,
    has_update_hooks: bool,
    allow_replay: bool,
) -> PreparedReconnectReplay {
    if let Err(error) =
        super::updates::recover_owned_updates_on_connect(state, service_id, runtime_instance_id)
            .await
    {
        tracing::error!(
            error = %error,
            %service_id,
            "failed to recover owned updates on connect"
        );
    }

    if !(has_update_hooks && allow_replay) {
        return PreparedReconnectReplay {
            messages: Vec::new(),
            replay_prepared: false,
        };
    }

    match super::updates::prepare_pending_replay_messages(state, service_id).await {
        Ok(messages) => PreparedReconnectReplay {
            messages,
            replay_prepared: true,
        },
        Err(error) => {
            tracing::error!(
                error = %error,
                %service_id,
                "failed to prepare pending updates on reconnect"
            );
            PreparedReconnectReplay {
                messages: Vec::new(),
                replay_prepared: false,
            }
        }
    }
}
