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
        self.send_log.push(msg);
        Ok(())
    }

    async fn transport_recv(&mut self) -> Option<ControllerMessage> {
        match poll_fn(|cx| self.poll_recv(cx)).await {
            Ok(msg) => msg,
            Err(_) => None,
        }
    }

    fn close_policy(&self) -> TransportClosePolicy {
        self.close_policy.clone()
    }

    fn is_yielded(&self) -> bool {
        self.yielded
    }
}
