//! Helpers for claiming inherited TCP sockets passed via `LISTEN_FDS`.

use listenfd::ListenFd;
use rootcause::Report;
use tokio::net::TcpListener;

/// Tokio TCP listeners claimed from `LISTEN_FDS` at process start.
pub(crate) struct InheritedSockets {
    pub(crate) https: TcpListener,
    /// `None` when the process was started without PKI HTTP enabled.
    pub(crate) pki: Option<TcpListener>,
}

/// Attempt to claim inherited TCP listeners from the environment.
///
/// Returns `Ok(None)` when `LISTEN_FDS` is absent or zero (normal cold-start).
/// Returns `Err` when the count is neither 1 (HTTPS only) nor 2 (HTTPS + PKI),
/// or a listener cannot be converted.
pub(crate) fn take_inherited_listeners() -> Result<Option<InheritedSockets>, Report> {
    let mut lf = ListenFd::from_env();
    let count = lf.len();
    if count == 0 {
        return Ok(None);
    }
    if count != 1 && count != 2 {
        return Err(rootcause::report!(
            "LISTEN_FDS={count} but binary expects 1 (no PKI) or 2 (with PKI)"
        ));
    }
    let https = take_one(&mut lf, 0, "Https")?;
    https
        .set_nonblocking(true)
        .map_err(|e| rootcause::report!("set_nonblocking failed for Https slot: {e}"))?;
    let https = TcpListener::from_std(https)
        .map_err(|e| rootcause::report!("from_std failed for Https slot: {e}"))?;

    let pki = if count == 2 {
        let p = take_one(&mut lf, 1, "Pki")?;
        p.set_nonblocking(true)
            .map_err(|e| rootcause::report!("set_nonblocking failed for Pki slot: {e}"))?;
        let p = TcpListener::from_std(p)
            .map_err(|e| rootcause::report!("from_std failed for Pki slot: {e}"))?;
        Some(p)
    } else {
        None
    };

    Ok(Some(InheritedSockets { https, pki }))
}

fn take_one(
    lf: &mut ListenFd,
    idx: usize,
    slot: &'static str,
) -> Result<std::net::TcpListener, Report> {
    lf.take_tcp_listener(idx)
        .map_err(|e| rootcause::report!("take_tcp_listener({idx}) failed: {e}"))?
        .ok_or_else(|| rootcause::report!("LISTEN_FDS slot {idx} ({slot}) empty"))
}

/// Return the current process reexec generation.
///
/// Reads `UPTRAKIT_REEXEC_GENERATION` set by the previous generation's
/// `perform_reexec`. Returns `0` on cold start (env var absent or unparseable).
pub(crate) fn current_generation() -> u64 {
    std::env::var("UPTRAKIT_REEXEC_GENERATION")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Clear the `FD_CLOEXEC` flag on a raw file descriptor so it survives `exec()`.
///
/// Called by the reexec path for each bound listener before replacing the
/// process image.  Without this step the OS closes all `O_CLOEXEC` descriptors
/// on exec and the new process image receives empty `LISTEN_FDS` slots.
///
/// # Errors
///
/// Returns an error if the `fcntl` call fails (e.g. the file descriptor is
/// invalid or not open).
///
/// # Safety
///
/// The caller must ensure `fd` is a valid, open file descriptor for the
/// lifetime of this call.  `RawFd` is unguarded; passing a closed or
/// reused descriptor yields undefined behaviour in the kernel call.
pub(crate) fn clear_cloexec_raw(fd: std::os::unix::io::RawFd) -> Result<(), Report> {
    use std::os::unix::io::BorrowedFd;

    use nix::fcntl::{FcntlArg, FdFlag, fcntl};

    // SAFETY: The caller guarantees `fd` is valid and open.  We borrow it
    // only for the duration of the `fcntl` call.
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    fcntl(borrowed, FcntlArg::F_SETFD(FdFlag::empty()))
        .map_err(|e| rootcause::report!("fcntl F_SETFD on fd {fd}: {e}"))?;
    Ok(())
}
