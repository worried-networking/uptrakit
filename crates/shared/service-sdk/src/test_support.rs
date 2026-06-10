#![cfg(any(test, feature = "test-support"))]

use std::collections::VecDeque;
use std::future::poll_fn;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use crate::wire_api::{
    ControllerMessage, ServiceMessage, ServiceTransport, TransportClosePolicy, TransportError,
};
use async_trait::async_trait;

// ── WS-level test harness ─────────────────────────────────────────────────────

/// Type alias for a WebSocket stream backed by an in-process duplex pipe.
///
/// Used by WS-level unit tests to exercise enrollment protocol logic without
/// a TLS connection.
pub type MockWsStream = tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>;

/// Start an in-process mock controller that writes `messages` in order and
/// then closes.
///
/// The server task sends all queued messages and then drains any messages the
/// client sends (e.g. `RequestCertificate`) before closing. This prevents
/// the client from receiving a `BrokenPipe` when it attempts to write
/// mid-session.
///
/// Returns the client-side [`MockWsStream`] already past the WebSocket
/// handshake. The server half runs in a detached Tokio task.
///
/// # Panics
///
/// Panics if the in-process handshake fails (indicates a bug in the harness).
#[expect(
    clippy::expect_used,
    reason = "test harness: panicking on setup failure is intentional; the in-process handshake cannot reasonably fail"
)]
pub async fn serve_mock_controller(
    messages: Vec<tokio_tungstenite::tungstenite::Message>,
) -> MockWsStream {
    use futures_util::{SinkExt as _, StreamExt as _};

    let (server_half, client_half) = tokio::io::duplex(8 * 1024);

    tokio::spawn(async move {
        let mut server_ws = tokio_tungstenite::accept_async(server_half)
            .await
            .expect("server WS handshake");
        for msg in messages {
            server_ws.send(msg).await.expect("server send");
        }
        // Drain remaining inbound messages from the client (e.g. RequestCertificate)
        // so the send buffer can flush before the connection closes. Without this
        // drain, dropping `server_ws` while the client is mid-send causes BrokenPipe.
        while let Some(Ok(_)) = server_ws.next().await {}
        // Dropping `server_ws` sends a WS Close frame, signalling EOF to the client.
    });

    let (client_ws, _) = tokio_tungstenite::client_async("ws://mock/", client_half)
        .await
        .expect("client WS handshake");

    client_ws
}

/// Build a JSON-serialised `ControllerEnvelope` carrying a `Certificate`
/// message, suitable for feeding to [`resume_enrollment_inner`] after an
/// `Approved` message.
///
/// The envelope uses sequence number `seq` and the current protocol version.
/// Wire format: `ControllerEnvelope` flattens the message fields at the top
/// level, and `ControllerMessage` is tagged as `"type": "certificate"`.
/// `not_after` is serialised as an i64 millisecond timestamp.
///
/// The cert fields are minimal stubs; they are stored by the identity but
/// not verified against a real CA in unit tests.
pub fn mock_certificate_envelope(seq: u64) -> serde_json::Value {
    use crate::wire_api::CURRENT_PROTOCOL_VERSION;

    // Minimal PEM stub — just enough to be stored by `save_certificate`,
    // which only writes the raw string to disk without validation.
    let stub_pem = "-----BEGIN CERTIFICATE-----\nMIIBstub\n-----END CERTIFICATE-----\n";

    // `not_after` uses `utc_datetime_millis`: i64 milliseconds since epoch.
    // Use a far-future timestamp (year 2099) so identity checks don't fail.
    let not_after_ms: i64 = 4_070_908_800_000;

    serde_json::json!({
        "protocol_version": CURRENT_PROTOCOL_VERSION,
        "seq": seq,
        "type": "certificate",
        "cert_pem": stub_pem,
        "not_after": not_after_ms,
    })
}

/// In-memory transport test double for event-loop and client-path tests.
pub struct MockTransport {
    inbound: VecDeque<Result<Option<ControllerMessage>, TransportError>>,
    send_log: Vec<ServiceMessage>,
    parked_waker: Option<Waker>,
    ping_interval: Option<Duration>,
    fail_send: bool,
    yielded: bool,
    close_policy: TransportClosePolicy,
}

impl MockTransport {
    pub fn new() -> Self {
        Self {
            inbound: VecDeque::new(),
            send_log: Vec::new(),
            parked_waker: None,
            ping_interval: None,
            fail_send: false,
            yielded: false,
            close_policy: TransportClosePolicy::Reconnect { reason: None },
        }
    }

    pub fn enqueue(&mut self, msg: ControllerMessage) {
        self.inbound.push_back(Ok(Some(msg)));
        if let Some(waker) = self.parked_waker.take() {
            waker.wake();
        }
    }

