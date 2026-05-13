//! Integration tests for the config-reload coordinator pipeline.
//!
//! These tests exercise the public API of `uptrakit_config_reload` together
//! with concrete `Reloadable` implementations, verifying that:
//!
//! 1. The coordinator processes `ReloadRequest`s and stays healthy.
//! 2. SIGHUP triggers a reload cycle that completes without degrading.
//! 3. Multiple reload requests in flight are processed without degrading.

use std::sync::Arc;
use std::time::Duration;

use time::OffsetDateTime;
use uptrakit_config_reload::ReloadCoordinator;
use uptrakit_config_reload::config::Scope;
use uptrakit_config_reload::coordinator::{CoordinatorState, ReloadRequest, ReloadSource};
use uptrakit_web_api_queries::reload::plugin_registry::PluginCatalogReloadable;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_coordinator() -> ReloadCoordinator {
    let (plugin, _rx) = PluginCatalogReloadable::new(Default::default());
    let mut coord = ReloadCoordinator::new_for_test(vec![]);
    coord.extend_reloadables(vec![Arc::new(plugin)]);
    coord
}

async fn wait_for_idle(
    handle: &uptrakit_config_reload::coordinator::ReloadCoordinatorHandle,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if matches!(handle.state(), CoordinatorState::Idle) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ---------------------------------------------------------------------------
// Test 1: manual DbBump request — coordinator stays Idle and healthy
// ---------------------------------------------------------------------------

/// Sending a `DbBump` request directly causes a reload cycle; coordinator
/// returns to `Idle` without degrading.
#[tokio::test(flavor = "multi_thread")]
async fn coordinator_handles_db_bump_request() {
    let coord = make_coordinator();
    let handle = coord.handle();
    tokio::spawn(coord.run());

    handle
        .sender()
        .send(ReloadRequest {
            source: ReloadSource::DbBump {
                scope: Scope::Global,
                sections: vec!["audit".into()],
            },
            timestamp: OffsetDateTime::now_utc(),
        })
        .await
        .expect("send");

    assert!(
        wait_for_idle(&handle, Duration::from_secs(5)).await,
        "coordinator did not return to Idle within 5 s"
    );
    assert!(
        !matches!(handle.state(), CoordinatorState::Degraded(_)),
        "coordinator entered Degraded after reload"
    );
}

// ---------------------------------------------------------------------------
// Test 2: multiple sequential requests all complete cleanly
// ---------------------------------------------------------------------------

/// Several reload requests queued back-to-back are each processed; coordinator
/// stays healthy throughout.
#[tokio::test(flavor = "multi_thread")]
async fn coordinator_handles_multiple_requests_cleanly() {
    let coord = make_coordinator();
    let handle = coord.handle();
    let sender = handle.sender();
    tokio::spawn(coord.run());

    for scope in [Scope::Global, Scope::Tenant(uuid::Uuid::now_v7())] {
        sender
            .send(ReloadRequest {
                source: ReloadSource::DbBump {
                    scope,
                    sections: vec!["audit".into()],
                },
                timestamp: OffsetDateTime::now_utc(),
            })
            .await
            .expect("send");
    }

    assert!(
        wait_for_idle(&handle, Duration::from_secs(5)).await,
        "coordinator did not drain queue within 5 s"
    );
    assert!(
        !matches!(handle.state(), CoordinatorState::Degraded(_)),
        "coordinator entered Degraded after multiple reloads"
    );
}

// ---------------------------------------------------------------------------
// Test 3: SIGHUP triggers a reload cycle (run in isolation)
// ---------------------------------------------------------------------------

/// Raising `SIGHUP` causes the sighup task to enqueue a reload request; the
/// coordinator processes it and returns to `Idle`.
///
/// Marked `#[ignore]` because it raises a process-wide signal and should be
/// run in isolation: `cargo test -p uptrakit-controller-runtime
/// coordinator_handles_sighup -- --ignored`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "sends SIGHUP to the whole process; run in isolation with -- --ignored"]
async fn coordinator_handles_sighup() {
    let coord = make_coordinator();
    let handle = coord.handle();
    tokio::spawn(coord.run());

    let _sighup = uptrakit_config_reload::triggers::sighup::spawn_sighup_task(handle.sender());

    // tokio::signal() registers the OS handler inside the spawned task body;
    // yield until that task has a chance to run and register the handler,
    // otherwise raise() delivers SIGHUP before the handler is installed and
    // the OS default action (terminate) fires.
    tokio::time::sleep(Duration::from_millis(50)).await;

    nix::sys::signal::raise(nix::sys::signal::Signal::SIGHUP).expect("raise SIGHUP");

    assert!(
        wait_for_idle(&handle, Duration::from_secs(5)).await,
        "coordinator did not return to Idle within 5 s after SIGHUP"
    );
    assert!(
        !matches!(handle.state(), CoordinatorState::Degraded(_)),
        "coordinator entered Degraded after SIGHUP reload"
    );
}
