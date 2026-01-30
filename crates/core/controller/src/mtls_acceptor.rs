use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::http::Request;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::server::TlsStream;
use tower::Service;
use uptrakit_web_api::extract::AgentIdentity;

use axum_server::accept::Accept;
use axum_server::tls_rustls::RustlsAcceptor;

/// Custom acceptor wrapping `RustlsAcceptor`.
/// After TLS handshake, extracts peer cert CN as `AgentIdentity`.
#[derive(Clone)]
pub struct MtlsAcceptor {
    inner: RustlsAcceptor,
}

impl MtlsAcceptor {
    pub fn new(inner: RustlsAcceptor) -> Self {
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
pub struct MtlsAcceptFuture<I, S>
where
    I: AsyncRead + AsyncWrite + Unpin,
{
    #[allow(clippy::type_complexity)]
    inner: Pin<Box<dyn Future<Output = io::Result<(TlsStream<I>, S)>> + Send>>,
}

impl<I, S> Future for MtlsAcceptFuture<I, S>
where
    I: AsyncRead + AsyncWrite + Unpin,
{
    type Output = io::Result<(TlsStream<I>, MtlsService<S>)>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.as_mut().poll(cx) {
            Poll::Ready(Ok((tls_stream, service))) => {
                let agent_identity = extract_agent_identity(&tls_stream);
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

/// Service wrapper that injects `AgentIdentity` into request extensions.
#[derive(Clone)]
pub struct MtlsService<S> {
    inner: S,
    agent_identity: Option<AgentIdentity>,
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

/// Extract agent UUID from peer certificate CN.
fn extract_agent_identity<I>(stream: &TlsStream<I>) -> Option<AgentIdentity>
where
    I: AsyncRead + AsyncWrite + Unpin,
{
    let (_, conn) = stream.get_ref();
    let certs = conn.peer_certificates()?;
    let leaf = certs.first()?;
    let (_, cert) = x509_parser::parse_x509_certificate(leaf.as_ref()).ok()?;
    let cn = cert.subject().iter_common_name().next()?.as_str().ok()?;
    let agent_id = uuid::Uuid::parse_str(cn).ok()?;
    let cert_serial = cert.raw_serial_as_string();
    Some(AgentIdentity {
        agent_id,
        cert_serial,
    })
}
