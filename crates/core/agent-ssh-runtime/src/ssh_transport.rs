//! SSH client wrapper for the bootstrap workflow.
//!
//! Provides [`connect_and_authenticate`] for establishing an SSH session and
//! [`SshSession::exec_command`] for running remote commands. Host key
//! verification supports both strict fingerprint pinning and
//! trust-on-first-use (TOFU).

use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use russh::client::{self, AuthResult, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::agent::client::AgentClient;
use russh::keys::key::PrivateKeyWithHashAlg;
use russh::keys::ssh_key::Algorithm;
use russh::keys::ssh_key::HashAlg;
use russh::keys::{self};
use russh::{ChannelMsg, Disconnect, MethodKind};
use tokio::sync::{Mutex, mpsc};
use uptrakit_command::{DEFAULT_COMMAND_TIMEOUT, UpdateOutputLine};
use uptrakit_shared_types::OutputStreamType;

use crate::error::{Error, Result};

// ── Raw exec error ────────────────────────────────────────────────────

/// Error from a raw SSH exec or SFTP operation.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SshExecError {
    #[error("SSH exec failed: {0}")]
    Exec(String),
    #[error("SSH exec timed out")]
    TimedOut,
}

/// Maximum accumulated output size (10 MB) to prevent OOM from runaway commands.
const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Timeout for the pre-connect SSH banner peek. Tight bound so a slow peek
/// does not extend the bootstrap latency budget — fall through to the shell
/// probe if the server is sluggish.
const BANNER_PEEK_TIMEOUT: Duration = Duration::from_secs(2);

/// SSH client keepalive interval: send keep-alive messages every 15 seconds.
/// Detects dead/zombie SSH peers on the agent side.
const SSH_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// SSH client keepalive max: allow up to 4 consecutive keepalive messages
/// without a response before closing the connection.
const SSH_KEEPALIVE_MAX: usize = 4;

/// Header whose value is a bearer credential and must never reach a log line.
const SENSITIVE_HEADER: &str = "authorization:";

/// Replacement written in place of a redacted header value.
const REDACTION: &str = " <redacted>";

/// Mask credential-bearing header values before a command string is traced.
///
/// A remote command is handed to the peer's shell as a single string, so a
/// caller that must present a bearer credential — the PVE on-node token proof
/// in `uptrakit_plugin_infrastructure_proxmox::pve_setup::prove_token_on_node`
/// — has no way to keep the secret out of `command`. Redaction therefore
/// happens here, at the only place the string is logged, instead of relying on
/// every present and future call site to police itself.
///
/// The value runs to the quote that closes the header argument (or to the end
/// of the line when the argument is unquoted), which is what both `curl -H`
/// forms produce.
fn redact_for_log(command: &str) -> std::borrow::Cow<'_, str> {
    if !command.to_ascii_lowercase().contains(SENSITIVE_HEADER) {
        return std::borrow::Cow::Borrowed(command);
    }
    let header_len = SENSITIVE_HEADER.chars().count();
    let mut out = String::with_capacity(command.len());
    let mut tail = String::with_capacity(SENSITIVE_HEADER.len());
    let mut skipping = false;
    for ch in command.chars() {
        if skipping {
            // The closing quote (or newline) ends the header argument; it is
            // structure, not secret, so it is kept.
            if matches!(ch, '\'' | '"' | '\n') {
                skipping = false;
                out.push(ch);
            }
            continue;
        }
        out.push(ch);
        tail.push(ch.to_ascii_lowercase());
        while tail.chars().count() > header_len {
            tail.remove(0);
        }
        if tail == SENSITIVE_HEADER {
            out.push_str(REDACTION);
            tail.clear();
            skipping = true;
        }
    }
    std::borrow::Cow::Owned(out)
}

// ── Configuration types ──────────────────────────────────────────────

/// Builds an SSH client configuration with keepalive enabled.
///
/// Keepalive sends periodic messages every [`SSH_KEEPALIVE_INTERVAL`] seconds
/// and closes the connection after [`SSH_KEEPALIVE_MAX`] consecutive
/// unanswered messages. This detects dead/zombie SSH peers on the agent side.
///
/// Note: [`inactivity_timeout`](client::Config::inactivity_timeout) is left as
/// `None` to allow graceful detection via keepalive instead of a hard timeout.
fn build_ssh_client_config() -> client::Config {
    client::Config {
        keepalive_interval: Some(SSH_KEEPALIVE_INTERVAL),
        keepalive_max: SSH_KEEPALIVE_MAX,
        ..Default::default()
    }
}

/// SSH connection configuration.
pub(crate) struct SshConnectionConfig {
    pub hostname: String,
    pub port: u16,
    pub connect_timeout: Duration,
}

/// Authentication method for the SSH session.
pub(crate) enum AuthMethod<'a> {
    Password(&'a str),
    PrivateKey(&'a str),
    /// Authenticate using keys from the local SSH agent (`SSH_AUTH_SOCK`).
    Agent,
}

/// Result of executing a remote command.
#[derive(Debug)]
pub(crate) struct RemoteCommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: u32,
}

// ── Handler ──────────────────────────────────────────────────────────

/// Client handler for host key verification (private — callers see [`SshSession`]).
struct BootstrapHandler {
    expected_fingerprint: Option<String>,
    observed_fingerprint: Arc<Mutex<Option<String>>>,
    hostname: String,
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
                tracing::debug!(
                    hostname = %self.hostname,
                    fingerprint = %fingerprint,
                    "host key fingerprint verified"
                );
                let mut fp = self.observed_fingerprint.lock().await;
                *fp = Some(fingerprint);
            }
            Ok(matches)
        } else {
            // TOFU: accept and record.
            tracing::info!(
                hostname = %self.hostname,
                fingerprint = %fingerprint,
                "accepting host key via trust-on-first-use (TOFU)"
            );
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
    /// Whether accumulated output has been truncated due to exceeding the limit.
    truncated: bool,
    /// Which output stream this buffer represents.
    stream: OutputStreamType,
    /// Optional channel for streaming lines in real time.
    sender: Option<mpsc::Sender<UpdateOutputLine>>,
}

impl LineBuffer {
    /// Create a new `LineBuffer` for the given stream.
    fn new(stream: OutputStreamType, sender: Option<mpsc::Sender<UpdateOutputLine>>) -> Self {
        Self {
            partial: String::new(),
            accumulated: String::new(),
            total_bytes: 0,
            truncated: false,
            stream,
            sender,
        }
    }

