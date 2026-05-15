//! Integration tests for the `ReloadCoordinator::run()` loop.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use time::OffsetDateTime;
use uptrakit_config_reload::ReloadCoordinator;
use uptrakit_config_reload::audit::ReloadAuditEvent;
use uptrakit_config_reload::coordinator::{ReloadRequest, ReloadSource};
use uptrakit_config_reload::delta::RuntimeConfigDelta;

// Minimal no-op reloadable that records apply calls.
struct CountingReloadable {
    count: Arc<std::sync::atomic::AtomicU32>,
}

#[async_trait::async_trait]
impl uptrakit_config_reload::reloadable::ReloadableErased for CountingReloadable {
    fn name(&self) -> &'static str {
        "counter"
    }
    fn validate(&self, _: &RuntimeConfigDelta) -> Result<(), rootcause::Report> {
        Ok(())
    }
    async fn apply(&self, delta: &RuntimeConfigDelta) -> Result<(), rootcause::Report> {
        if matches!(delta, RuntimeConfigDelta::Audit(_)) {
            self.count.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
    async fn revert(&self) -> Result<(), rootcause::Report> {
        Ok(())
    }
    async fn health_check(&self) -> Result<(), rootcause::Report> {
        Ok(())
    }
    fn rollback_window(&self) -> std::time::Duration {
        std::time::Duration::from_secs(1)
    }
}

#[tokio::test(flavor = "current_thread")]
async fn run_loop_routes_db_bump_to_run_cycle() {
    let count = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let reloadable = Arc::new(CountingReloadable {
        count: Arc::clone(&count),
    });
    let coord = ReloadCoordinator::new_for_test(vec![reloadable]);
    let handle = coord.handle();
    let _task = tokio::spawn(coord.run());

    handle
        .sender()
        .send(ReloadRequest {
            source: ReloadSource::DbBump {
                scope: uptrakit_config_reload::Scope::Global,
                sections: vec!["audit".to_string()],
            },
            timestamp: OffsetDateTime::now_utc(),
        })
        .await
        .unwrap();

    // Yield to let the coordinator task process the message.
    // Never use tokio::time::sleep — yield_now is deterministic and avoids
    // wall-clock coupling (testing.md: "never sleep on wall-clock").
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "apply should have been called once"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn run_loop_emits_requested_then_applied() {
    use tokio::sync::mpsc;

    let (audit_tx, mut audit_rx) = mpsc::unbounded_channel();
    let (coord, handle) = uptrakit_config_reload::ReloadCoordinator::new(
        vec![],
        audit_tx,
        Arc::new(uptrakit_config_reload::NoopAlertWriter),
    );
    let _task = tokio::spawn(coord.run());

    handle
        .sender()
        .send(ReloadRequest {
            source: ReloadSource::DbBump {
                scope: uptrakit_config_reload::Scope::Global,
                sections: vec![],
            },
            timestamp: OffsetDateTime::now_utc(),
        })
        .await
        .unwrap();

    // Yield to let the coordinator loop process the message without sleeping.
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }

    // Drain without timeout — events are already in the channel after yielding.
    let mut events = Vec::new();
    while let Ok(e) = audit_rx.try_recv() {
        events.push(e);
    }

    assert!(
        events
            .iter()
            .any(|e| matches!(e, ReloadAuditEvent::Requested { .. })),
        "expected Requested event; got: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ReloadAuditEvent::Applied { .. })),
        "expected Applied event; got: {events:?}"
    );
}

// This test uses start_paused = true because run_cycle's watchdog calls
// tokio::time::timeout with rollback_window. With paused time, we advance manually
// so the watchdog fires deterministically without wall-clock waiting.
#[tokio::test(start_paused = true)]
async fn run_loop_refuses_when_degraded() {
    use tokio::sync::mpsc;

    // Use a reloadable that passes apply but fails health_check (watchdog phase).
    // This makes apply_and_watch_phase call revert, which also fails, forcing Degraded.
    struct FailingWatchdogReloadable;
    #[async_trait::async_trait]
    impl uptrakit_config_reload::reloadable::ReloadableErased for FailingWatchdogReloadable {
        fn name(&self) -> &'static str {
            "watchdog-fail"
        }
        fn validate(&self, _: &RuntimeConfigDelta) -> Result<(), rootcause::Report> {
            Ok(())
        }
        async fn apply(&self, _: &RuntimeConfigDelta) -> Result<(), rootcause::Report> {
            Ok(())
        }
        async fn revert(&self) -> Result<(), rootcause::Report> {
            Err(rootcause::report!("revert failure — triggers Degraded"))
        }
        async fn health_check(&self) -> Result<(), rootcause::Report> {
            Err(rootcause::report!("watchdog health check failure"))
        }
        fn rollback_window(&self) -> std::time::Duration {
            std::time::Duration::from_millis(10)
        }
    }

    let (audit_tx, mut audit_rx) = mpsc::unbounded_channel();
    let (coord, handle) = uptrakit_config_reload::ReloadCoordinator::new(
        vec![Arc::new(FailingWatchdogReloadable)],
        audit_tx,
        Arc::new(uptrakit_config_reload::NoopAlertWriter),
    );
    let _task = tokio::spawn(coord.run());

    // Send first request; apply succeeds, watchdog (health_check) fails, revert fails → Degraded.
    handle
        .sender()
        .send(ReloadRequest {
            source: ReloadSource::DbBump {
                scope: uptrakit_config_reload::Scope::Global,
                sections: vec!["audit".to_string()],
            },
            timestamp: OffsetDateTime::now_utc(),
        })
        .await
        .unwrap();
    // Yield to let the coordinator process the request and run all phases.
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }

    // Send second request; Degraded coordinator should emit Refused.
    handle
        .sender()
        .send(ReloadRequest {
            source: ReloadSource::DbBump {
                scope: uptrakit_config_reload::Scope::Global,
                sections: vec!["config".to_string()],
            },
            timestamp: OffsetDateTime::now_utc(),
        })
        .await
        .unwrap();
    // Yield to let the coordinator check the Degraded state and emit Refused.
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }

    let mut events = Vec::new();
    while let Ok(e) = audit_rx.try_recv() {
        events.push(e);
    }

    assert!(
        events
            .iter()
            .any(|e| matches!(e, ReloadAuditEvent::Refused { .. })),
        "expected Refused event after Degraded; got: {events:?}"
    );
}
