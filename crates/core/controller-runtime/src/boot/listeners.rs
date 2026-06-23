//! Phase 8b: Claim inherited TCP sockets and pre-bind listeners.
//!
//! This phase must execute before any fd-allocating operations (PKI/TLS init,
//! file opens, etc.) that follow it.  The consecutive-FD invariant described
//! below is enforced by keeping the entire bind sequence in a single function.

use rootcause::prelude::*;
use std::os::unix::io::AsRawFd as _;

use crate::AppError;
use crate::boot::settings::SettingsBundle;

/// Bound TCP listeners produced by [`claim`] and consumed by the server tasks.
pub(crate) struct Listeners {
    pub https_std: std::net::TcpListener,
    pub pki_std_for_spawn: Option<std::net::TcpListener>,
    pub listener_count: usize,
    pub first_listener_fd: std::os::unix::io::RawFd,
}

/// Phase 8b: Claim inherited TCP sockets and pre-bind listeners.
///
/// This function is **synchronous** (`fn`, no `.await`).  It reads
/// `settings.reconciled.https_addr` and `settings.validated.pki_http_port`.
/// It consumes nothing from identity or PKI — those phases follow this one.
///
/// # CONSECUTIVE-FD INVARIANT
///
/// `perform_reexec` sets `LISTEN_FDS_FIRST_FD` to the raw fd of the HTTPS
/// socket and `LISTEN_FDS` to the listener count (1 or 2).  The `listenfd`
/// crate maps slot N to fd `FIRST + N`, so it assumes HTTPS and PKI fds are
/// consecutive (no gap).  This holds as long as no fd-allocating call (file
/// open, socket, dup, etc.) executes between the HTTPS bind and the PKI bind
/// inside this function.  Do not insert any such call in this block.
pub(crate) fn claim(settings: &SettingsBundle) -> crate::Result<Listeners> {
    // CONSECUTIVE-FD INVARIANT: `perform_reexec` sets `LISTEN_FDS_FIRST_FD` to the
    // raw fd of the HTTPS socket and `LISTEN_FDS` to the listener count (1 or 2).
    // The `listenfd` crate maps slot N to fd `FIRST + N`, so it assumes HTTPS and
    // PKI fds are consecutive (no gap). This holds as long as no fd-allocating call
    // (file open, socket, dup, etc.) executes between the HTTPS bind and the PKI bind
    // below. Do not insert any such call in this block.
    let inherited = crate::reexec::listenfd::take_inherited_listeners().unwrap_or_else(|e| {
        tracing::warn!("LISTEN_FDS claim failed: {e}; falling back to fresh bind");
        None
    });
    let (inherited_https, inherited_pki) = match inherited {
        Some(s) => {
            let https_std = s.https.into_std().map_err(|e| {
                rootcause::report!(AppError::Config(format!(
                    "into_std failed for inherited HTTPS socket: {e}"
                )))
            })?;
            let pki_std = s
                .pki
                .map(|p| {
                    p.into_std().map_err(|e| {
                        rootcause::report!(AppError::Config(format!(
                            "into_std failed for inherited PKI socket: {e}"
                        )))
                    })
                })
                .transpose()?;
            (Some(https_std), pki_std)
        }
        None => (None, None),
    };

    // Obtain the HTTPS socket — either freshly bound or inherited from LISTEN_FDS.
    // clear_cloexec is required on BOTH paths:
    //   fresh-bind: clears the flag for the *current* exec() (generation 0→1).
    //   inherited:  clears it again for the *next* exec() (generation N→N+1).
    //               Without this call on the inherited path, every second reexec
    //               fails silently — the kernel closes FD_CLOEXEC descriptors on
    //               exec() and the child receives empty LISTEN_FDS slots.
    // Do not move this call inside either match arm.
    let https_std = match inherited_https {
        Some(l) => l,
        None => {
            let l = std::net::TcpListener::bind(settings.reconciled.https_addr).map_err(|e| {
                report!(AppError::Config(format!(
                    "bind HTTPS {}: {e}",
                    settings.reconciled.https_addr
                )))
            })?;
            l.set_nonblocking(true)
                .map_err(|e| report!(AppError::Config(format!("set_nonblocking HTTPS: {e}"))))?;
            l
        }
    };
    // ORDERING: call clear_cloexec AFTER take_inherited_listeners(); listenfd
    // re-arms FD_CLOEXEC on every claimed socket, so clearing it again here is
    // required to ensure the fd survives exec() in subsequent reexec generations.
    crate::reexec::listenfd::clear_cloexec(&https_std)
        .map_err(|e| report!(AppError::Config(format!("clear_cloexec HTTPS: {e}"))))?;

    // Record the fd so perform_reexec can set LISTEN_FDS_FIRST_FD.
    // The database was opened before the socket, so sockets are not at fd 3.
    let first_listener_fd = https_std.as_raw_fd();

    // Obtain the PKI socket (if enabled) — either freshly bound or inherited.
    // Same invariant as HTTPS: clear_cloexec is required on both paths.
    // Do not move the call inside either match arm.
    let (listener_count, pki_std_for_spawn): (usize, Option<std::net::TcpListener>) =
        if let Some(pki_port) = settings.validated.pki_http_port {
            let pki_std = match inherited_pki {
                Some(l) => l,
                None => {
                    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], pki_port));
                    let l = std::net::TcpListener::bind(addr).map_err(|e| {
                        report!(AppError::Config(format!("bind PKI HTTP {addr}: {e}")))
                    })?;
                    l.set_nonblocking(true).map_err(|e| {
                        report!(AppError::Config(format!("set_nonblocking PKI: {e}")))
                    })?;
                    l
                }
            };
            // ORDERING: same invariant as the HTTPS clear_cloexec above.
            // Both sockets must be cleared after take_inherited_listeners().
            crate::reexec::listenfd::clear_cloexec(&pki_std)
                .map_err(|e| report!(AppError::Config(format!("clear_cloexec PKI: {e}"))))?;
            (2, Some(pki_std))
        } else {
            // PKI disabled: only HTTPS socket inherited (1 FD).
            (1, None)
        };

    Ok(Listeners {
        https_std,
        pki_std_for_spawn,
        listener_count,
        first_listener_fd,
    })
}