    /// Push raw bytes into the buffer. Complete lines are sent to the
    /// channel (if present) and appended to the accumulated output.
    fn push(&mut self, data: &[u8]) {
        let text = String::from_utf8_lossy(data);
        for ch in text.chars() {
            if ch == '\n' {
                // Complete line — send and accumulate.
                if let Some(ref tx) = self.sender {
                    // Best-effort live streaming: drop the line when the channel
                    // is full (consumer >= capacity behind) or closed (subscriber
                    // gone). The authoritative full output is still accumulated
                    // below and returned in RemoteCommandResult, so a dropped live
                    // line loses nothing durable — and a non-blocking send can
                    // never stall the read loop or defeat the command deadline.
                    // Mirrors the interactive PTY path.
                    #[expect(
                        clippy::let_underscore_must_use,
                        reason = "try_send failures (Full or Closed) are expected — a full channel means the consumer is behind and a closed one means the subscriber disconnected; recovery is impossible and the loss is harmless because the accumulated output is still returned"
                    )]
                    let _ = tx.try_send(UpdateOutputLine {
                        text: self.partial.clone(),
                        stream: self.stream,
                    });
                }
                if self.total_bytes < MAX_OUTPUT_BYTES {
                    self.accumulated.push_str(&self.partial);
                    self.accumulated.push('\n');
                    self.total_bytes += self.partial.len() + 1;
                } else if !self.truncated {
                    self.truncated = true;
                    tracing::warn!(
                        stream = ?self.stream,
                        "output exceeded {MAX_OUTPUT_BYTES} bytes, truncating accumulation"
                    );
                    self.accumulated.push_str("\n[output truncated at 10 MB]\n");
                }
                self.partial.clear();
            } else {
                self.partial.push(ch);
            }
        }
    }

    /// Flush any remaining partial line. Call once after the channel
    /// closes to capture trailing output without a terminating newline.
    fn flush(&mut self) {
        if !self.partial.is_empty() {
            if let Some(ref tx) = self.sender {
                // See `push` above for why this is a non-blocking, best-effort send.
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "try_send failures (Full or Closed) are expected — a full channel means the consumer is behind and a closed one means the subscriber disconnected; recovery is impossible and the loss is harmless because the accumulated output is still returned"
                )]
                let _ = tx.try_send(UpdateOutputLine {
                    text: self.partial.clone(),
                    stream: self.stream,
                });
            }
            if self.total_bytes < MAX_OUTPUT_BYTES {
                self.accumulated.push_str(&self.partial);
                self.accumulated.push('\n');
                self.total_bytes += self.partial.len() + 1;
            } else if !self.truncated {
                self.truncated = true;
                tracing::warn!(
                    stream = ?self.stream,
                    "output exceeded {MAX_OUTPUT_BYTES} bytes, truncating accumulation"
                );
                self.accumulated.push_str("\n[output truncated at 10 MB]\n");
            }
            self.partial.clear();
        }
    }

    /// Consume the buffer and return the accumulated output.
    fn into_output(self) -> String {
        self.accumulated
    }
}

/// Bound a channel-setup await by the same fixed deadline as the read loop.
async fn setup_bounded<T>(
    deadline: Option<(Duration, tokio::time::Instant)>,
    fut: impl std::future::Future<Output = T>,
) -> Result<T> {
    match deadline {
        Some((d, at)) => tokio::time::timeout_at(at, fut)
            .await
            .map_err(|_elapsed| report!(Error::SshCommandTimedOut(d))),
        None => Ok(fut.await),
    }
}

/// Whether the read loop may terminate before `Close`: the server has
/// signalled both stream end (`Eof`) and command completion (`ExitStatus`),
/// in either arrival order. `ExitStatus` alone is NOT sufficient — output
/// may still be in flight; a server that never sends `Eof`/`Close` after
/// `ExitStatus` is bounded by the command deadline instead.
fn remote_command_finished(eof_seen: bool, exit_code: Option<u32>) -> bool {
    eof_seen && exit_code.is_some()
}

// ── Session wrapper ──────────────────────────────────────────────────

/// An authenticated SSH session. Wraps the russh [`Handle`] so the
/// private handler type does not leak into the public API.
pub(crate) struct SshSession {
    handle: Handle<BootstrapHandler>,
    pub(crate) hostname: String,
    /// Server software string from the SSH identification banner
    /// (the portion after `SSH-2.0-` / `SSH-1.99-`). `None` when the pre-connect
    /// banner peek failed. Used for OS feature detection — never for security.
    server_software: Option<String>,
}

impl SshSession {
    /// Server software string captured from the SSH identification banner.
    pub(crate) fn server_software(&self) -> Option<&str> {
        self.server_software.as_deref()
    }
    /// Open an SSH channel for the given command and return it before
    /// consuming any output.
    ///
    /// The caller owns the raw [`russh::Channel`] and can call
    /// `.into_stream()` to obtain a bidirectional `ChannelStream` for
    /// byte-level I/O (used by [`crate::ssh_stdio_tunnel::SshStdioTunnel`]).
    pub(crate) async fn open_channel_for_command(
        &self,
        command: &str,
    ) -> Result<russh::Channel<russh::client::Msg>> {
        tracing::trace!(hostname = %self.hostname, command = %redact_for_log(command), "opening SSH channel for command");
        let channel = self.handle.channel_open_session().await.map_err(|e| {
            report!(Error::SshCommand(format!(
                "failed to open session channel: {e}"
            )))
        })?;

        channel
            .exec(true, command)
            .await
            .map_err(|e| report!(Error::SshCommand(format!("failed to execute command: {e}"))))?;

        Ok(channel)
    }

    /// Execute a raw command on the remote host, returning combined stdout + stderr.
    ///
    /// This is a low-level helper that makes no POSIX assumptions about the remote
    /// shell. `timeout: None` now means the default deadline
    /// ([`DEFAULT_COMMAND_TIMEOUT`]) applies — `None` no longer means unbounded.
    pub(crate) async fn exec_raw(
        &self,
        cmd: &str,
        timeout: Option<std::time::Duration>,
    ) -> std::result::Result<String, SshExecError> {
        let result = match self
            .exec_command_with_timeout(cmd, timeout.or(Some(DEFAULT_COMMAND_TIMEOUT)))
            .await
        {
            Ok(result) => result,
            Err(e) if matches!(e.current_context(), Error::SshCommandTimedOut(_)) => {
                return Err(SshExecError::TimedOut);
            }
            Err(e) => return Err(SshExecError::Exec(e.to_string())),
        };
        let mut out = result.stdout;
        out.push_str(&result.stderr);
        Ok(out)
    }

    /// Execute a command and collect stdout/stderr, bounded at
    /// [`DEFAULT_COMMAND_TIMEOUT`]. Use [`Self::exec_command_with_timeout`]
    /// for a caller-chosen deadline.
    pub(crate) async fn exec_command(&self, command: &str) -> Result<RemoteCommandResult> {
        self.exec_command_with_timeout(command, Some(DEFAULT_COMMAND_TIMEOUT))
            .await
    }

    /// Execute a command with an explicit deadline (`None` = unbounded; only
    /// for callers that provably bound execution themselves).
    pub(crate) async fn exec_command_with_timeout(
        &self,
        command: &str,
        timeout: Option<Duration>,
    ) -> Result<RemoteCommandResult> {
        self.exec_command_streaming(command, None, timeout).await
    }

