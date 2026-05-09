//! Interactive command execution with PTY allocation.
//!
//! This module provides PTY-backed command execution for interactive update
//! sessions. The child process gets a real terminal (via `rustix::pty::openpt`/
//! `grantpt`/`unlockpt`/`ptsname`), enabling package managers and other tools
//! that require stdin to function correctly.
//!
//! This entire module is gated on the `interactive` feature.

use std::io::{self, Read, Write};
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::process::CommandExt;

use rootcause::prelude::*;
use rustix::fs::{Mode, OFlags};
use rustix::pty::OpenptFlags;
use tokio::sync::mpsc;

use crate::command::send_output;
use crate::error::CommandError;
use crate::types::{CommandOutput, CommandSpec, InteractiveHandle, UpdateOutputLine};
use uptrakit_shared_types::OutputStreamType;

/// Channels for an interactive session, bundled to reduce function argument count.
struct SessionChannels {
    stdin_rx: mpsc::Receiver<Vec<u8>>,
    signal_rx: mpsc::Receiver<i32>,
    attention_tx: mpsc::Sender<()>,
    output_tx: mpsc::Sender<UpdateOutputLine>,
}

/// Duration of output silence before emitting an attention notification.
const ATTENTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Maximum accumulated output size (10 MB, matching non-interactive limit).
const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Truncation marker appended when output exceeds `MAX_OUTPUT_BYTES`.
const TRUNCATION_MARKER: &str = "\n... [output truncated at 10 MB] ...\n";

/// Run a command interactively with a PTY.
///
/// Allocates a PTY pair, spawns the child with the slave fd as its
/// stdin/stdout/stderr, and returns an [`InteractiveHandle`] for stdin
/// forwarding, signal delivery, and completion awaiting.
pub async fn run_command_interactive(
    spec: &CommandSpec,
    output_tx: &mpsc::Sender<UpdateOutputLine>,
) -> crate::Result<InteractiveHandle> {
    let (program, args) = spec.resolve()?;

    let (master_fd, slave_fd) = allocate_pty()?;

    let child = spawn_child_with_pty(
        &program,
        &args,
        spec.working_dir.as_deref(),
        &spec.envs,
        slave_fd,
    )?;
    let child_pid = child.id().unwrap_or(0) as i32;

    let (stdin_tx, stdin_rx) = mpsc::channel::<Vec<u8>>(64);
    let (signal_tx, signal_rx) = mpsc::channel::<i32>(8);
    let (attention_tx, attention_rx) = mpsc::channel::<()>(4);

    let output_tx_clone = output_tx.clone();
    let timeout = spec.timeout;

    let channels = SessionChannels {
        stdin_rx,
        signal_rx,
        attention_tx,
        output_tx: output_tx_clone,
    };

    let completion = tokio::spawn(async move {
        drive_interactive_session(master_fd, child, child_pid, channels, timeout).await
    });

    Ok(InteractiveHandle {
        stdin_tx,
        signal_tx,
        completion,
        attention_rx,
    })
}

/// Allocate a PTY master/slave pair.
fn allocate_pty() -> crate::Result<(OwnedFd, OwnedFd)> {
    let master_fd = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)
        .map_err(|e| report!(CommandError::PtyAllocationFailed(io::Error::from(e))))?;

    rustix::pty::grantpt(&master_fd)
        .map_err(|e| report!(CommandError::PtyAllocationFailed(io::Error::from(e))))?;
    rustix::pty::unlockpt(&master_fd)
        .map_err(|e| report!(CommandError::PtyAllocationFailed(io::Error::from(e))))?;

    let slave_path = rustix::pty::ptsname(&master_fd, Vec::new())
        .map_err(|e| report!(CommandError::PtyAllocationFailed(io::Error::from(e))))?;

    let slave_fd = rustix::fs::open(&slave_path, OFlags::RDWR | OFlags::NOCTTY, Mode::empty())
        .map_err(|e| report!(CommandError::PtyAllocationFailed(io::Error::from(e))))?;

    Ok((master_fd, slave_fd))
}

/// Post-fork, pre-exec callback for PTY children.
///
/// Runs in the forked child after [`std::process::Command`] has dup2'd the slave PTY into
/// fd 0/1/2 and before `execvp`. Must be async-signal-safe; only setsid and
/// ioctl(TIOCSCTTY) are invoked.
fn pty_pre_exec() -> io::Result<()> {
    rustix::process::setsid().map_err(|e| io::Error::from_raw_os_error(e.raw_os_error()))?;
    // SAFETY: post-dup2 in child, fd 0 is the PTY slave (owned by Command's stdio
    // plumbing for the lifetime of the callback). TIOCSCTTY must follow setsid so
    // the child has no prior controlling terminal.
    let ctty = unsafe { BorrowedFd::borrow_raw(0) };
    rustix::process::ioctl_tiocsctty(ctty)
        .map_err(|e| io::Error::from_raw_os_error(e.raw_os_error()))?;
    Ok(())
}

