use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::http::Request;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::server::TlsStream;
use tower::Service;

use axum_server::accept::Accept;
use axum_server::tls_rustls::RustlsAcceptor;

use uptrakit_web_api::extract::ServiceIdentity;

/// Pinned, boxed future returned by the inner TLS acceptor.
type BoxedAcceptFuture<I, S> = Pin<Box<dyn Future<Output = io::Result<(TlsStream<I>, S)>> + Send>>;

/// Custom TLS acceptor that supports both enrolled and unenrolled agents on a
/// single listener.
///
/// # Dual-authentication model
///
/// The underlying rustls config is built with `.allow_unauthenticated()` so
/// that agents without a client certificate can still complete the TLS
/// handshake (they have not yet received one during initial enrollment).
///
/// After the handshake this acceptor inspects the peer certificates and injects
/// an `Option<ServiceIdentity>` into every request's extensions:
///
/// | Peer cert present? | `ServiceIdentity` in extensions | Authentication path       |
/// |--------------------|----------------------------------|---------------------------|
/// | Yes                | `Some(identity)`                 | mTLS — cert-based         |
/// | No                 | `None`                           | Enrollment secret bearer  |
///
/// Route handlers use the presence or absence of `ServiceIdentity` to enforce
/// their own authentication requirements.  Enrollment-only routes accept
/// `None`; all post-enrollment agent routes require `Some`.
#[derive(Clone)]
pub(crate) struct MtlsAcceptor {
    inner: RustlsAcceptor,
}

impl MtlsAcceptor {
    pub(crate) fn new(inner: RustlsAcceptor) -> Self {
        Self { inner }
    }
}

impl<I, S> Accept<I, S> for MtlsAcceptor
where
    I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    S: Send + 'static,
{
    type Stream = TlsStream<I>;
    type Service = MtlsService<S>;
    type Future = MtlsAcceptFuture<I, S>;

    fn accept(&self, stream: I, service: S) -> Self::Future {
        MtlsAcceptFuture {
            inner: Box::pin(self.inner.accept(stream, service)),
        }
    }
}

/// Future that completes TLS handshake and extracts agent identity.
pub(crate) struct MtlsAcceptFuture<I, S>
where
    I: AsyncRead + AsyncWrite + Unpin,
{
    inner: BoxedAcceptFuture<I, S>,
}

impl<I, S> Future for MtlsAcceptFuture<I, S>
where
    I: AsyncRead + AsyncWrite + Unpin,
{
    type Output = io::Result<(TlsStream<I>, MtlsService<S>)>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.as_mut().poll(cx) {
            Poll::Ready(Ok((tls_stream, service))) => {
                let agent_identity = extract_service_identity(&tls_stream);
                Poll::Ready(Ok((
                    tls_stream,
                    MtlsService {
                        inner: service,
                        agent_identity,
                    },
                )))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Service wrapper that injects `ServiceIdentity` into request extensions.
#[derive(Clone)]
pub(crate) struct MtlsService<S> {
    inner: S,
    agent_identity: Option<ServiceIdentity>,
}

impl<S, B> Service<Request<B>> for MtlsService<S>
where
    S: Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<B>) -> Self::Future {
        if let Some(ref identity) = self.agent_identity {
            req.extensions_mut().insert(identity.clone());
        }
        self.inner.call(req)
    }
}

/// Extract service UUID from peer certificate CN.
fn extract_service_identity<I>(stream: &TlsStream<I>) -> Option<ServiceIdentity>
where
    I: AsyncRead + AsyncWrite + Unpin,
{
    let (_, conn) = stream.get_ref();
    let certs = conn.peer_certificates()?;
    let leaf = certs.first()?;
    uptrakit_web_api::extract::service_identity_from_der(leaf.as_ref())
}