    /// Execute a command on the remote host, optionally streaming output
    /// lines through `output_tx` in real time.
    ///
    /// `timeout` bounds the whole call — channel setup and the read loop —
    /// with a single fixed deadline. `None` means unbounded; only callers
    /// that provably bound execution themselves (e.g. via an outer timeout)
    /// should pass `None`.
    pub(crate) async fn exec_command_streaming(
        &self,
        command: &str,
        output_tx: Option<&mpsc::Sender<UpdateOutputLine>>,
        timeout: Option<Duration>,
    ) -> Result<RemoteCommandResult> {
        tracing::trace!(hostname = %self.hostname, command = %redact_for_log(command), "executing SSH command");

        // Fixed instant: the deadline covers the whole command — including
        // channel setup — not each read.
        let deadline = timeout.map(|d| (d, tokio::time::Instant::now() + d));

        let mut channel = setup_bounded(deadline, self.handle.channel_open_session())
            .await?
            .map_err(|e| {
                report!(Error::SshCommand(format!(
                    "failed to open session channel: {e}"
                )))
            })?;

        setup_bounded(deadline, channel.exec(true, command))
            .await?
            .map_err(|e| report!(Error::SshCommand(format!("failed to execute command: {e}"))))?;

        // Close stdin so remote scripts cannot block on `read`.
        setup_bounded(deadline, channel.eof()).await?.map_err(|e| {
            report!(Error::SshCommand(format!(
                "failed to close channel stdin: {e}"
            )))
        })?;

        let mut stdout_buf = LineBuffer::new(OutputStreamType::Stdout, output_tx.cloned());
        let mut stderr_buf = LineBuffer::new(OutputStreamType::Stderr, output_tx.cloned());
        let mut exit_code: Option<u32> = None;
        let mut eof_seen = false;
        let mut exit_signal: Option<String> = None;

        // Build the deadline timer ONCE and reuse it across iterations —
        // recreating a `Sleep` inside `select!` on every message churns the
        // timer wheel (tokio's select! docs call this out). With no timeout
        // the branch is disabled and the placeholder timer is never polled.
        let sleep =
            tokio::time::sleep_until(deadline.map_or_else(tokio::time::Instant::now, |(_, at)| at));
        tokio::pin!(sleep);
        loop {
            let maybe = tokio::select! {
                m = channel.wait() => Some(m),
                () = &mut sleep, if deadline.is_some() => None,
            };
            let Some(msg) = maybe else {
                // Deadline fired. Best-effort close; the server may already be gone.
                if let Err(e) = channel.close().await {
                    tracing::trace!(error = %e, "channel close after deadline failed");
                }
                let dur = deadline.map_or(Duration::ZERO, |(d, _)| d);
                tracing::warn!(timeout = ?dur, "remote command exceeded its deadline; channel closed");
                stdout_buf.flush();
                stderr_buf.flush();
                bail!(Error::SshCommandTimedOut(dur));
            };
            let Some(msg) = msg else { break };
            match msg {
                ChannelMsg::Data { ref data } => {
                    stdout_buf.push(data);
                }
                ChannelMsg::ExtendedData { ref data, ext: 1 } => {
                    stderr_buf.push(data);
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    exit_code = Some(exit_status);
                    if remote_command_finished(eof_seen, exit_code) {
                        break;
                    }
                }
                ChannelMsg::Eof => {
                    eof_seen = true;
                    if remote_command_finished(eof_seen, exit_code) {
                        break;
                    }
                }
                ChannelMsg::ExitSignal { signal_name, .. } => {
                    exit_signal = Some(format!("{signal_name:?}"));
                }
                ChannelMsg::Close => break,
                _ => {}
            }
        }

        stdout_buf.flush();
        stderr_buf.flush();

        let mut stderr = stderr_buf.into_output();
        if let Some(sig) = exit_signal {
            tracing::warn!(signal = %sig, "remote process terminated by signal");
            stderr.push_str(&format!("\n[remote process terminated by signal {sig}]"));
        }

        Ok(RemoteCommandResult {
            stdout: stdout_buf.into_output(),
            stderr,
            exit_code: exit_code.unwrap_or(u32::MAX),
        })
    }

    /// Execute a command interactively with a PTY on the remote host.
    ///
    /// Allocates a remote PTY via `request_pty`, executes the command, and
    /// returns an [`InteractiveHandle`] with channels for stdin forwarding,
    /// signal delivery, and completion awaiting.
    ///
    /// Unlike [`exec_command_streaming`], stdin is kept open so the remote
    /// process can read user input. Signals are delivered by writing the
    /// corresponding control character to the PTY (e.g., `\x03` for SIGINT).
    #[cfg(feature = "interactive")]
    pub(crate) async fn exec_command_interactive(
        &self,
        command: &str,
        output_tx: &mpsc::Sender<uptrakit_command::UpdateOutputLine>,
        timeout: Option<Duration>,
    ) -> Result<uptrakit_command::executor::InteractiveHandle> {
        use uptrakit_command::executor::InteractiveHandle;

        tracing::debug!(
            hostname = %self.hostname,
            command = %redact_for_log(command),
            "executing interactive SSH command with PTY"
        );

        let mut channel = self.handle.channel_open_session().await.map_err(|e| {
            report!(Error::SshCommand(format!(
                "failed to open session channel: {e}"
            )))
        })?;

        // Request a PTY on the remote side.
        channel
            .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
            .await
            .map_err(|e| report!(Error::SshCommand(format!("failed to request PTY: {e}"))))?;

        channel
            .exec(true, command)
            .await
            .map_err(|e| report!(Error::SshCommand(format!("failed to execute command: {e}"))))?;

        // Do NOT call channel.eof() — keep stdin open for forwarding.

        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(64);
        let (signal_tx, mut signal_rx) = mpsc::channel::<i32>(8);
        let (attention_tx, attention_rx) = mpsc::channel::<()>(4);
        let output_tx_clone = output_tx.clone();

        let completion = tokio::spawn(async move {
            Self::drive_interactive_ssh_session(
                &mut channel,
                &mut stdin_rx,
                &mut signal_rx,
                &attention_tx,
                &output_tx_clone,
                timeout,
            )
            .await
        });

        Ok(InteractiveHandle {
            // No local child process exists for an SSH-backed session — the
            // command runs on the remote host over the SSH channel, so there
            // is no local pid/pgid to group-kill. `0` is the established
            // sentinel: both `kill_process_group` and `send_signal` in
            // uptrakit-command treat `pid <= 0` as a no-op.
            child_pid: 0,
            stdin_tx,
            signal_tx,
            completion,
            attention_rx,
        })
    }