    pub fn enqueue_close(&mut self) {
        self.inbound.push_back(Ok(None));
        if let Some(waker) = self.parked_waker.take() {
            waker.wake();
        }
    }

    pub fn enqueue_error(&mut self) {
        self.inbound.push_back(Err(TransportError));
        if let Some(waker) = self.parked_waker.take() {
            waker.wake();
        }
    }

    pub fn send_log(&self) -> &[ServiceMessage] {
        &self.send_log
    }

    pub fn ping_interval(&self) -> Option<Duration> {
        self.ping_interval
    }

    pub fn set_ping_interval(&mut self, interval: Option<Duration>) {
        self.ping_interval = interval;
    }

    pub fn set_fail_send(&mut self, fail_send: bool) {
        self.fail_send = fail_send;
    }

    pub fn set_yielded(&mut self, yielded: bool) {
        self.yielded = yielded;
    }

    pub fn set_close_policy(&mut self, policy: TransportClosePolicy) {
        self.close_policy = policy;
    }

    pub fn has_parked_waker(&self) -> bool {
        self.parked_waker.is_some()
    }

    pub fn poll_recv(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<ControllerMessage>, TransportError>> {
        if let Some(item) = self.inbound.pop_front() {
            self.parked_waker.take();
            return Poll::Ready(item);
        }

        self.parked_waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ServiceTransport for MockTransport {
    async fn transport_send(&mut self, msg: ServiceMessage) -> Result<(), TransportError> {
        if self.fail_send {
            return Err(TransportError);
        }
        self.send_log.push(msg);
        Ok(())
    }

    async fn transport_send_best_effort(&mut self, msg: ServiceMessage) {
        self.send_log.push(msg);
    }

    async fn transport_send_auto_paginate(
        &mut self,
        msg: ServiceMessage,
    ) -> Result<(), TransportError> {
        if self.fail_send {
            return Err(TransportError);
        }
        self.send_log.push(msg);
        Ok(())
    }

    async fn transport_recv(&mut self) -> Option<ControllerMessage> {
        poll_fn(|cx| self.poll_recv(cx)).await.unwrap_or_default()
    }

    fn close_policy(&self) -> TransportClosePolicy {
        self.close_policy.clone()
    }

    fn is_yielded(&self) -> bool {
        self.yielded
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };
    use std::task::{Context, Poll, Wake, Waker};
    use std::time::Duration;

    use super::*;

    fn noop_cx() -> Context<'static> {
        Context::from_waker(Waker::noop())
    }

    struct CountingWaker {
        wake_count: Arc<AtomicU32>,
    }

    impl Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.wake_count.fetch_add(1, Ordering::SeqCst);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.wake_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn counting_waker() -> (Arc<AtomicU32>, Waker) {
        let wake_count = Arc::new(AtomicU32::new(0));
        let waker = Waker::from(Arc::new(CountingWaker {
            wake_count: Arc::clone(&wake_count),
        }));
        (wake_count, waker)
    }

    #[test]
    fn new_defaults() {
        let transport = MockTransport::new();

        assert!(transport.send_log().is_empty());
        assert!(!transport.has_parked_waker());
        assert!(matches!(
            transport.close_policy(),
            TransportClosePolicy::Reconnect { reason: None }
        ));
        assert_eq!(transport.ping_interval(), None);
        assert!(!transport.is_yielded());
    }

    #[test]
    fn enqueue_fifo_delivery() {
        let mut transport = MockTransport::new();
        let mut cx = noop_cx();

        transport.enqueue(ControllerMessage::ResetData);
        transport.enqueue(ControllerMessage::Unknown);

        assert!(matches!(
            transport.poll_recv(&mut cx),
            Poll::Ready(Ok(Some(ControllerMessage::ResetData)))
        ));
        assert!(matches!(
            transport.poll_recv(&mut cx),
            Poll::Ready(Ok(Some(ControllerMessage::Unknown)))
        ));
    }

    #[test]
    fn enqueue_close_returns_ok_none() {
        let mut transport = MockTransport::new();
        let mut cx = noop_cx();

        transport.enqueue_close();

        assert!(matches!(
            transport.poll_recv(&mut cx),
            Poll::Ready(Ok(None))
        ));
    }

    #[test]
    fn enqueue_error_returns_err() {
        let mut transport = MockTransport::new();
        let mut cx = noop_cx();

        transport.enqueue_error();

        assert!(matches!(transport.poll_recv(&mut cx), Poll::Ready(Err(_))));
    }

    #[tokio::test]
    async fn send_log_records_all_three_send_methods() {
        let mut transport = MockTransport::new();

        transport
            .transport_send(ServiceMessage::Unknown)
            .await
            .expect("transport_send should record without failing");
        transport
            .transport_send_best_effort(ServiceMessage::Unknown)
            .await;
        transport
            .transport_send_auto_paginate(ServiceMessage::Unknown)
            .await
            .expect("transport_send_auto_paginate should record without failing");

        let send_log = transport.send_log();
        assert_eq!(send_log.len(), 3);
        assert!(matches!(send_log[0], ServiceMessage::Unknown));
        assert!(matches!(send_log[1], ServiceMessage::Unknown));
        assert!(matches!(send_log[2], ServiceMessage::Unknown));
    }

    #[test]
    fn set_close_policy_overrides_default() {
        let mut transport = MockTransport::new();

        transport.set_close_policy(TransportClosePolicy::Shutdown);

        assert!(matches!(
            transport.close_policy(),
            TransportClosePolicy::Shutdown
        ));
    }

    #[test]
    fn set_ping_interval() {
        let mut transport = MockTransport::new();

        transport.set_ping_interval(Some(Duration::from_secs(30)));

        assert_eq!(transport.ping_interval(), Some(Duration::from_secs(30)));
    }

    #[tokio::test]
    async fn set_fail_send_causes_reliable_sends_to_error_without_affecting_best_effort() {
        let mut transport = MockTransport::new();
        transport.set_fail_send(true);

        let reliable = transport.transport_send(ServiceMessage::Unknown).await;
        assert!(reliable.is_err());

        transport
            .transport_send_best_effort(ServiceMessage::Unknown)
            .await;
        assert_eq!(transport.send_log().len(), 1);
        assert!(matches!(transport.send_log()[0], ServiceMessage::Unknown));
    }

    #[test]
    fn set_yielded() {
        let mut transport = MockTransport::new();

        transport.set_yielded(true);

        assert!(transport.is_yielded());
    }

    #[tokio::test]
    async fn async_transport_recv_parks_waker_when_empty() {
        let mut transport = MockTransport::new();
        let poll = futures_util::poll!(transport.transport_recv());
        assert!(matches!(poll, Poll::Pending));

        assert!(transport.has_parked_waker());
    }

    #[test]
    fn wake_on_enqueue_variants() {
        let mut transport = MockTransport::new();
        let (wake_count, waker) = counting_waker();
        let mut cx = Context::from_waker(&waker);

        assert!(matches!(transport.poll_recv(&mut cx), Poll::Pending));
        assert!(transport.has_parked_waker());
        assert_eq!(wake_count.load(Ordering::SeqCst), 0);

        transport.enqueue(ControllerMessage::ResetData);
        assert_eq!(wake_count.load(Ordering::SeqCst), 1);
        assert!(!transport.has_parked_waker());
        assert!(matches!(
            transport.poll_recv(&mut cx),
            Poll::Ready(Ok(Some(ControllerMessage::ResetData)))
        ));
        assert!(!transport.has_parked_waker());

        assert!(matches!(transport.poll_recv(&mut cx), Poll::Pending));
        assert!(transport.has_parked_waker());

        transport.enqueue_close();
        assert_eq!(wake_count.load(Ordering::SeqCst), 2);
        assert!(!transport.has_parked_waker());
        assert!(matches!(
            transport.poll_recv(&mut cx),
            Poll::Ready(Ok(None))
        ));
        assert!(!transport.has_parked_waker());

        assert!(matches!(transport.poll_recv(&mut cx), Poll::Pending));
        assert!(transport.has_parked_waker());

        transport.enqueue_error();
        assert_eq!(wake_count.load(Ordering::SeqCst), 3);
        assert!(!transport.has_parked_waker());
        assert!(matches!(transport.poll_recv(&mut cx), Poll::Ready(Err(_))));
        assert!(!transport.has_parked_waker());
    }

    #[test]
    fn poll_recv_replaces_parked_waker() {
        let mut transport = MockTransport::new();
        let (wake_count_a, waker_a) = counting_waker();
        let (wake_count_b, waker_b) = counting_waker();
        let mut cx_a = Context::from_waker(&waker_a);
        let mut cx_b = Context::from_waker(&waker_b);

        assert!(matches!(transport.poll_recv(&mut cx_a), Poll::Pending));
        assert!(transport.has_parked_waker());
        assert!(matches!(transport.poll_recv(&mut cx_b), Poll::Pending));
        assert!(transport.has_parked_waker());

        transport.enqueue(ControllerMessage::Unknown);

        assert_eq!(wake_count_a.load(Ordering::SeqCst), 0);
        assert_eq!(wake_count_b.load(Ordering::SeqCst), 1);

        assert!(matches!(
            transport.poll_recv(&mut cx_b),
            Poll::Ready(Ok(Some(ControllerMessage::Unknown)))
        ));
        assert!(!transport.has_parked_waker());
    }
}
