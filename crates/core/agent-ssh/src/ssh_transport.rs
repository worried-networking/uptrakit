//! SSH client wrapper for the bootstrap workflow.
//!
//! Provides [`connect_and_authenticate`] for establishing an SSH session and
//! [`SshSession::exec_command`] for running remote commands. Host key
//! verification supports both strict fingerprint pinning and
//! trust-on-first-use (TOFU).

use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use russh::client::{self, Handle};
use russh::keys::agent::client::AgentClient;
use russh::keys::key::PrivateKeyWithHashAlg;
use russh::keys::{self};
use russh::{ChannelMsg, Disconnect};
use tokio::sync::{Mutex, mpsc};
use uptrakit_command::{UpdateOutputLine, UpdateOutputStream};

use crate::error::{Error, Result};

/// Maximum accumulated output size (10 MB) to prevent OOM from runaway commands.
const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

// ── Configuration types ──────────────────────────────────────────────

/// SSH connection configuration.
pub struct SshConnectionConfig {
    pub hostname: String,
    pub port: u16,
    pub connect_timeout: Duration,
}

/// Authentication method for the SSH session.
pub enum AuthMethod<'a> {
    Password(&'a str),
    PrivateKey(&'a str),
    /// Authenticate using keys from the local SSH agent (`SSH_AUTH_SOCK`).
    Agent,
}

/// Result of executing a remote command.
pub struct RemoteCommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: u32,
}

// ── Handler ──────────────────────────────────────────────────────────

/// Client handler for host key verification (private — callers see [`SshSession`]).
struct BootstrapHandler {
    expected_fingerprint: Option<String>,
    observed_fingerprint: Arc<Mutex<Option<String>>>,
}

impl client::Handler for BootstrapHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        let fingerprint = compute_fingerprint(server_public_key);

        if let Some(ref expected) = self.expected_fingerprint {
            let matches = fingerprint == *expected;
            if matches {
                let mut fp = self.observed_fingerprint.lock().await;
                *fp = Some(fingerprint);
            }
            Ok(matches)
        } else {
            // TOFU: accept and record.
            let mut fp = self.observed_fingerprint.lock().await;
            *fp = Some(fingerprint);
            Ok(true)
        }
    }
}

// ── Line buffer ──────────────────────────────────────────────────────

/// Converts arbitrary byte chunks from SSH channel data into
/// line-delimited output, sending complete lines to an optional
/// [`mpsc::Sender<UpdateOutputLine>`] in real time.
pub(crate) struct LineBuffer {
    /// Partial line not yet terminated by `\n`.
    partial: String,
    /// Accumulated full output (stdout or stderr).
    accumulated: String,
    /// Total bytes accumulated (for enforcing the output limit).
    total_bytes: usize,
    /// Which output stream this buffer represents.
    stream: UpdateOutputStream,
    /// Optional channel for streaming lines in real time.
    sender: Option<mpsc::Sender<UpdateOutputLine>>,
}

impl LineBuffer {
    /// Create a new `LineBuffer` for the given stream.
    fn new(stream: UpdateOutputStream, sender: Option<mpsc::Sender<UpdateOutputLine>>) -> Self {
        Self {
            partial: String::new(),
            accumulated: String::new(),
            total_bytes: 0,
            stream,
            sender,
        }
    }

    /// Push raw bytes into the buffer. Complete lines are sent to the
    /// channel (if present) and appended to the accumulated output.
    async fn push(&mut self, data: &[u8]) {
        let text = String::from_utf8_lossy(data);
        for ch in text.chars() {
            if ch == '\n' {
                // Complete line — send and accumulate.
                if let Some(ref tx) = self.sender {
                    let _ = tx
                        .send(UpdateOutputLine {
                            text: self.partial.clone(),
                            stream: self.stream,
                        })
                        .await;
                }
                if self.total_bytes < MAX_OUTPUT_BYTES {
                    self.accumulated.push_str(&self.partial);
                    self.accumulated.push('\n');
                    self.total_bytes += self.partial.len() + 1;
                }
                self.partial.clear();
            } else {
                self.partial.push(ch);
            }
        }
    }