    /// Drive the interactive SSH session event loop.
    ///
    /// Reads output from the SSH channel, forwards stdin and signals, and
    /// detects attention timeouts (10s of output silence).
    ///
    /// PTY output is coalesced over a short flush interval before being sent
    /// to `output_tx`. This prevents the channel from being flooded by rapid
    /// terminal redraws (e.g. progress bars that use `\r` without `\n`), which
    /// would cause the capacity-bounded channel to drop chunks.
    #[cfg(feature = "interactive")]
    async fn drive_interactive_ssh_session(
        channel: &mut russh::Channel<russh::client::Msg>,
        stdin_rx: &mut mpsc::Receiver<Vec<u8>>,
        signal_rx: &mut mpsc::Receiver<i32>,
        attention_tx: &mpsc::Sender<()>,
        output_tx: &mpsc::Sender<uptrakit_command::UpdateOutputLine>,
        timeout: Option<Duration>,
    ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
        use rootcause::prelude::*;
        use uptrakit_command::CommandError;

        const ATTENTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
        /// Flush interval for coalescing rapid PTY output chunks.
        ///
        /// PTY sessions (especially PHS update scripts) emit many tiny data
        /// chunks per second when drawing progress bars with `\r`. Sending each
        /// chunk individually would saturate the capacity-bounded output channel.
        /// Coalescing over 50 ms (20 Hz) reduces message rate dramatically while
        /// keeping the UI refresh rate imperceptible to a human operator.
        const PTY_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

        let mut state = InteractiveSessionState::new();

        // Periodic flush timer. The first tick fires immediately but is a no-op
        // since both pending buffers are empty at loop start.
        let mut flush_interval = tokio::time::interval(PTY_FLUSH_INTERVAL);
        flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Overall-budget backstop. Resolves once the update's total timeout
        // elapses (or never, when `timeout` is `None`). Created once and pinned
        // so its deadline is fixed across loop iterations.
        let deadline = interactive_deadline(timeout);
        tokio::pin!(deadline);

        loop {
            let attention_sleep = if state.attention_sent {
                tokio::time::sleep(std::time::Duration::from_secs(3600))
            } else {
                let elapsed = state.last_output_time.elapsed();
                if elapsed >= ATTENTION_TIMEOUT {
                    tokio::time::sleep(std::time::Duration::ZERO)
                } else {
                    tokio::time::sleep(ATTENTION_TIMEOUT - elapsed)
                }
            };

            tokio::select! {
                biased;

                msg = channel.wait() => {
                    match msg {
                        Some(ChannelMsg::Data { ref data }) => {
                            state.handle_channel_data(data, OutputStreamType::Stdout, output_tx);
                        }
                        Some(ChannelMsg::ExtendedData { ref data, ext: 1 }) => {
                            state.handle_channel_data(data, OutputStreamType::Stderr, output_tx);
                        }
                        Some(ChannelMsg::ExitStatus { exit_status }) => {
                            state.exit_code = Some(exit_status);
                            if state.eof_received { break; }
                        }
                        Some(ChannelMsg::Eof | ChannelMsg::Close) => {
                            state.eof_received = true;
                            if state.exit_code.is_some() { break; }
                        }
                        Some(_) => {}
                        None => break,
                    }
                }

                Some(data) = stdin_rx.recv() => {
                    if channel.data(&data[..]).await.is_err() {
                        tracing::warn!("failed to write stdin to SSH channel");
                    }
                }

                Some(sig) = signal_rx.recv() => {
                    if let Some(ch) = translate_signal_to_control_char(sig)
                        && channel.data(&[ch][..]).await.is_err()
                    {
                        tracing::warn!("failed to write signal character to SSH channel");
                    }
                }

                _ = flush_interval.tick() => {
                    state.flush_pending_output(output_tx, "coalesced ");
                }

                _ = attention_sleep => {
                    if !state.attention_sent {
                        #[expect(
                            clippy::let_underscore_must_use,
                            reason = "best-effort attention signal; receiver drop or full channel both indicate the consumer no longer needs the heads-up"
                        )]
                        let _ = attention_tx.try_send(());
                        state.attention_sent = true;
                    }
                }

                () = &mut deadline => {
                    tracing::warn!(timeout = ?timeout, "interactive SSH update exceeded its timeout");
                    state.flush_pending_output(output_tx, "final ");
                    bail!(CommandError::TimedOut);
                }
            }
        }

        // Flush any remaining buffered output before returning so the last line
        // (e.g. "Done." without a trailing newline) is visible in the UI.
        state.flush_pending_output(output_tx, "final ");

        // When the remote side closed the channel cleanly (Eof/Close) without
        // sending an ExitStatus — which happens on some PTY sessions over OpenSSH
        // (e.g. Proxmox VE) due to a race between Eof and ExitStatus — treat
        // the exit code as 0 (success) rather than u32::MAX → -1 (failure).
        let code = state
            .exit_code
            .unwrap_or(if state.eof_received { 0 } else { u32::MAX });
        let code_i32 = i32::try_from(code).unwrap_or(-1);

        if code_i32 != 0 {
            bail!(CommandError::CommandFailed(code_i32));
        }

        Ok(uptrakit_command::CommandOutput {
            output: state.accumulated_output,
            exit_code: code_i32,
        })
    }

    /// Upload `data` bytes to `remote_path` via an SFTP subsystem channel.
    pub(crate) async fn sftp_put(
        &self,
        remote_path: &str,
        data: &[u8],
    ) -> std::result::Result<(), SshExecError> {
        use russh_sftp::client::SftpSession;
        use tokio::io::AsyncWriteExt as _;

        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| SshExecError::Exec(format!("SFTP channel open failed: {e}")))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| SshExecError::Exec(format!("SFTP subsystem request failed: {e}")))?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| SshExecError::Exec(format!("SFTP session init failed: {e}")))?;

        let mut file = sftp
            .create(remote_path)
            .await
            .map_err(|e| SshExecError::Exec(format!("SFTP create '{remote_path}' failed: {e}")))?;
        file.write_all(data).await.map_err(|e| {
            SshExecError::Exec(format!("SFTP write to '{remote_path}' failed: {e}"))
        })?;
        file.shutdown()
            .await
            .map_err(|e| SshExecError::Exec(format!("SFTP flush '{remote_path}' failed: {e}")))?;

        Ok(())
    }

    /// Delete `remote_path` via an SFTP subsystem channel.
    pub(crate) async fn sftp_remove(
        &self,
        remote_path: &str,
    ) -> std::result::Result<(), SshExecError> {
        use russh_sftp::client::SftpSession;

        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| SshExecError::Exec(format!("SFTP channel open failed: {e}")))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| SshExecError::Exec(format!("SFTP subsystem request failed: {e}")))?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| SshExecError::Exec(format!("SFTP session init failed: {e}")))?;

        sftp.remove_file(remote_path)
            .await
            .map_err(|e| SshExecError::Exec(format!("SFTP remove '{remote_path}' failed: {e}")))?;

        Ok(())
    }

    /// Disconnect the SSH session.
    pub(crate) async fn disconnect(self) {
        #[expect(
            clippy::let_underscore_must_use,
            reason = "best-effort disconnect on shutdown path; failures here cannot be acted upon and the connection will be closed regardless"
        )]
        let _ = self
            .handle
            .disconnect(Disconnect::ByApplication, "bootstrap complete", "en")
            .await;
    }

    /// Disconnect an [`SshSession`] held behind an [`Arc`].
    ///
    /// Attempts to unwrap the `Arc` and disconnect. If the `Arc` still has
    /// other strong owners at call time (which is a programming error — all
    /// clones should have been dropped before disconnecting), the disconnect
    /// is skipped and a warning is logged so the bug is visible without
    /// panicking.
    pub(crate) async fn disconnect_shared(this: Arc<Self>) {
        match Arc::try_unwrap(this) {
            Ok(session) => session.disconnect().await,
            Err(_) => {
                tracing::warn!(
                    "SshSession::disconnect_shared called with multiple Arc owners; \
                     skipping disconnect — this is a programming error"
                );
            }
        }
    }
}

