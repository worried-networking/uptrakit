//! Per-service bridge between an embedded service and its `MessageProcessor`.
//!
//! The bridge connects the service-side `EmbeddedTransport` channels to the
//! controller-side response delivery channel. The response forwarder task
//! reads `ControllerMessage` replies from the processor and forwards them
//! to the service's receive channel.

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uptrakit_internal_wire::ControllerMessage;

/// Run the response forwarder: reads controller messages from the processor
/// output channel and sends them to the embedded service's receive channel.
pub(crate) async fn run_response_forwarder(
    mut proc_rx: mpsc::Receiver<ControllerMessage>,
    service_tx: mpsc::Sender<ControllerMessage>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            msg = proc_rx.recv() => {
                let Some(msg) = msg else { break };
                if service_tx.send(msg).await.is_err() {
                    break;
                }
            }
        }
    }
}