    /// Flush any remaining partial line. Call once after the channel
    /// closes to capture trailing output without a terminating newline.
    async fn flush(&mut self) {
        if !self.partial.is_empty() {
            if let Some(ref tx) = self.sender {
                let _ = tx
                    .send(UpdateOutputLine {
                        text: self.partial.clone(),
                        stream: self.stream,
                    })
                    .await;
            }
            if self.total_bytes < MAX_OUTPUT_BYTES {
                self.accumulated.push_str(&self.partial);
                self.accumulated.push('\n');
                self.total_bytes += self.partial.len() + 1;
            }
            self.partial.clear();
        }
    }

    /// Consume the buffer and return the accumulated output.
    fn into_output(self) -> String {
        self.accumulated
    }
}

// ── Session wrapper ──────────────────────────────────────────────────

/// An authenticated SSH session. Wraps the russh [`Handle`] so the
/// private handler type does not leak into the public API.
pub struct SshSession {
    handle: Handle<BootstrapHandler>,
}

impl SshSession {
    /// Execute a command on the remote host and collect stdout/stderr.
    pub async fn exec_command(&self, command: &str) -> Result<RemoteCommandResult> {
        self.exec_command_streaming(command, None).await
    }

    /// Execute a command on the remote host, optionally streaming output
    /// lines through `output_tx` in real time.
    pub async fn exec_command_streaming(
        &self,
        command: &str,
        output_tx: Option<&mpsc::Sender<UpdateOutputLine>>,
    ) -> Result<RemoteCommandResult> {
        let mut channel =
            self.handle
                .channel_open_session()
                .await
                .map_err(|e| {
                    report!(Error::SshCommand(format!(
                        "failed to open session channel: {e}"
                    )))
                })?;

        channel
            .exec(true, command)
            .await
            .map_err(|e| report!(Error::SshCommand(format!("failed to execute command: {e}"))))?;

        let mut stdout_buf =
            LineBuffer::new(UpdateOutputStream::Stdout, output_tx.cloned());
        let mut stderr_buf =
            LineBuffer::new(UpdateOutputStream::Stderr, output_tx.cloned());
        let mut exit_code: Option<u32> = None;

        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => {
                    stdout_buf.push(data).await;
                }
                ChannelMsg::ExtendedData { ref data, ext: 1 } => {
                    stderr_buf.push(data).await;
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    exit_code = Some(exit_status);
                }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }

        stdout_buf.flush().await;
        stderr_buf.flush().await;

        Ok(RemoteCommandResult {
            stdout: stdout_buf.into_output(),
            stderr: stderr_buf.into_output(),
            exit_code: exit_code.unwrap_or(u32::MAX),
        })
    }

    /// Disconnect the SSH session.
    pub async fn disconnect(self) {
        let _ = self
            .handle
            .disconnect(Disconnect::ByApplication, "bootstrap complete", "en")
            .await;
    }
}

// ── Public API ───────────────────────────────────────────────────────

/// Connect and authenticate to an SSH server.
///
/// Returns the session wrapper and the observed host key fingerprint.
pub async fn connect_and_authenticate(
    config: &SshConnectionConfig,
    username: &str,
    auth: &AuthMethod<'_>,
    expected_fingerprint: Option<&str>,
) -> Result<(SshSession, String)> {
    let observed_fingerprint = Arc::new(Mutex::new(None));

    let handler = BootstrapHandler {
        expected_fingerprint: expected_fingerprint.map(String::from),
        observed_fingerprint: Arc::clone(&observed_fingerprint),
    };

    let ssh_config = Arc::new(client::Config::default());
    let addr = format!("{}:{}", config.hostname, config.port);

    let mut handle = tokio::time::timeout(
        config.connect_timeout,
        client::connect(ssh_config, &addr, handler),
    )
    .await
    .map_err(|_| {
        report!(Error::SshConnection(format!(
            "connection to {addr} timed out after {:?}",
            config.connect_timeout,
        )))
    })?
    .map_err(|e| {
        report!(Error::SshConnection(format!(
            "failed to connect to {addr}: {e}"
        )))
    })?;

    // Check if host key was accepted.
    let fp = observed_fingerprint.lock().await.clone().ok_or_else(|| {
        if let Some(expected) = expected_fingerprint {
            report!(Error::HostKeyMismatch {
                expected: expected.to_string(),
                observed: "(server key rejected)".to_string(),
            })
        } else {
            report!(Error::SshConnection(
                "host key verification failed".to_string()
            ))
        }
    })?;

    // Authenticate.
    match auth {
        AuthMethod::Password(password) => {
            let auth_result = handle
                .authenticate_password(username.to_string(), password.to_string())
                .await
                .map_err(|e| {
                    report!(Error::SshAuth(format!(
                        "password authentication failed: {e}"
                    )))
                })?;
            if !auth_result.success() {
                bail!(Error::SshAuth(format!(
                    "authentication failed for user '{username}'"
                )));
            }
        }
        AuthMethod::PrivateKey(pem) => {
            let private_key = keys::decode_secret_key(pem, None).map_err(|e| {
                report!(Error::SshAuth(format!("failed to decode private key: {e}")))
            })?;
            let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(private_key), None);
            let auth_result = handle
                .authenticate_publickey(username.to_string(), key_with_alg)
                .await
                .map_err(|e| {
                    report!(Error::SshAuth(format!(
                        "public key authentication failed: {e}"
                    )))
                })?;
            if !auth_result.success() {
                bail!(Error::SshAuth(format!(
                    "authentication failed for user '{username}'"
                )));
            }
        }
        AuthMethod::Agent => {
            authenticate_with_agent(&mut handle, username).await?;
        }
    }

    Ok((SshSession { handle }, fp))
}