// ── Interactive session helpers ──────────────────────────────────────

/// Future that resolves once the interactive update's overall budget elapses.
/// `None` means no deadline (resolves never).
#[cfg(feature = "interactive")]
async fn interactive_deadline(timeout: Option<Duration>) {
    match timeout {
        Some(dur) => tokio::time::sleep(dur).await,
        None => std::future::pending::<()>().await,
    }
}

/// Mutable state for the interactive SSH session event loop.
///
/// Extracted from [`SshSession::drive_interactive_ssh_session`] to reduce
/// cyclomatic complexity. Groups accumulated output, truncation tracking,
/// pending coalesced buffers, and attention-timeout bookkeeping.
#[cfg(feature = "interactive")]
struct InteractiveSessionState {
    accumulated_output: String,
    truncated: bool,
    exit_code: Option<u32>,
    eof_received: bool,
    last_output_time: tokio::time::Instant,
    attention_sent: bool,
    /// Pending buffer for coalesced stdout PTY output.
    pending_stdout: String,
    /// Pending buffer for coalesced stderr PTY output.
    pending_stderr: String,
}

#[cfg(feature = "interactive")]
impl InteractiveSessionState {
    /// Maximum accumulated output size (10 MB) to prevent OOM from runaway commands.
    const MAX_OUTPUT: usize = 10 * 1024 * 1024;
    const TRUNCATION_MARKER: &str = "\n... [output truncated at 10 MB] ...\n";
    /// Size threshold for immediate flush: prevent unbounded buffer growth
    /// when output is continuous without any event-loop lull.
    const PTY_FLUSH_SIZE_THRESHOLD: usize = 64 * 1024;

    fn new() -> Self {
        Self {
            accumulated_output: String::new(),
            truncated: false,
            exit_code: None,
            eof_received: false,
            last_output_time: tokio::time::Instant::now(),
            attention_sent: false,
            pending_stdout: String::new(),
            pending_stderr: String::new(),
        }
    }

    /// Handle incoming channel data (stdout or stderr).
    ///
    /// Resets the attention timer, accumulates data (with truncation),
    /// and appends to the coalesced pending buffer. If the pending buffer
    /// exceeds [`Self::PTY_FLUSH_SIZE_THRESHOLD`], flushes it immediately
    /// via a non-blocking send to prevent unbounded memory growth.
    fn handle_channel_data(
        &mut self,
        data: &[u8],
        stream: OutputStreamType,
        output_tx: &mpsc::Sender<uptrakit_command::UpdateOutputLine>,
    ) {
        self.last_output_time = tokio::time::Instant::now();
        self.attention_sent = false;

        let text = String::from_utf8_lossy(data).to_string();

        // Accumulate for total output (with truncation guard).
        if !self.truncated {
            if self.accumulated_output.len() + text.len() > Self::MAX_OUTPUT {
                self.accumulated_output.push_str(Self::TRUNCATION_MARKER);
                self.truncated = true;
            } else {
                self.accumulated_output.push_str(&text);
            }
        }

        // Accumulate for coalesced flush. The flush_interval tick drains the
        // buffer every PTY_FLUSH_INTERVAL. If the buffer exceeds the size
        // threshold (continuous high-throughput output), flush now.
        let pending = match stream {
            OutputStreamType::Stdout => &mut self.pending_stdout,
            OutputStreamType::Stderr => &mut self.pending_stderr,
            _ => {
                tracing::warn!(?stream, "unexpected output stream type; treating as stdout");
                &mut self.pending_stdout
            }
        };
        pending.push_str(&text);

        if pending.len() >= Self::PTY_FLUSH_SIZE_THRESHOLD
            && output_tx
                .try_send(uptrakit_command::UpdateOutputLine {
                    text: std::mem::take(pending),
                    stream,
                })
                .is_err()
        {
            tracing::debug!(?stream, "SSH output channel full; dropping chunk");
        }
    }

    /// Flush any remaining coalesced stdout/stderr to the output channel.
    ///
    /// The `context` parameter is included in the debug log message to
    /// distinguish between periodic flushes and the final flush (e.g.
    /// `"coalesced "` vs `"final "`).
    fn flush_pending_output(
        &mut self,
        output_tx: &mpsc::Sender<uptrakit_command::UpdateOutputLine>,
        context: &str,
    ) {
        for (pending, stream) in [
            (&mut self.pending_stdout, OutputStreamType::Stdout),
            (&mut self.pending_stderr, OutputStreamType::Stderr),
        ] {
            if !pending.is_empty()
                && output_tx
                    .try_send(uptrakit_command::UpdateOutputLine {
                        text: std::mem::take(pending),
                        stream,
                    })
                    .is_err()
            {
                tracing::debug!(?stream, "SSH output channel full; dropping {context}chunk");
            }
        }
    }
}

/// Translate a Unix signal number to the corresponding PTY control character.
///
/// Returns `None` for unsupported signals (with a warning logged).
#[cfg(feature = "interactive")]
fn translate_signal_to_control_char(signal: i32) -> Option<u8> {
    match signal {
        2 => Some(b'\x03'),  // SIGINT  -> Ctrl+C
        3 => Some(b'\x1c'),  // SIGQUIT -> Ctrl+backslash
        28 => Some(b'\x1a'), // SIGTSTP -> Ctrl+Z (signal 20 on Linux, 28 kept for safety)
        _ => {
            tracing::warn!(signal, "unsupported signal for SSH PTY");
            None
        }
    }
}

// ── Public API ───────────────────────────────────────────────────────

