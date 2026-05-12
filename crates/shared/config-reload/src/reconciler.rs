//! Lock-free settings-version tracking for reload reconciliation.

use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::config::Scope;

/// Lock-free read-often / write-rarely counter cache for `settings_version`
/// tracking.
///
/// Uses [`ArcSwap`] so reads never block writers and vice-versa — appropriate
/// for the reconciler polling pattern where reads vastly outnumber writes.
#[derive(Clone)]
pub struct SettingsVersionCache {
    inner: Arc<ArcSwap<HashMap<Scope, u64>>>,
}

impl SettingsVersionCache {
    /// Create an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ArcSwap::new(Arc::new(HashMap::new()))),
        }
    }

    /// Return the stored version for `scope`, or `None` if not yet set.
    #[must_use]
    pub fn get(&self, scope: Scope) -> Option<u64> {
        self.inner.load().get(&scope).copied()
    }

    /// Store or replace the version for `scope`.
    pub fn update(&self, scope: Scope, version: u64) {
        let mut next: HashMap<Scope, u64> = (**self.inner.load()).clone();
        next.insert(scope, version);
        self.inner.store(Arc::new(next));
    }
}

impl Default for SettingsVersionCache {
    fn default() -> Self {
        Self::new()
    }
}
