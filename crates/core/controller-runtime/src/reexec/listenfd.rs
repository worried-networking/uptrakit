//! Helpers for claiming inherited TCP sockets passed via `LISTEN_FDS`.
//!
//! This module is fully implemented but not yet wired into the startup path;
//! the integration happens in a later graceful-reload task.
#![expect(
    dead_code,
    reason = "wired into startup path in a later graceful-reload task"
)]

use listenfd::ListenFd;
use rootcause::Report;
use tokio::net::TcpListener;

/// Discriminant for each inherited file-descriptor slot.
///
/// The `#[repr(usize)]` matches the positional index used by
/// `ListenFd::take_tcp_listener(idx)`.  Variants must be kept in sync with
/// [`INHERITED_SLOT_COUNT`].
#[repr(usize)]
pub(crate) enum ListenerSlot {
    Https = 0,
    Pki = 1,
}

/// Expected number of file descriptors passed via `LISTEN_FDS`.
///
/// Must equal the number of variants in [`ListenerSlot`].
pub(crate) const INHERITED_SLOT_COUNT: usize = 2;

const _: () = assert!(
    INHERITED_SLOT_COUNT == 2,
    "ListenerSlot count out of sync with INHERITED_SLOT_COUNT; update both together"
);

/// Tokio TCP listeners claimed from `LISTEN_FDS` at process start.
pub(crate) struct InheritedSockets {
    pub(crate) https: TcpListener,
    pub(crate) pki: TcpListener,
}

/// Attempt to claim inherited TCP listeners from the environment.
///
/// Returns `Ok(None)` when `LISTEN_FDS` is absent or zero (normal cold-start).
/// Returns `Err` when the count does not match [`INHERITED_SLOT_COUNT`] or a
/// listener cannot be converted.
pub(crate) fn take_inherited_listeners() -> Result<Option<InheritedSockets>, Report> {
    let mut lf = ListenFd::from_env();
    if lf.len() == 0 {
        return Ok(None);
    }
    if lf.len() != INHERITED_SLOT_COUNT {
        return Err(rootcause::report!(
            "LISTEN_FDS={} but binary expects {INHERITED_SLOT_COUNT}",
            lf.len()
        ));
    }
    let https = take_one(&mut lf, 0, slot_name(&ListenerSlot::Https))?;
    let pki = take_one(&mut lf, 1, slot_name(&ListenerSlot::Pki))?;
    drop(https.set_nonblocking(true));
    drop(pki.set_nonblocking(true));
    let https = TcpListener::from_std(https)
        .map_err(|e| rootcause::report!("from_std failed for Https slot: {e}"))?;
    let pki = TcpListener::from_std(pki)
        .map_err(|e| rootcause::report!("from_std failed for Pki slot: {e}"))?;
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

fn slot_name(slot: &ListenerSlot) -> &'static str {
    match slot {
        ListenerSlot::Https => "Https",
        ListenerSlot::Pki => "Pki",
    }
}
