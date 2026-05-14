pub(crate) mod listenfd;
pub(crate) mod sd_notify;
pub(crate) mod triage;

// Coordinator pre-apply hook wiring: future task. For now, triage + reexec
// helpers are available but not yet called by the coordinator.

use std::path::PathBuf;

use rootcause::Report;

/// Parameters describing how to re-exec the current process.
///
/// Populated by the caller with the runtime values gathered at startup so that
/// `perform_reexec` can reconstruct an equivalent command line for the new
/// process generation.
pub(crate) struct ReexecPlan {
    /// Absolute path to the current executable (from [`std::env::current_exe`]).
    pub(crate) current_exe: PathBuf,
    /// Path to the TOML configuration file passed as `--config`.
    pub(crate) config_path: PathBuf,
    /// Path to the master key file, if one was provided via `--master-key-file`.
    pub(crate) master_key_file: Option<String>,
    /// Number of file descriptors that were bound and should be inherited via
    /// `LISTEN_FDS` by the new process generation.
    pub(crate) listener_count: usize,
    /// Current process generation counter.  The new process will receive
    /// `generation + 1` via `UPTRAKIT_REEXEC_GENERATION`.
    pub(crate) generation: u64,
}

/// Replace the current process image with a new instance of the same binary.
///
/// This function:
/// 1. Clears `FD_CLOEXEC` on each entry in `listener_fds` so the descriptors
///    survive `exec()` and the new process image can claim them via `LISTEN_FDS`.
/// 2. Constructs a `Command` equivalent to the original invocation.
/// 3. Forwards `LISTEN_FDS` / `LISTEN_PID` so the new process can inherit
///    the already-bound TCP sockets.
/// 4. Sets `UPTRAKIT_REEXEC_GENERATION` so observability tooling can track
///    how many times the process has re-execed.
/// 5. Calls `exec()` which, on success, replaces the process image and never
///    returns.  On failure the OS error is wrapped and returned.
///
/// # Errors
///
/// Returns an error if clearing `FD_CLOEXEC` fails or if `exec()` fails
/// (e.g. the binary path is no longer accessible).  The error is always
/// non-fatal from the caller's perspective because the original process is
/// still running.
#[expect(
    dead_code,
    reason = "wired into the coordinator pre-apply hook in a future graceful-reload task"
)]
pub(crate) fn perform_reexec(
    plan: &ReexecPlan,
    listener_fds: &[std::os::unix::io::RawFd],
) -> Result<std::convert::Infallible, Report> {
    use std::os::unix::process::CommandExt as _;

    // Clear FD_CLOEXEC on each listener so it survives exec().
    for &fd in listener_fds {
        listenfd::clear_cloexec_raw(fd)?;
    }

    let mut cmd = std::process::Command::new(&plan.current_exe);
    cmd.arg("--config").arg(&plan.config_path);

    if let Some(mk) = &plan.master_key_file {
        cmd.arg("--master-key-file").arg(mk);
    }

    cmd.env("LISTEN_FDS", plan.listener_count.to_string());
    cmd.env("LISTEN_PID", std::process::id().to_string());
    cmd.env(
        "UPTRAKIT_REEXEC_GENERATION",
        (plan.generation + 1).to_string(),
    );

    let err = cmd.exec();
    Err(rootcause::report!("exec failed: {err}"))
}
