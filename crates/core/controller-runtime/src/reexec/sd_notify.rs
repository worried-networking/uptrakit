use sd_notify::NotifyState;

/// Signal the service manager that the controller is ready to accept connections.
///
/// Sends `READY=1` to the socket named by `NOTIFY_SOCKET` when present.
/// On platforms without a service manager (macOS dev, FreeBSD without a
/// supervisor) the call is a no-op.  Also prints `READY` to stdout so that
/// non-systemd supervisors (e.g. s6, runit) can detect readiness via pipe.
pub(crate) fn signal_ready() {
    drop(sd_notify::notify(&[NotifyState::Ready]));
    println!("READY");
}

/// Update the service manager's status line for this unit.
///
/// Sends `STATUS=<text>` to `NOTIFY_SOCKET` when present; silently ignored
/// otherwise.
#[expect(dead_code, reason = "wired in a later graceful-reload task")]
pub(crate) fn signal_status(text: &str) {
    drop(sd_notify::notify(&[NotifyState::Status(text)]));
}
