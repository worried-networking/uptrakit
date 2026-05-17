pub(crate) mod listenfd;
pub(crate) mod sd_notify;
pub(crate) mod triage;

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
    /// Value passed to `--master-key-from` for master key resolution (e.g. `env:VAR` or `file:/path`).
    pub(crate) master_key_file: Option<String>,
    /// Number of file descriptors that were bound and should be inherited via
    /// `LISTEN_FDS` by the new process generation.
    pub(crate) listener_count: usize,
    /// Current process generation counter.  The new process will receive
    /// `generation + 1` via `UPTRAKIT_REEXEC_GENERATION`.
    pub(crate) generation: u64,
    /// Raw fd of the first (HTTPS) bound listener.
    ///
    /// The `listenfd` crate defaults `LISTEN_FDS_FIRST_FD` to 3, but the controller
    /// opens a database connection in Phase 3 (before socket binding in Phase 8b), so
    /// sockets are not at fd 3. Setting this env var tells the child where to find them.
    pub(crate) first_listener_fd: std::os::unix::io::RawFd,
}

/// Replace the current process image with a new instance of the same binary.
///
/// This function:
/// 1. Constructs a `Command` equivalent to the original invocation.
/// 2. Forwards `LISTEN_FDS` / `LISTEN_PID` so the new process can inherit
///    the already-bound TCP sockets (their `FD_CLOEXEC` flag is cleared at
///    bind time — see [`super::listenfd::clear_cloexec`]).
/// 3. Sets `UPTRAKIT_REEXEC_GENERATION` so observability tooling can track
///    how many times the process has re-execed.
/// 4. Calls `exec()` which, on success, replaces the process image and never
///    returns.  On failure the OS error is wrapped and returned.
///
/// # Errors
///
/// Returns an error if `exec()` fails (e.g. the binary path is no longer
/// accessible).  The error is always non-fatal from the caller's perspective
/// because the original process is still running.
pub(crate) fn perform_reexec(plan: &ReexecPlan) -> Result<std::convert::Infallible, Report> {
    use std::os::unix::process::CommandExt as _;

    // FD_CLOEXEC cleared at bind time — nothing to do here before exec.

    let mut cmd = std::process::Command::new(&plan.current_exe);
    cmd.arg("--config").arg(&plan.config_path);

    if let Some(mk) = &plan.master_key_file {
        cmd.arg("--master-key-from").arg(mk);
    }

    cmd.env("LISTEN_FDS", plan.listener_count.to_string());
    cmd.env("LISTEN_FDS_FIRST_FD", plan.first_listener_fd.to_string());
    cmd.env("LISTEN_PID", std::process::id().to_string());
    cmd.env(
        "UPTRAKIT_REEXEC_GENERATION",
        (plan.generation + 1).to_string(),
    );

    let err = cmd.exec();
    Err(rootcause::report!("exec failed: {err}"))
}
