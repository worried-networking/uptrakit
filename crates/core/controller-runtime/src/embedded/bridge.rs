//! Per-service bridge between an embedded service and its `MessageProcessor`.
//!
//! The bridge connects the service-side `EmbeddedTransport` channels to the
//! controller-side response delivery channel. The response forwarder task
//! reads `ControllerMessage` replies from the processor and forwards them
//! to the service's receive channel.

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uptrakit_wire::ControllerMessage;

/// Run the response forwarder: reads controller messages from the processor
/// output channel and sends them to the embedded service's receive channel.
pub(crate) async fn run_response_forwarder(
    mut proc_rx: mpsc::Receiver<ControllerMessage>,
    service_tx: mpsc::Sender<ControllerMessage>,
    cancel: CancellationToken,
) {
    loop {
        let msg = tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            msg = proc_rx.recv() => match msg {
                Some(m) => m,
                None => break,
            },
        };
        tokio::select! {
            biased;
            () = cancel.cancelled() => {
                // Cancel fired after receiving msg but before forwarding.
                // Safe to drop: forwarder_cancel is only cancelled during shutdown,
                // at which point the downstream embedded service is also shutting down.
                tracing::debug!("response forwarder cancelled mid-send; dropping in-flight message");
                break;
            }
            result = service_tx.send(msg) => {
                if result.is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_wire::ControllerMessage;

    use super::*;

    /// Cancel fired while the forwarder is blocked on service_tx.send (channel full).
    /// The forwarder must exit without draining all pending messages.
    #[tokio::test]
    async fn forwarder_exits_on_cancel_when_send_would_block() {
        let (proc_tx, proc_rx) = mpsc::channel::<ControllerMessage>(4);
        // service_tx has capacity 1 so the second send blocks.
        let (service_tx, _service_rx) = mpsc::channel::<ControllerMessage>(1);
        let cancel = CancellationToken::new();

        // Fill proc_rx with 3 messages (more than service_tx can accept without draining).
        for _ in 0..3 {
            proc_tx
                .send(ControllerMessage::Unknown)
                .await
                .expect("send to proc_tx");
        }
        drop(proc_tx);

        let cancel_clone = cancel.clone();
        let forwarder = tokio::spawn(run_response_forwarder(proc_rx, service_tx, cancel));

        // Give the forwarder a tick to start and forward the first message (fills service_tx).
        tokio::task::yield_now().await;

        // Cancel while the second send is blocking.
        cancel_clone.cancel();

        // Forwarder must exit promptly; it must NOT drain all 3 messages.
        forwarder.await.expect("forwarder task panicked");
    }
}