/// Connect and authenticate to an SSH server.
///
/// Returns the session wrapper and the observed host key fingerprint.
pub(crate) async fn connect_and_authenticate(
    config: &SshConnectionConfig,
    username: &str,
    auth: &AuthMethod<'_>,
    expected_fingerprint: Option<&str>,
) -> Result<(SshSession, String)> {
    tracing::debug!(hostname = %config.hostname, port = config.port, "connecting to SSH host");
    let observed_fingerprint = Arc::new(Mutex::new(None));

    // Best-effort peek of the SSH identification banner. Used downstream by
    // `detect_host_os` for fast RouterOS detection. Failure is non-fatal —
    // the post-auth shell probe handles unknown servers.
    let server_software =
        peek_ssh_server_id(&config.hostname, config.port, BANNER_PEEK_TIMEOUT).await;
    if let Some(ref software) = server_software {
        tracing::debug!(
            hostname = %config.hostname,
            server_software = %software,
            "peeked SSH server identification banner",
        );
    }

    let handler = BootstrapHandler {
        expected_fingerprint: expected_fingerprint.map(String::from),
        observed_fingerprint: Arc::clone(&observed_fingerprint),
        hostname: config.hostname.clone(),
    };

    let ssh_config = Arc::new(build_ssh_client_config());
    let addr = format!("{}:{}", config.hostname, config.port);

    #[expect(
        clippy::map_err_ignore,
        reason = "tokio Elapsed carries no contextual information beyond the timeout duration already reported"
    )]
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
            authenticate_password_or_kbi(&mut handle, username, password).await?;
        }
        AuthMethod::PrivateKey(pem) => {
            let private_key = Arc::new(keys::decode_secret_key(pem, None).map_err(|e| {
                report!(Error::SshAuth(format!("failed to decode private key: {e}")))
            })?);
            let hash_algs = rsa_hash_alg_candidates_for(&handle, private_key.algorithm()).await;
            let mut success = false;
            for hash_alg in hash_algs {
                let key_with_alg = PrivateKeyWithHashAlg::new(Arc::clone(&private_key), hash_alg);
                let auth_result = handle
                    .authenticate_publickey(username.to_string(), key_with_alg)
                    .await
                    .map_err(|e| {
                        report!(Error::SshAuth(format!(
                            "public key authentication failed: {e}"
                        )))
                    })?;
                if auth_result.success() {
                    success = true;
                    break;
                }
                if let AuthResult::Failure {
                    ref remaining_methods,
                    ..
                } = auth_result
                {
                    tracing::debug!(
                        ?remaining_methods,
                        "public key probe rejected; trying next hash algorithm if any",
                    );
                }
            }
            if !success {
                bail!(Error::SshAuth(format!(
                    "authentication failed for user '{username}'"
                )));
            }
        }
        AuthMethod::Agent => {
            authenticate_with_agent(&mut handle, username).await?;
        }
    }

    Ok((
        SshSession {
            handle,
            hostname: config.hostname.clone(),
            server_software,
        },
        fp,
    ))
}

// ── Internal helpers ─────────────────────────────────────────────────

/// Authenticate using a username + password pair.
///
/// SSH defines two distinct auth methods that take a password:
/// - `password` — single round, password-in-the-request
/// - `keyboard-interactive` — challenge/response (the server may send any
///   number of prompts)
///
/// Most servers accept both; some (notably MikroTik RouterOS) only accept
/// `keyboard-interactive`. To avoid burning a server-side failed-auth counter
/// on a method the server will reject anyway, we first call
/// [`Handle::authenticate_none`] to discover the methods the server is willing
/// to accept, then dispatch to the right one. Mirrors OpenSSH client behavior.
async fn authenticate_password_or_kbi<H: client::Handler>(
    handle: &mut Handle<H>,
    username: &str,
    password: &str,
) -> Result<()> {
    // `authenticate_none` does NOT consume a failed-auth counter on most
    // servers — it is the standard method-discovery probe used by OpenSSH.
    let none_result = handle
        .authenticate_none(username.to_string())
        .await
        .map_err(|e| {
            report!(Error::SshAuth(format!(
                "ssh method-discovery probe failed: {e}"
            )))
        })?;

    let methods = match none_result {
        AuthResult::Success => {
            // Server accepts auth without credentials — odd but legal.
            return Ok(());
        }
        AuthResult::Failure {
            remaining_methods, ..
        } => remaining_methods,
    };
    tracing::debug!(
        ?methods,
        "ssh method-discovery probe complete; server-accepted methods",
    );

    let supports_password = methods.contains(&MethodKind::Password);
    let supports_kbi = methods.contains(&MethodKind::KeyboardInteractive);

    if !supports_password && !supports_kbi {
        bail!(Error::SshAuth(format!(
            "server does not accept password-based authentication for user '{username}' \
             (advertised methods: {methods:?})"
        )));
    }

    if supports_password {
        let result = handle
            .authenticate_password(username.to_string(), password.to_string())
            .await
            .map_err(|e| {
                report!(Error::SshAuth(format!(
                    "password authentication failed: {e}"
                )))
            })?;
        if result.success() {
            return Ok(());
        }
        if let AuthResult::Failure {
            ref remaining_methods,
            ..
        } = result
        {
            tracing::debug!(
                ?remaining_methods,
                "password auth rejected; checking for keyboard-interactive fallback",
            );
            if !remaining_methods.contains(&MethodKind::KeyboardInteractive) {
                bail!(Error::SshAuth(format!(
                    "authentication failed for user '{username}'"
                )));
            }
        }
        // Fall through to keyboard-interactive below.
    }

    authenticate_keyboard_interactive(handle, username, password).await
}

/// Authenticate using the SSH `keyboard-interactive` method, responding to
/// every server prompt with `password`.
///
/// Used as a fallback when the server (notably MikroTik RouterOS) rejects the
/// raw `password` auth method but accepts `keyboard-interactive` for password
/// logins, mirroring OpenSSH client behavior.
///
/// To avoid silently echoing the user's password into a "new password" or OTP
/// prompt, the helper bails if any prompt does not match
/// [`is_password_prompt`].
async fn authenticate_keyboard_interactive<H: client::Handler>(
    handle: &mut Handle<H>,
    username: &str,
    password: &str,
) -> Result<()> {
    let mut response = handle
        .authenticate_keyboard_interactive_start(username.to_string(), None)
        .await
        .map_err(|e| {
            report!(Error::SshAuth(format!(
                "keyboard-interactive auth failed: {e}"
            )))
        })?;
    loop {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(()),
            KeyboardInteractiveAuthResponse::Failure { .. } => {
                bail!(Error::SshAuth(format!(
                    "authentication failed for user '{username}'"
                )));
            }
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                if !prompts.iter().all(|p| is_password_prompt(&p.prompt)) {
                    let raw: Vec<&String> = prompts.iter().map(|p| &p.prompt).collect();
                    bail!(Error::SshAuth(format!(
                        "keyboard-interactive server requested non-password prompt(s) ({raw:?}); \
                         interactive bootstrap is not supported"
                    )));
                }
                let responses: Vec<String> = prompts.iter().map(|_| password.to_string()).collect();
                response = handle
                    .authenticate_keyboard_interactive_respond(responses)
                    .await
                    .map_err(|e| {
                        report!(Error::SshAuth(format!(
                            "keyboard-interactive auth response failed: {e}"
                        )))
                    })?;
            }
        }
    }
}

/// True for prompts that look like a current-password prompt.
///
/// Mirrors OpenSSH's heuristic: matches `password` (case-insensitive) with an
/// optional trailing colon and surrounding whitespace. Rejects `new password`,
/// `verify password`, OTP and security-question prompts so the caller does not
/// silently send the user's existing password into an unrelated prompt.
fn is_password_prompt(prompt: &str) -> bool {
    let trimmed = prompt.trim().trim_end_matches(':').trim();
    trimmed.eq_ignore_ascii_case("password")
}

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
        let pub_key = key.public_key().into_owned();
        let hash_algs = rsa_hash_alg_candidates_for(handle, pub_key.algorithm()).await;

        let mut accepted = false;
        for hash_alg in hash_algs {
            let result = handle
                .authenticate_publickey_with(
                    username.to_string(),
                    pub_key.clone(),
                    hash_alg,
                    &mut agent,
                )
                .await
                .map_err(|e| report!(Error::SshAuth(format!("SSH agent signing failed: {e}"))))?;

            if result.success() {
                accepted = true;
                break;
            }
            if let AuthResult::Failure {
                ref remaining_methods,
                ..
            } = result
            {
                tracing::debug!(
                    ?remaining_methods,
                    "agent key probe rejected; trying next hash algorithm if any",
                );
            }
        }

        if accepted {
            return Ok(());
        }
    }

    bail!(Error::SshAuth(format!(
        "none of the {} SSH agent key(s) were accepted for user '{username}'",
        identities.len()
    )));
}

