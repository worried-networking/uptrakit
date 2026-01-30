use std::net::IpAddr;

/// The transport protocol used for the connection.
///
/// Injected as a request extension by the server layer so that middleware
/// can distinguish TLS connections from plain-text ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Plain,
    Tls,
}

/// The resolved client IP address.
/// Set by the `resolve_ip` middleware.
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);

/// The trusted proxy's IP address.
/// Only present when the peer is a known trusted proxy.
/// Set by the `resolve_ip` middleware.
#[derive(Debug, Clone, Copy)]
pub struct ProxyIp(pub IpAddr);
