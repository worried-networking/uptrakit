#![cfg(any(test, feature = "test-support"))]

use std::collections::VecDeque;
use std::future::poll_fn;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use async_trait::async_trait;
use uptrakit_internal_wire::{
    ControllerMessage, ServiceMessage, ServiceTransport, TransportClosePolicy, TransportError,
};

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