/// Decide which RSA hash algorithms to try, preferring `server-sig-algs` when
/// the server advertises it via `EXT_INFO`.
///
/// - For non-RSA keys (Ed25519, ECDSA), hash algorithm selection is irrelevant
///   so we return a single `None` entry without polling for the extension.
/// - For RSA keys, we ask russh's [`Handle::best_supported_rsa_hash`] which
///   returns the strongest hash algorithm the server advertised. We use only
///   that algorithm — matching OpenSSH client behavior.
/// - When the server does not advertise `server-sig-algs` (older or
///   intentionally minimal servers), we fall back to a candidate list of
///   `[Some(SHA-256), Some(SHA-512), None]`. SHA-256 is RFC 8332 mandatory
///   and accepted by every modern server; SHA-512 is optional. Trying SHA-512
///   first is what triggered the original RouterOS bug, so SHA-256 leads.
async fn rsa_hash_alg_candidates_for<H: client::Handler>(
    handle: &Handle<H>,
    algorithm: Algorithm,
) -> Vec<Option<HashAlg>> {
    if !matches!(algorithm, Algorithm::Rsa { .. }) {
        return vec![None];
    }

    match handle.best_supported_rsa_hash().await {
        Ok(Some(alg)) => {
            tracing::debug!(?alg, "using server-advertised RSA hash algorithm");
            vec![alg]
        }
        Ok(None) => {
            tracing::debug!(
                "server advertised server-sig-algs without rsa-sha2-* — using legacy ssh-rsa",
            );
            vec![None]
        }
        Err(e) => {
            tracing::debug!(
                error = %e,
                "server did not provide server-sig-algs; falling back to candidate list",
            );
            vec![Some(HashAlg::Sha256), Some(HashAlg::Sha512), None]
        }
    }
}

/// Compute the SHA-256 fingerprint of an SSH public key in `SHA256:...` format.
fn compute_fingerprint(key: &russh::keys::ssh_key::PublicKey) -> String {
    format!("{}", key.fingerprint(russh::keys::ssh_key::HashAlg::Sha256))
}

