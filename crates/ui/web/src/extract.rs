/// The transport protocol used for the connection.
///
/// Injected as a request extension by the server layer so that middleware
/// can distinguish TLS connections from plain-text ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Plain,
    Tls,
}
