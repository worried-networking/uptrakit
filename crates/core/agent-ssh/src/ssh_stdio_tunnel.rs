//! SSH-backed [`StdioTunnel`] implementation.
//!
//! Wraps a russh [`ChannelStream`] in a newtype that implements
//! [`AsyncRead`], [`AsyncWrite`], and [`StdioTunnel`], allowing the
//! Docker plugin to run `docker system dial-stdio` over an existing
//! SSH session.

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use uptrakit_command::StdioTunnel;

/// Bidirectional byte stream over an SSH channel.
///
/// Created by [`crate::ssh_executor::SshCommandExecutor::open_stdio_tunnel`],
/// which opens a channel via [`crate::ssh_transport::SshSession::open_channel_for_command`]
/// and converts it to a stream.
pub(crate) struct SshStdioTunnel(russh::ChannelStream<russh::client::Msg>);

impl SshStdioTunnel {
    /// Wrap a russh channel as a stdio tunnel.
    ///
    /// Calls [`russh::Channel::into_stream`] to obtain the underlying
    /// bidirectional byte stream.
    pub(crate) fn new(channel: russh::Channel<russh::client::Msg>) -> Self {
        Self(channel.into_stream())
    }
}

impl AsyncRead for SshStdioTunnel {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for SshStdioTunnel {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl StdioTunnel for SshStdioTunnel {}