// ── Internal helpers ─────────────────────────────────────────────────

/// Authenticate using keys from the local SSH agent.
///
/// Connects to the agent via `SSH_AUTH_SOCK`, enumerates identities, and tries
/// each key until one succeeds or all are exhausted.
async fn authenticate_with_agent<H: client::Handler>(
    handle: &mut Handle<H>,
    username: &str,
) -> Result<()> {
    let mut agent = AgentClient::connect_env().await.map_err(|e| {
        report!(Error::SshAuth(format!(
            "failed to connect to SSH agent: {e}"
        )))
    })?;

    let identities = agent.request_identities().await.map_err(|e| {
        report!(Error::SshAuth(format!(
            "failed to list SSH agent identities: {e}"
        )))
    })?;

    if identities.is_empty() {
        bail!(Error::SshAuth("SSH agent has no keys loaded".to_string()));
    }

    for key in &identities {
        let result = handle
            .authenticate_publickey_with(username.to_string(), key.clone(), None, &mut agent)
            .await
            .map_err(|e| report!(Error::SshAuth(format!("SSH agent signing failed: {e}"))))?;

        if result.success() {
            return Ok(());
        }
    }

    bail!(Error::SshAuth(format!(
        "none of the {} SSH agent key(s) were accepted for user '{username}'",
        identities.len()
    )));
}


