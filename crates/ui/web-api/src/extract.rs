use std::net::IpAddr;

/// The resolved client IP address.
/// Set by the `resolve_ip` middleware.
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);

/// The trusted proxy's IP address.
/// Only present when the peer is a known trusted proxy.
/// Set by the `resolve_ip` middleware.
#[derive(Debug, Clone, Copy)]
pub struct ProxyIp(pub IpAddr);

/// The verified agent identity from a client TLS certificate.
/// Injected by the mTLS acceptor when the peer presents a valid cert
/// signed by the internal CA, with a parseable UUID as CN.
#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub agent_id: uuid::Uuid,
    pub cert_serial: String,
}
