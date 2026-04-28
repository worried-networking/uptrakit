use std::sync::Arc;

use uptrakit_wire::ControllerMessage;

use crate::AppState;

pub(super) struct PreparedReconnectReplay {
    pub(super) messages: Vec<ControllerMessage>,
}

pub(super) async fn prepare_reconnect_replay(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    runtime_instance_id: Option<uuid::Uuid>,
    has_update_hooks: bool,
    allow_replay: bool,
) -> PreparedReconnectReplay {
    let successor_dispatch_mode = if has_update_hooks && allow_replay {
        super::updates::ReconnectSuccessorDispatchMode::ReplayPrepared
    } else {
        super::updates::ReconnectSuccessorDispatchMode::Immediate
    };

    if let Err(error) = super::updates::recover_owned_updates_on_connect_with_dispatch_mode(
        state,
        service_id,
        runtime_instance_id,
        successor_dispatch_mode,
    )
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
        };
    }

    match super::updates::prepare_pending_replay_messages(state, service_id).await {
        Ok(messages) => PreparedReconnectReplay { messages },
        Err(error) => {
            tracing::error!(
                error = %error,
                %service_id,
                "failed to prepare pending updates on reconnect"
            );
            PreparedReconnectReplay {
                messages: Vec::new(),
            }
        }
    }
}