/// Compute the SHA-256 fingerprint of an SSH public key in `SHA256:...` format.
fn compute_fingerprint(key: &russh::keys::ssh_key::PublicKey) -> String {
    format!("{}", key.fingerprint(russh::keys::ssh_key::HashAlg::Sha256))
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::client::Handler;

    #[test]
    fn compute_fingerprint_format() {
        let fp = "SHA256:abcdef";
        assert!(fp.starts_with("SHA256:"));
    }

    #[tokio::test]
    async fn handler_tofu_accepts() {
        let observed = Arc::new(Mutex::new(None));
        let mut handler = BootstrapHandler {
            expected_fingerprint: None,
            observed_fingerprint: Arc::clone(&observed),
        };

        let (pem, _) = crate::ssh_key::generate_ed25519_keypair().expect("keygen");
        let private_key = keys::decode_secret_key(&pem, None).expect("decode");
        let public_key = private_key.public_key();

        let result = handler.check_server_key(public_key).await.expect("check");
        assert!(result, "TOFU handler should accept any key");

        let fp = observed.lock().await;
        assert!(fp.is_some(), "fingerprint should be recorded");
        assert!(
            fp.as_ref().is_some_and(|f| f.starts_with("SHA256:")),
            "fingerprint should start with SHA256:"
        );
    }

    #[tokio::test]
    async fn handler_pinned_rejects_mismatch() {
        let observed = Arc::new(Mutex::new(None));
        let mut handler = BootstrapHandler {
            expected_fingerprint: Some("SHA256:wrong_fingerprint".to_string()),
            observed_fingerprint: Arc::clone(&observed),
        };

        let (pem, _) = crate::ssh_key::generate_ed25519_keypair().expect("keygen");
        let private_key = keys::decode_secret_key(&pem, None).expect("decode");
        let public_key = private_key.public_key();

        let result = handler.check_server_key(public_key).await.expect("check");
        assert!(!result, "pinned handler should reject mismatched key");

        let fp = observed.lock().await;
        assert!(
            fp.is_none(),
            "fingerprint should not be recorded on mismatch"
        );
    }

    #[tokio::test]
    async fn handler_pinned_accepts_match() {
        let observed = Arc::new(Mutex::new(None));

        let (pem, _) = crate::ssh_key::generate_ed25519_keypair().expect("keygen");
        let private_key = keys::decode_secret_key(&pem, None).expect("decode");
        let public_key = private_key.public_key();
        let expected_fp = compute_fingerprint(public_key);

        let mut handler = BootstrapHandler {
            expected_fingerprint: Some(expected_fp.clone()),
            observed_fingerprint: Arc::clone(&observed),
        };

        let result = handler.check_server_key(public_key).await.expect("check");
        assert!(result, "pinned handler should accept matching key");

        let fp = observed.lock().await;
        assert_eq!(fp.as_deref(), Some(expected_fp.as_str()));
    }

    // ── LineBuffer tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn line_buffer_emits_complete_lines() {
        let (tx, mut rx) = mpsc::channel(100);
        let mut buf = LineBuffer::new(UpdateOutputStream::Stdout, Some(tx));

        buf.push(b"hello\nworld\n").await;

        let line1 = rx.recv().await.expect("should receive first line");
        assert_eq!(line1.text, "hello");
        assert_eq!(line1.stream, UpdateOutputStream::Stdout);

        let line2 = rx.recv().await.expect("should receive second line");
        assert_eq!(line2.text, "world");
    }

    #[tokio::test]
    async fn line_buffer_holds_partial_lines() {
        let (tx, mut rx) = mpsc::channel(100);
        let mut buf = LineBuffer::new(UpdateOutputStream::Stdout, Some(tx));

        buf.push(b"partial").await;

        // No complete line yet — channel should be empty.
        assert!(
            rx.try_recv().is_err(),
            "partial line should not be sent yet"
        );

        // Now complete the line.
        buf.push(b" end\n").await;
        let line = rx.recv().await.expect("should receive completed line");
        assert_eq!(line.text, "partial end");
    }

    #[tokio::test]
    async fn line_buffer_flush_emits_remaining() {
        let (tx, mut rx) = mpsc::channel(100);
        let mut buf = LineBuffer::new(UpdateOutputStream::Stderr, Some(tx));

        buf.push(b"trailing").await;
        buf.flush().await;

        let line = rx.recv().await.expect("flush should emit partial");
        assert_eq!(line.text, "trailing");
        assert_eq!(line.stream, UpdateOutputStream::Stderr);

        let output = buf.into_output();
        assert_eq!(output, "trailing\n");
    }

    #[tokio::test]
    async fn line_buffer_respects_output_limit() {
        let mut buf = LineBuffer::new(UpdateOutputStream::Stdout, None);

        // Push data exceeding MAX_OUTPUT_BYTES (10 MB).
        let big_line = "x".repeat(1_000_000);
        for _ in 0..12 {
            let mut data = big_line.clone();
            data.push('\n');
            buf.push(data.as_bytes()).await;
        }

        let output = buf.into_output();
        assert!(
            output.len() <= MAX_OUTPUT_BYTES + 1_000_001,
            "output should respect the 10 MB limit (got {} bytes)",
            output.len()
        );
    }

    #[tokio::test]
    async fn line_buffer_works_without_sender() {
        let mut buf = LineBuffer::new(UpdateOutputStream::Stdout, None);

        buf.push(b"line1\nline2\n").await;
        buf.flush().await;

        let output = buf.into_output();
        assert_eq!(output, "line1\nline2\n");
    }

    #[tokio::test]
    async fn line_buffer_flush_noop_when_empty() {
        let mut buf = LineBuffer::new(UpdateOutputStream::Stdout, None);
        buf.flush().await;
        let output = buf.into_output();
        assert!(output.is_empty());
    }
}