/// Spawn a child process with the slave PTY as stdin/stdout/stderr.
fn spawn_child_with_pty(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
    envs: &[(String, String)],
    slave_fd: OwnedFd,
) -> crate::Result<tokio::process::Child> {
    use std::process::Stdio;

    let slave_in = slave_fd
        .try_clone()
        .map_err(|e| report!(CommandError::PtyAllocationFailed(e)))?;
    let slave_out = slave_fd
        .try_clone()
        .map_err(|e| report!(CommandError::PtyAllocationFailed(e)))?;
    let slave_err = slave_fd; // last assignment consumes the original

    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    cmd.stdin(Stdio::from(slave_in));
    cmd.stdout(Stdio::from(slave_out));
    cmd.stderr(Stdio::from(slave_err));

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }
    for (name, value) in envs {
        cmd.env(name, value);
    }

    // SAFETY: Command::pre_exec is unsafe by language contract; the callback must be
    // async-signal-safe. `pty_pre_exec` only invokes setsid and ioctl(TIOCSCTTY), both
    // POSIX async-signal-safe syscalls; rustix wrappers add no allocation or locking.
    unsafe {
        cmd.pre_exec(pty_pre_exec);
    }

    let mut tokio_cmd = tokio::process::Command::from(cmd);
    tokio_cmd.kill_on_drop(true);

    tokio_cmd
        .spawn()
        .map_err(|e| report!(CommandError::CommandSpawn(e)))
}

/// Drive the interactive session.
///
/// Reads output from PTY master via a blocking-thread reader, forwards stdin
/// from the channel, handles signals, and detects attention timeouts.
async fn drive_interactive_session(
    master_fd: OwnedFd,
    mut child: tokio::process::Child,
    child_pid: i32,
    channels: SessionChannels,
    timeout: Option<std::time::Duration>,
) -> crate::Result<CommandOutput> {
    let SessionChannels {
        mut stdin_rx,
        mut signal_rx,
        attention_tx,
        output_tx,
    } = channels;
    // Duplicate the master fd for the reader thread.
    let reader_fd = master_fd
        .try_clone()
        .map_err(|e| report!(CommandError::PtyAllocationFailed(e)))?;
    let reader_file = std::fs::File::from(reader_fd);
    let writer_file = std::fs::File::from(master_fd);

    // Channel for output chunks from the blocking reader thread
    let (chunk_tx, mut chunk_rx) = mpsc::channel::<Result<Vec<u8>, io::Error>>(64);

    // Spawn blocking reader thread for the PTY master
    tokio::task::spawn_blocking(move || {
        let mut file = reader_file;
        let mut buf = vec![0u8; 4096];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // n <= buf.len() is guaranteed by the Read::read contract.
                    #[expect(
                        clippy::indexing_slicing,
                        reason = "n <= buf.len() is guaranteed by Read::read; indexing cannot panic here"
                    )]
                    let chunk = buf[..n].to_vec();
                    if chunk_tx.blocking_send(Ok(chunk)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    // EIO is expected when the slave side closes
                    if e.raw_os_error() != Some(rustix::io::Errno::IO.raw_os_error()) {
                        #[expect(
                            clippy::let_underscore_must_use,
                            reason = "blocking_send error means receiver dropped; we're already breaking out of the loop"
                        )]
                        let _ = chunk_tx.blocking_send(Err(e));
                    }
                    break;
                }
            }
        }
    });

    // Spawn stdin writer task
    let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(64);
    tokio::task::spawn_blocking(move || {
        let mut file = writer_file;
        while let Some(data) = write_rx.blocking_recv() {
            if file.write_all(&data).is_err() {
                break;
            }
        }
    });

    let mut accumulated_output = String::new();
    let mut truncated = false;
    let mut last_output_time = tokio::time::Instant::now();
    let mut attention_sent = false;
    let deadline = timeout.map(|d| tokio::time::Instant::now() + d);

    loop {
        let attention_sleep = if attention_sent {
            tokio::time::sleep(std::time::Duration::from_secs(3600))
        } else {
            let elapsed = last_output_time.elapsed();
            if elapsed >= ATTENTION_TIMEOUT {
                tokio::time::sleep(std::time::Duration::ZERO)
            } else {
                tokio::time::sleep(ATTENTION_TIMEOUT - elapsed)
            }
        };

        let deadline_sleep = match deadline {
            Some(d) => tokio::time::sleep_until(d),
            None => tokio::time::sleep(std::time::Duration::from_secs(86400)),
        };

        tokio::select! {
            biased;

            _ = deadline_sleep, if deadline.is_some() => {
                tracing::warn!("interactive command timed out");
                kill_process_group(child_pid);
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "wait() after kill; we're returning an error immediately and don't need the exit status"
                )]
                let _ = child.wait().await;
                return Err(report!(CommandError::TimedOut));
            }

            result = chunk_rx.recv() => {
                match result {
                    Some(Ok(data)) => {
                        last_output_time = tokio::time::Instant::now();
                        attention_sent = false;

                        let text = String::from_utf8_lossy(&data).to_string();
                        if !truncated {
                            if accumulated_output.len() + text.len() > MAX_OUTPUT_BYTES {
                                accumulated_output.push_str(TRUNCATION_MARKER);
                                truncated = true;
                            } else {
                                accumulated_output.push_str(&text);
                            }
                        }

                        send_output(&output_tx, &text, OutputStreamType::Stdout).await;
                    }
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, "error reading from PTY master");
                        break;
                    }
                    None => break, // Reader finished
                }
            }

            Some(data) = stdin_rx.recv() => {
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "send failure means writer task exited (PTY closed); the next chunk_rx.recv() will also fail and break the loop"
                )]
                let _ = write_tx.send(data).await;
            }

            Some(sig) = signal_rx.recv() => {
                send_signal(child_pid, sig);
            }

            _ = attention_sleep => {
                if !attention_sent {
                    #[expect(
                        clippy::let_underscore_must_use,
                        reason = "try_send failure means the attention channel is full or closed; attention is best-effort and dropping is intentional"
                    )]
                    let _ = attention_tx.try_send(());
                    attention_sent = true;
                }
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| report!(CommandError::CommandWait(e)))?;

    let exit_code = status.code().unwrap_or(-1);

    if exit_code != 0 {
        bail!(CommandError::CommandFailed(exit_code));
    }

    Ok(CommandOutput {
        output: accumulated_output,
        exit_code,
    })
}