/// Peek at the server's SSH identification string (e.g. `"ROSSSH"`,
/// `"OpenSSH_10.0p2 Debian-7+deb13u2"`, `"dropbear"`).
///
/// Returns the software portion after the `SSH-2.0-` / `SSH-1.99-` prefix, or
/// `None` if the peek fails (timeout, refused, malformed banner). Best-effort
/// only — callers fall back to a post-auth shell probe.
///
/// This opens a SECOND TCP connection to the host (russh creates its own when
/// `connect()` is called). Acceptable because bootstrap is rare and single-host.
/// The banner is read over an unauthenticated channel and is therefore used
/// only for feature detection, never security decisions.
async fn peek_ssh_server_id(hostname: &str, port: u16, timeout: Duration) -> Option<String> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

    let stream = tokio::time::timeout(timeout, tokio::net::TcpStream::connect((hostname, port)))
        .await
        .ok()?
        .ok()?;
    // RFC 4253 §4.2 caps the SSH identification string at 255 bytes including CRLF.
    let mut reader = BufReader::new(stream).take(255);
    let mut line = String::new();
    tokio::time::timeout(timeout, reader.read_line(&mut line))
        .await
        .ok()?
        .ok()?;
    let trimmed = line.trim_end_matches(['\r', '\n']);
    trimmed
        .strip_prefix("SSH-2.0-")
        .or_else(|| trimmed.strip_prefix("SSH-1.99-"))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::client::Handler;

    /// The exact shape `pve_setup::prove_token_on_node` builds: the live PVE
    /// token would otherwise be traced verbatim by `exec_command_streaming`.
    #[test]
    fn redact_for_log_masks_authorization_header_value() {
        let secret = "uptrakit@pve!tenant-3f2b=9d1c-SECRET-VALUE";
        let command = format!(
            "curl -sk -o /dev/null -w '%{{http_code}}' \
             -H 'Authorization: PVEAPIToken={secret}' \
             https://localhost:8006/api2/json/version"
        );

        let redacted = redact_for_log(&command);

        assert!(
            !redacted.contains(secret),
            "token must not survive redaction: {redacted}"
        );
        assert!(
            !redacted.contains("PVEAPIToken="),
            "the whole header value is masked, not just the token tail: {redacted}"
        );
        assert!(
            redacted.contains("-H 'Authorization: <redacted>' "),
            "the header argument stays structurally intact: {redacted}"
        );
        assert!(
            redacted.contains("https://localhost:8006/api2/json/version"),
            "text after the header is preserved: {redacted}"
        );
    }

    #[test]
    fn redact_for_log_leaves_ordinary_commands_untouched() {
        let command = "pveum user token list 'uptrakit@pve' --output-format json 2>&1";

        let redacted = redact_for_log(command);

        assert_eq!(redacted, command);
        assert!(
            matches!(redacted, std::borrow::Cow::Borrowed(_)),
            "a command with no sensitive header is not reallocated"
        );
    }

    #[test]
    fn redact_for_log_masks_every_header_and_survives_an_unterminated_value() {
        // Mixed case (headers are case-insensitive), two occurrences, and a
        // trailing header with no closing quote — the failure mode where a
        // naive "mask up to the next quote" pass would emit the secret.
        let command =
            "sh -c \"curl -H 'authorization: Bearer AAA' host; curl -H \"Authorization: Bearer BBB";

        let redacted = redact_for_log(command);

        assert!(!redacted.contains("AAA"), "first secret leaked: {redacted}");
        assert!(
            !redacted.contains("BBB"),
            "second secret leaked: {redacted}"
        );
        assert_eq!(
            redacted.matches("<redacted>").count(),
            2,
            "both headers are masked: {redacted}"
        );
    }

    #[tokio::test]
    async fn handler_tofu_accepts() {
        let observed = Arc::new(Mutex::new(None));
        let mut handler = BootstrapHandler {
            expected_fingerprint: None,
            observed_fingerprint: Arc::clone(&observed),
            hostname: "test-host".to_string(),
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

    #[test]
    fn is_password_prompt_accepts_canonical_forms() {
        assert!(is_password_prompt("Password:"));
        assert!(is_password_prompt("password:"));
        assert!(is_password_prompt("PASSWORD:"));
        assert!(is_password_prompt("Password: "));
        assert!(is_password_prompt(" Password "));
        assert!(is_password_prompt("password"));
    }

    #[test]
    fn is_password_prompt_rejects_unrelated_or_compound_prompts() {
        // Compound prompts that include "password" but mean something else.
        assert!(!is_password_prompt("New password:"));
        assert!(!is_password_prompt("Verify password:"));
        assert!(!is_password_prompt("Enter new password:"));
        assert!(!is_password_prompt("Re-enter new password:"));
        // OTP / security challenges.
        assert!(!is_password_prompt("OTP:"));
        assert!(!is_password_prompt("Verification code:"));
        assert!(!is_password_prompt("Answer:"));
        // Empty / pathological.
        assert!(!is_password_prompt(""));
        assert!(!is_password_prompt(":"));
    }

    #[tokio::test]
    async fn handler_pinned_rejects_mismatch() {
        let observed = Arc::new(Mutex::new(None));
        let mut handler = BootstrapHandler {
            expected_fingerprint: Some("SHA256:wrong_fingerprint".to_string()),
            observed_fingerprint: Arc::clone(&observed),
            hostname: "test-host".to_string(),
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
            hostname: "test-host".to_string(),
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
        let mut buf = LineBuffer::new(OutputStreamType::Stdout, Some(tx));

        buf.push(b"hello\nworld\n");

        let line1 = rx.recv().await.expect("should receive first line");
        assert_eq!(line1.text, "hello");
        assert_eq!(line1.stream, OutputStreamType::Stdout);

        let line2 = rx.recv().await.expect("should receive second line");
        assert_eq!(line2.text, "world");
    }

    #[tokio::test]
    async fn line_buffer_holds_partial_lines() {
        let (tx, mut rx) = mpsc::channel(100);
        let mut buf = LineBuffer::new(OutputStreamType::Stdout, Some(tx));

        buf.push(b"partial");

        // No complete line yet — channel should be empty.
        assert!(
            rx.try_recv().is_err(),
            "partial line should not be sent yet"
        );

        // Now complete the line.
        buf.push(b" end\n");
        let line = rx.recv().await.expect("should receive completed line");
        assert_eq!(line.text, "partial end");
    }

    #[tokio::test]
    async fn line_buffer_flush_emits_remaining() {
        let (tx, mut rx) = mpsc::channel(100);
        let mut buf = LineBuffer::new(OutputStreamType::Stderr, Some(tx));

        buf.push(b"trailing");
        buf.flush();

        let line = rx.recv().await.expect("flush should emit partial");
        assert_eq!(line.text, "trailing");
        assert_eq!(line.stream, OutputStreamType::Stderr);

        let output = buf.into_output();
        assert_eq!(output, "trailing\n");
    }

    #[tokio::test]
    async fn line_buffer_respects_output_limit() {
        let mut buf = LineBuffer::new(OutputStreamType::Stdout, None);

        // Push data exceeding MAX_OUTPUT_BYTES (10 MB).
        let big_line = "x".repeat(1_000_000);
        for _ in 0..12 {
            let mut data = big_line.clone();
            data.push('\n');
            buf.push(data.as_bytes());
        }

        assert!(buf.truncated, "buffer should be marked as truncated");
        let output = buf.into_output();
        assert!(
            output.contains("[output truncated at 10 MB]"),
            "output should contain truncation marker"
        );
    }

    #[tokio::test]
    async fn line_buffer_streams_all_lines_even_after_truncation() {
        let (tx, mut rx) = mpsc::channel(100);
        let mut buf = LineBuffer::new(OutputStreamType::Stdout, Some(tx));

        // Push data exceeding MAX_OUTPUT_BYTES (10 MB).
        let big_line = "x".repeat(1_000_000);
        for _ in 0..12 {
            let mut data = big_line.clone();
            data.push('\n');
            buf.push(data.as_bytes());
        }

        // All 12 lines should be streamed even though accumulation is truncated.
        let mut streamed_count = 0;
        while rx.try_recv().is_ok() {
            streamed_count += 1;
        }
        assert_eq!(
            streamed_count, 12,
            "all lines should be streamed regardless of truncation"
        );
    }

    #[tokio::test]
    async fn line_buffer_works_without_sender() {
        let mut buf = LineBuffer::new(OutputStreamType::Stdout, None);

        buf.push(b"line1\nline2\n");
        buf.flush();

        let output = buf.into_output();
        assert_eq!(output, "line1\nline2\n");
    }

    #[tokio::test]
    async fn line_buffer_flush_noop_when_empty() {
        let mut buf = LineBuffer::new(OutputStreamType::Stdout, None);
        buf.flush();
        let output = buf.into_output();
        assert!(output.is_empty());
    }

    #[test]
    fn remote_command_finished_requires_both_eof_and_exit_status() {
        assert!(!remote_command_finished(false, None));
        assert!(!remote_command_finished(true, None));
        assert!(!remote_command_finished(false, Some(0)));
        assert!(remote_command_finished(true, Some(1)));
    }

    #[test]
    fn client_config_enables_keepalive() {
        let config = build_ssh_client_config();

        // Verify keepalive_interval is set to SSH_KEEPALIVE_INTERVAL.
        assert_eq!(
            config.keepalive_interval,
            Some(SSH_KEEPALIVE_INTERVAL),
            "keepalive_interval should be set to SSH_KEEPALIVE_INTERVAL (15s)"
        );

        // Verify keepalive_max is set to SSH_KEEPALIVE_MAX.
        assert_eq!(
            config.keepalive_max, SSH_KEEPALIVE_MAX,
            "keepalive_max should be set to SSH_KEEPALIVE_MAX (4)"
        );

        // Verify inactivity_timeout is None (not a hard timeout).
        assert_eq!(
            config.inactivity_timeout, None,
            "inactivity_timeout should be None to allow graceful keepalive detection"
        );
    }
}

#[cfg(all(test, feature = "interactive"))]
mod interactive_timeout_tests {
    use super::*;
    use std::time::Duration;

    // The deadline policy: `Some(budget)` resolves after the budget elapses;
    // `None` never resolves. Tested via `tokio::time::timeout` under
    // `start_paused` — the paused runtime auto-advances to the nearest timer
    // when all tasks are parked, so racing two timers resolves deterministically
    // by earliest deadline, with NO new dependency and NO manual `advance`.
    #[tokio::test(start_paused = true)]
    async fn some_budget_resolves_after_it_elapses() {
        let mut fut = Box::pin(interactive_deadline(Some(Duration::from_secs(7200))));
        // A 7199s timeout fires before the 7200s budget → still pending.
        assert!(
            tokio::time::timeout(Duration::from_secs(7199), &mut fut)
                .await
                .is_err(),
            "deadline must not resolve before the budget elapses"
        );
        // 2 more seconds crosses 7200s → the budget future now resolves first.
        assert!(
            tokio::time::timeout(Duration::from_secs(2), &mut fut)
                .await
                .is_ok(),
            "deadline must resolve once the budget elapses"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn none_budget_never_resolves() {
        let fut = interactive_deadline(None);
        assert!(
            tokio::time::timeout(Duration::from_secs(100_000), fut)
                .await
                .is_err(),
            "a None budget must never resolve"
        );
    }
}
