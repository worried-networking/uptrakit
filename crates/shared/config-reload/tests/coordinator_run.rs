//! Integration tests for the `ReloadCoordinator::run()` loop.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use time::OffsetDateTime;
use uptrakit_config_reload::ReloadCoordinator;
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