/// Send a signal to the process group.
fn send_signal(pid: i32, signal: i32) {
    if pid <= 0 {
        tracing::warn!(pid, signal, "cannot send signal to invalid pid");
        return;
    }
    let Some(pid_typed) = rustix::process::Pid::from_raw(pid) else {
        tracing::warn!(pid, signal, "cannot send signal to invalid pid");
        return;
    };
    let Some(sig) = rustix::process::Signal::from_named_raw(signal) else {
        tracing::warn!(pid, signal, "cannot send signal: not a named signal number");
        return;
    };
    if let Err(err) = rustix::process::kill_process_group(pid_typed, sig) {
        tracing::warn!(pid, signal, error = %err, "failed to send signal to process group");
    }
}

/// Kill the process group with SIGKILL.
fn kill_process_group(pid: i32) {
    if pid <= 0 {
        return;
    }
    let Some(pid_typed) = rustix::process::Pid::from_raw(pid) else {
        return;
    };
    if let Err(err) = rustix::process::kill_process_group(pid_typed, rustix::process::Signal::KILL)
    {
        tracing::warn!(pid, error = %err, "failed to deliver SIGKILL to process group");
    }
}

pub use run_command_interactive as run_interactive;

#[cfg(test)]
mod tests {

    use super::*;

    #[tokio::test]
    async fn interactive_echo() {
        let spec = CommandSpec::shell("echo 'hello interactive'");
        let (output_tx, mut output_rx) = mpsc::channel(100);

        let handle = match run_command_interactive(&spec, &output_tx).await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(error = %e, "skipping: PTY unavailable");
                return;
            }
        };

        let result = handle.completion.await.expect("task should not panic");
        assert!(result.is_ok(), "echo should succeed: {result:?}");
        let output = result.unwrap();
        assert!(
            output.output.contains("hello interactive"),
            "expected 'hello interactive', got: {:?}",
            output.output
        );

        output_rx.close();
        while output_rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn interactive_stdin_forwarding() {
        let spec = CommandSpec::shell("cat");
        let (output_tx, mut output_rx) = mpsc::channel(100);

        let handle = match run_command_interactive(&spec, &output_tx).await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(error = %e, "skipping: PTY unavailable");
                return;
            }
        };

        handle
            .stdin_tx
            .send(b"test input\n".to_vec())
            .await
            .expect("send should succeed");

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Send EOF (Ctrl+D)
        handle
            .stdin_tx
            .send(vec![0x04])
            .await
            .expect("send should succeed");

        let result = handle.completion.await.expect("task should not panic");
        assert!(result.is_ok(), "cat should succeed: {result:?}");

        output_rx.close();
        while output_rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn interactive_signal_delivery() {
        let spec = CommandSpec::shell("sleep 300");
        let (output_tx, mut output_rx) = mpsc::channel(100);

        let handle = match run_command_interactive(&spec, &output_tx).await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(error = %e, "skipping: PTY unavailable");
                return;
            }
        };

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        handle
            .signal_tx
            .send(2) // SIGINT
            .await
            .expect("send should succeed");

        let result = handle.completion.await.expect("task should not panic");
        assert!(result.is_err(), "sleep should fail after SIGINT");

        output_rx.close();
        while output_rx.recv().await.is_some() {}
    }
}
