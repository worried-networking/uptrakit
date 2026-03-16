//! Global workload claim registry for exclusive config-key ownership.
//!
//! Services participating in the workload claim protocol request ownership of
//! config keys (e.g. `"clients.{uuid}"`). Each config key is owned by exactly
//! one service instance at a time, preventing duplicate work (e.g. duplicate
//! MQTT publishes).
//!
//! The registry is shared across all WebSocket handlers on a single controller
//! instance and synchronized across controllers via NATS announcements.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use time::OffsetDateTime;
use uuid::Uuid;

/// Identifies the owner of a claimed config key.
#[derive(Debug, Clone)]
pub struct ClaimOwner {
    /// The service instance that owns the claim.
    pub service_id: Uuid,
    /// The controller that granted the claim.
    pub controller_id: Uuid,
    /// The tenant this config key belongs to.
    pub tenant_id: Uuid,
    /// When the claim was granted (for conflict resolution).
    pub claimed_at: OffsetDateTime,
}

/// Thread-safe global claim registry.
///
/// Uses `parking_lot::RwLock` per project conventions (sync lock in async code,
/// guard dropped before `.await`).
pub struct WorkloadClaimRegistry {
    inner: parking_lot::RwLock<ClaimRegistryInner>,
}

struct ClaimRegistryInner {
    /// config_key → owner
    claims: HashMap<String, ClaimOwner>,
    /// service_id → set of config_keys (reverse index for fast release)
    by_service: HashMap<Uuid, BTreeSet<String>>,
    /// tenant_id → set of service_ids serving at least one config for this tenant
    /// (derived index for SoftwareStates routing)
    tenant_services: HashMap<Uuid, BTreeSet<Uuid>>,
    /// service_id → map of config_keys the service wanted but were rejected
    /// (key → tenant_id, for proactive re-grant when claims are released)
    pending_desires: HashMap<Uuid, BTreeMap<String, Uuid>>,
}

impl ClaimRegistryInner {
    fn new() -> Self {
        Self {
            claims: HashMap::new(),
            by_service: HashMap::new(),
            tenant_services: HashMap::new(),
            pending_desires: HashMap::new(),
        }
    }

    /// Rebuild the `tenant_services` index for a given service from its claimed keys.
    fn rebuild_tenant_index_for_service(&mut self, service_id: Uuid) {
        // Remove this service from all tenant sets first
        self.tenant_services.retain(|_, services| {
            services.remove(&service_id);
            !services.is_empty()
        });

        // Re-add based on current claims
        if let Some(keys) = self.by_service.get(&service_id) {
            for key in keys {
                if let Some(owner) = self.claims.get(key) {
                    self.tenant_services
                        .entry(owner.tenant_id)
                        .or_default()
                        .insert(service_id);
                }
            }
        }
    }
}

/// Result of a claim attempt, indicating previous tenant set for the service
/// so the caller can determine which tenants are newly served.
pub struct ClaimResult {
    /// Config keys that were granted.
    pub granted: BTreeSet<String>,
    /// Config keys that were rejected (already claimed by another service).
    pub rejected: BTreeSet<String>,
    /// Config keys that were released (no longer desired by the service).
    pub released: BTreeMap<String, Uuid>,
    /// Tenants the service serves after this claim (for initial state push).
    pub tenants_after: BTreeSet<Uuid>,
    /// Tenants the service served before this claim.
    pub tenants_before: BTreeSet<Uuid>,
}

/// A revocation caused by applying a remote announcement.
pub struct Revocation {
    /// The local service whose claim was revoked.
    pub service_id: Uuid,
    /// The config keys that were revoked.
    pub revoked_keys: BTreeSet<String>,
}

impl WorkloadClaimRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            inner: parking_lot::RwLock::new(ClaimRegistryInner::new()),
        }
    }

    /// Attempt to claim config keys for a service (full replacement semantics).
    ///
    /// The `claims` map contains `config_key → tenant_id`. The controller diffs
    /// against the service's current grants:
    /// - Keys in `claims` but not currently granted: attempt to claim
    /// - Keys currently granted but not in `claims`: release
    /// - Keys in both: unchanged (kept)
    ///
    /// Returns the result including granted/rejected/released sets and tenant
    /// membership changes.
    pub fn try_claim(
        &self,
        service_id: Uuid,
        controller_id: Uuid,
        claims: BTreeMap<String, Uuid>,
    ) -> ClaimResult {
        let mut inner = self.inner.write();
        let now = OffsetDateTime::now_utc();

        // Snapshot tenants before
        let tenants_before = Self::tenants_for_service_inner(&inner, service_id);

        // Determine currently held keys
        let current_keys: BTreeSet<String> = inner
            .by_service
            .get(&service_id)
            .cloned()
            .unwrap_or_default();

        // Keys to release (held but no longer desired)
        let desired_keys: BTreeSet<String> = claims.keys().cloned().collect();
        let to_release: BTreeSet<String> =
            current_keys.difference(&desired_keys).cloned().collect();
        let mut released = BTreeMap::new();
        for key in &to_release {
            if let Some(owner) = inner.claims.remove(key) {
                released.insert(key.clone(), owner.tenant_id);
            }
            if let Some(svc_keys) = inner.by_service.get_mut(&service_id) {
                svc_keys.remove(key);
            }
        }

        // Keys to claim (desired but not held)
        let to_claim: BTreeSet<String> = desired_keys.difference(&current_keys).cloned().collect();
        let mut granted = BTreeSet::new();
        let mut rejected = BTreeSet::new();

        for key in &to_claim {
            if let Some(existing) = inner.claims.get(key) {
                // Already claimed by another service
                if existing.service_id != service_id {
                    rejected.insert(key.clone());
                    continue;
                }
            }
            // Grant the claim
            let tenant_id = claims[key];
            inner.claims.insert(
                key.clone(),
                ClaimOwner {
                    service_id,
                    controller_id,
                    tenant_id,
                    claimed_at: now,
                },
            );
            inner
                .by_service
                .entry(service_id)
                .or_default()
                .insert(key.clone());
            granted.insert(key.clone());
        }

        // Update pending desires: store rejected keys, remove previously rejected
        // keys that are now granted or no longer desired
        if rejected.is_empty() {
            inner.pending_desires.remove(&service_id);
        } else {
            let desires: BTreeMap<String, Uuid> = rejected
                .iter()
                .filter_map(|k| claims.get(k).map(|tid| (k.clone(), *tid)))
                .collect();
            inner.pending_desires.insert(service_id, desires);
        }

        // Rebuild tenant index for this service
        inner.rebuild_tenant_index_for_service(service_id);

        let tenants_after = Self::tenants_for_service_inner(&inner, service_id);

        ClaimResult {
            granted,
            rejected,
            released,
            tenants_after,
            tenants_before,
        }
    }

    /// Release all claims held by a service (e.g. on disconnect).
    ///
    /// Returns the released keys with their tenant IDs.
    pub fn release(&self, service_id: Uuid) -> BTreeMap<String, Uuid> {
        let mut inner = self.inner.write();
        let mut released = BTreeMap::new();

        if let Some(keys) = inner.by_service.remove(&service_id) {
            for key in keys {
                if let Some(owner) = inner.claims.remove(&key) {
                    released.insert(key, owner.tenant_id);
                }
            }
        }
        inner.pending_desires.remove(&service_id);
        inner.rebuild_tenant_index_for_service(service_id);

        released
    }

    /// Release specific keys held by a service.
    ///
    /// Returns the released keys with their tenant IDs.
    pub fn release_keys(
        &self,
        service_id: Uuid,
        keys: &BTreeSet<String>,
    ) -> BTreeMap<String, Uuid> {
        let mut inner = self.inner.write();
        let mut released = BTreeMap::new();

        for key in keys {
            if let Some(owner) = inner.claims.get(key) {
                if owner.service_id == service_id {
                    let owner = inner.claims.remove(key).unwrap();
                    released.insert(key.clone(), owner.tenant_id);
                    if let Some(svc_keys) = inner.by_service.get_mut(&service_id) {
                        svc_keys.remove(key);
                    }
                }
            }
        }

        if !released.is_empty() {
            inner.rebuild_tenant_index_for_service(service_id);
        }

        released
    }

    /// Apply a remote claim announcement from another controller.
    ///
    /// Returns any local revocations needed (when conflict is detected and
    /// the remote claim wins by timestamp/service_id comparison).
    pub fn apply_remote_announcement(
        &self,
        announcement_service_id: Uuid,
        announcement_controller_id: Uuid,
        claimed: &BTreeMap<String, Uuid>,
        released: &BTreeSet<String>,
        claimed_at: OffsetDateTime,
    ) -> Vec<Revocation> {
        let mut inner = self.inner.write();
        let mut revocations: HashMap<Uuid, BTreeSet<String>> = HashMap::new();

        // Process released keys
        for key in released {
            if let Some(owner) = inner.claims.get(key) {
                if owner.service_id == announcement_service_id {
                    inner.claims.remove(key);
                    if let Some(svc_keys) = inner.by_service.get_mut(&announcement_service_id) {
                        svc_keys.remove(key);
                    }
                }
            }
        }

        // Process claimed keys
        for (key, tenant_id) in claimed {
            if let Some(existing) = inner.claims.get(key) {
                if existing.service_id == announcement_service_id {
                    // Same service, update
                    // (may happen if service reconnected to different controller)
                } else {
                    // Conflict: compare (claimed_at, service_id)
                    let remote_wins = claimed_at < existing.claimed_at
                        || (claimed_at == existing.claimed_at
                            && announcement_service_id < existing.service_id);

                    if remote_wins {
                        // Revoke the local claim
                        let loser_id = existing.service_id;
                        revocations.entry(loser_id).or_default().insert(key.clone());
                        if let Some(svc_keys) = inner.by_service.get_mut(&loser_id) {
                            svc_keys.remove(key);
                        }
                    } else {
                        // Local claim wins, ignore remote
                        continue;
                    }
                }
            }

            // Insert or update the remote claim
            inner.claims.insert(
                key.clone(),
                ClaimOwner {
                    service_id: announcement_service_id,
                    controller_id: announcement_controller_id,
                    tenant_id: *tenant_id,
                    claimed_at,
                },
            );
            inner
                .by_service
                .entry(announcement_service_id)
                .or_default()
                .insert(key.clone());
        }

        // Rebuild tenant indexes for affected services
        inner.rebuild_tenant_index_for_service(announcement_service_id);
        for svc_id in revocations.keys() {
            inner.rebuild_tenant_index_for_service(*svc_id);
        }

        revocations
            .into_iter()
            .map(|(service_id, revoked_keys)| Revocation {
                service_id,
                revoked_keys,
            })
            .collect()
    }

    /// Apply a sync response from another controller (no conflict resolution).
    ///
    /// Used during controller startup to populate the global registry.
    pub fn apply_sync_response(
        &self,
        remote_controller_id: Uuid,
        claims: &BTreeMap<String, (Uuid, Uuid, OffsetDateTime)>,
    ) {
        let mut inner = self.inner.write();

        for (key, (service_id, tenant_id, claimed_at)) in claims {
            // Don't overwrite existing claims (first-come-first-served during sync)
            if inner.claims.contains_key(key) {
                continue;
            }
            inner.claims.insert(
                key.clone(),
                ClaimOwner {
                    service_id: *service_id,
                    controller_id: remote_controller_id,
                    tenant_id: *tenant_id,
                    claimed_at: *claimed_at,
                },
            );
            inner
                .by_service
                .entry(*service_id)
                .or_default()
                .insert(key.clone());
            inner
                .tenant_services
                .entry(*tenant_id)
                .or_default()
                .insert(*service_id);
        }
    }

    /// Find local services that have pending desires for the given keys.
    ///
    /// Returns `service_id → (config_key → tenant_id)` for services that
    /// previously had these keys rejected. Used for proactive re-grant.
    pub fn find_pending_desires_for_keys(
        &self,
        keys: &BTreeSet<String>,
    ) -> HashMap<Uuid, BTreeMap<String, Uuid>> {
        let inner = self.inner.read();
        let mut result: HashMap<Uuid, BTreeMap<String, Uuid>> = HashMap::new();

        for (svc_id, desires) in &inner.pending_desires {
            for (key, tenant_id) in desires {
                if keys.contains(key) {
                    result
                        .entry(*svc_id)
                        .or_default()
                        .insert(key.clone(), *tenant_id);
                }
            }
        }

        result
    }

    /// Returns the set of config keys currently granted to a service.
    pub fn service_claims(&self, service_id: Uuid) -> BTreeSet<String> {
        let inner = self.inner.read();
        inner
            .by_service
            .get(&service_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns all service IDs that hold at least one config for this tenant.
    pub fn services_for_tenant(&self, tenant_id: Uuid) -> BTreeSet<Uuid> {
        let inner = self.inner.read();
        inner
            .tenant_services
            .get(&tenant_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns the unique tenant IDs for a service's granted claims.
    pub fn tenants_for_service(&self, service_id: Uuid) -> BTreeSet<Uuid> {
        let inner = self.inner.read();
        Self::tenants_for_service_inner(&inner, service_id)
    }

    fn tenants_for_service_inner(inner: &ClaimRegistryInner, service_id: Uuid) -> BTreeSet<Uuid> {
        inner
            .by_service
            .get(&service_id)
            .map(|keys| {
                keys.iter()
                    .filter_map(|k| inner.claims.get(k).map(|o| o.tenant_id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns the local claims for this controller (used for sync responses).
    pub fn local_claims(
        &self,
        local_controller_id: Uuid,
    ) -> BTreeMap<String, (Uuid, Uuid, OffsetDateTime)> {
        let inner = self.inner.read();
        inner
            .claims
            .iter()
            .filter(|(_, owner)| owner.controller_id == local_controller_id)
            .map(|(key, owner)| {
                (
                    key.clone(),
                    (owner.service_id, owner.tenant_id, owner.claimed_at),
                )
            })
            .collect()
    }

    /// Returns `true` if any claims exist in the registry.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        let inner = self.inner.read();
        inner.claims.is_empty()
    }
}

impl Default for WorkloadClaimRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVC_A: Uuid = Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]);
    const SVC_B: Uuid = Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x02,
    ]);
    const CTRL_1: Uuid = Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
        0x01,
    ]);
    const CTRL_2: Uuid = Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
        0x02,
    ]);
    const TENANT_1: Uuid = Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
        0x01,
    ]);
    const TENANT_2: Uuid = Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
        0x02,
    ]);

    fn claims(pairs: &[(&str, Uuid)]) -> BTreeMap<String, Uuid> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn basic_claim_and_release() {
        let reg = WorkloadClaimRegistry::new();

        // Claim two keys for SVC_A
        let result = reg.try_claim(SVC_A, CTRL_1, claims(&[("k1", TENANT_1), ("k2", TENANT_1)]));
        assert_eq!(result.granted.len(), 2);
        assert!(result.rejected.is_empty());
        assert!(result.released.is_empty());
        assert!(result.tenants_after.contains(&TENANT_1));

        // Verify service_claims
        let svc_keys = reg.service_claims(SVC_A);
        assert!(svc_keys.contains("k1"));
        assert!(svc_keys.contains("k2"));

        // Verify tenant routing
        let tenant_svcs = reg.services_for_tenant(TENANT_1);
        assert!(tenant_svcs.contains(&SVC_A));

        // Release all
        let released = reg.release(SVC_A);
        assert_eq!(released.len(), 2);
        assert!(reg.is_empty());
        assert!(reg.services_for_tenant(TENANT_1).is_empty());
    }

    #[test]
    fn exclusive_claim_rejection() {
        let reg = WorkloadClaimRegistry::new();

        // SVC_A claims k1
        let r1 = reg.try_claim(SVC_A, CTRL_1, claims(&[("k1", TENANT_1)]));
        assert_eq!(r1.granted.len(), 1);

        // SVC_B tries to claim k1 — rejected
        let r2 = reg.try_claim(SVC_B, CTRL_1, claims(&[("k1", TENANT_1)]));
        assert!(r2.granted.is_empty());
        assert!(r2.rejected.contains("k1"));

        // SVC_B still has a pending desire
        let pending = reg.find_pending_desires_for_keys(&["k1".to_string()].into());
        assert!(pending.contains_key(&SVC_B));
    }

    #[test]
    fn full_replacement_semantics() {
        let reg = WorkloadClaimRegistry::new();

        // Initial claim: k1, k2
        reg.try_claim(SVC_A, CTRL_1, claims(&[("k1", TENANT_1), ("k2", TENANT_1)]));

        // Replace with k2, k3 — k1 should be released, k3 claimed
        let result = reg.try_claim(SVC_A, CTRL_1, claims(&[("k2", TENANT_1), ("k3", TENANT_1)]));

        assert!(result.granted.contains("k3"));
        assert!(!result.granted.contains("k2")); // k2 was already held
        assert!(result.released.contains_key("k1"));

        let keys = reg.service_claims(SVC_A);
        assert!(keys.contains("k2"));
        assert!(keys.contains("k3"));
        assert!(!keys.contains("k1"));
    }

    #[test]
    fn multi_tenant_routing() {
        let reg = WorkloadClaimRegistry::new();

        // SVC_A claims keys for two tenants
        let result = reg.try_claim(SVC_A, CTRL_1, claims(&[("k1", TENANT_1), ("k2", TENANT_2)]));
        assert_eq!(result.granted.len(), 2);

        // Both tenants route to SVC_A
        assert!(reg.services_for_tenant(TENANT_1).contains(&SVC_A));
        assert!(reg.services_for_tenant(TENANT_2).contains(&SVC_A));

        // SVC_B claims a different key for TENANT_1
        reg.try_claim(SVC_B, CTRL_1, claims(&[("k3", TENANT_1)]));

        // TENANT_1 now routes to both services
        let t1_svcs = reg.services_for_tenant(TENANT_1);
        assert!(t1_svcs.contains(&SVC_A));
        assert!(t1_svcs.contains(&SVC_B));
    }

    #[test]
    fn release_specific_keys() {
        let reg = WorkloadClaimRegistry::new();

        reg.try_claim(SVC_A, CTRL_1, claims(&[("k1", TENANT_1), ("k2", TENANT_2)]));

        // Release only k1
        let released = reg.release_keys(SVC_A, &["k1".to_string()].into());
        assert_eq!(released.len(), 1);
        assert_eq!(released["k1"], TENANT_1);

        // k2 still held
        let keys = reg.service_claims(SVC_A);
        assert!(keys.contains("k2"));
        assert!(!keys.contains("k1"));

        // TENANT_1 no longer routed to SVC_A (only had k1)
        assert!(!reg.services_for_tenant(TENANT_1).contains(&SVC_A));
        // TENANT_2 still routed
        assert!(reg.services_for_tenant(TENANT_2).contains(&SVC_A));
    }

    #[test]
    fn proactive_re_grant_after_release() {
        let reg = WorkloadClaimRegistry::new();

        // SVC_A claims k1
        reg.try_claim(SVC_A, CTRL_1, claims(&[("k1", TENANT_1)]));

        // SVC_B wants k1 — rejected
        let r = reg.try_claim(SVC_B, CTRL_1, claims(&[("k1", TENANT_1)]));
        assert!(r.rejected.contains("k1"));

        // SVC_A releases k1
        reg.release(SVC_A);

        // Now SVC_B's pending desire should be found
        let pending = reg.find_pending_desires_for_keys(&["k1".to_string()].into());
        assert!(pending.contains_key(&SVC_B));
        assert_eq!(pending[&SVC_B]["k1"], TENANT_1);
    }

    #[test]
    fn remote_announcement_no_conflict() {
        let reg = WorkloadClaimRegistry::new();
        let now = OffsetDateTime::now_utc();

        let revocations = reg.apply_remote_announcement(
            SVC_A,
            CTRL_2,
            &claims(&[("k1", TENANT_1)]),
            &BTreeSet::new(),
            now,
        );

        assert!(revocations.is_empty());
        assert!(reg.services_for_tenant(TENANT_1).contains(&SVC_A));
    }

    #[test]
    fn remote_announcement_conflict_resolution() {
        let reg = WorkloadClaimRegistry::new();
        let earlier = OffsetDateTime::now_utc() - time::Duration::seconds(10);

        // Local claim (uses current time, which is after `earlier`)
        reg.try_claim(SVC_A, CTRL_1, claims(&[("k1", TENANT_1)]));

        // Remote announcement at `earlier` timestamp — remote wins
        let revocations = reg.apply_remote_announcement(
            SVC_B,
            CTRL_2,
            &claims(&[("k1", TENANT_1)]),
            &BTreeSet::new(),
            earlier,
        );

        // SVC_A should be revoked
        assert_eq!(revocations.len(), 1);
        assert_eq!(revocations[0].service_id, SVC_A);
        assert!(revocations[0].revoked_keys.contains("k1"));

        // SVC_B now owns k1
        assert!(reg.services_for_tenant(TENANT_1).contains(&SVC_B));
        assert!(!reg.services_for_tenant(TENANT_1).contains(&SVC_A));
    }

    #[test]
    fn remote_announcement_conflict_local_wins() {
        let reg = WorkloadClaimRegistry::new();
        let earlier = OffsetDateTime::now_utc() - time::Duration::seconds(10);

        // Local claim at earlier timestamp
        // We need to manipulate the claim time. Let's use sync response for that.
        let mut sync_claims = BTreeMap::new();
        sync_claims.insert("k1".to_string(), (SVC_A, TENANT_1, earlier));
        reg.apply_sync_response(CTRL_1, &sync_claims);

        // Remote announcement at later timestamp — local wins
        let later = OffsetDateTime::now_utc();
        let revocations = reg.apply_remote_announcement(
            SVC_B,
            CTRL_2,
            &claims(&[("k1", TENANT_1)]),
            &BTreeSet::new(),
            later,
        );

        // No revocations — local claim wins
        assert!(revocations.is_empty());
        assert!(reg.services_for_tenant(TENANT_1).contains(&SVC_A));
    }

    #[test]
    fn remote_announcement_release() {
        let reg = WorkloadClaimRegistry::new();
        let now = OffsetDateTime::now_utc();

        // Remote claims k1
        reg.apply_remote_announcement(
            SVC_A,
            CTRL_2,
            &claims(&[("k1", TENANT_1)]),
            &BTreeSet::new(),
            now,
        );

        // Remote releases k1
        reg.apply_remote_announcement(
            SVC_A,
            CTRL_2,
            &BTreeMap::new(),
            &["k1".to_string()].into(),
            now,
        );

        assert!(reg.is_empty());
    }

    #[test]
    fn sync_response_populates_registry() {
        let reg = WorkloadClaimRegistry::new();
        let now = OffsetDateTime::now_utc();

        let mut sync_claims = BTreeMap::new();
        sync_claims.insert("k1".to_string(), (SVC_A, TENANT_1, now));
        sync_claims.insert("k2".to_string(), (SVC_B, TENANT_2, now));

        reg.apply_sync_response(CTRL_2, &sync_claims);

        assert!(reg.services_for_tenant(TENANT_1).contains(&SVC_A));
        assert!(reg.services_for_tenant(TENANT_2).contains(&SVC_B));

        // New local service trying to claim k1 should be rejected
        let result = reg.try_claim(SVC_B, CTRL_1, claims(&[("k1", TENANT_1)]));
        assert!(result.rejected.contains("k1"));
    }

    #[test]
    fn local_claims_returns_only_local() {
        let reg = WorkloadClaimRegistry::new();
        let now = OffsetDateTime::now_utc();

        // Local claim
        reg.try_claim(SVC_A, CTRL_1, claims(&[("k1", TENANT_1)]));

        // Remote claim via sync
        let mut sync_claims = BTreeMap::new();
        sync_claims.insert("k2".to_string(), (SVC_B, TENANT_2, now));
        reg.apply_sync_response(CTRL_2, &sync_claims);

        let local = reg.local_claims(CTRL_1);
        assert!(local.contains_key("k1"));
        assert!(!local.contains_key("k2"));
    }

    #[test]
    fn empty_claim_releases_all() {
        let reg = WorkloadClaimRegistry::new();

        reg.try_claim(SVC_A, CTRL_1, claims(&[("k1", TENANT_1), ("k2", TENANT_2)]));

        // Empty claim = release everything
        let result = reg.try_claim(SVC_A, CTRL_1, BTreeMap::new());
        assert!(result.granted.is_empty());
        assert_eq!(result.released.len(), 2);
        assert!(reg.service_claims(SVC_A).is_empty());
    }

    #[test]
    fn tenants_before_and_after() {
        let reg = WorkloadClaimRegistry::new();

        // Initial claim
        reg.try_claim(SVC_A, CTRL_1, claims(&[("k1", TENANT_1)]));

        // Add TENANT_2, keep TENANT_1
        let result = reg.try_claim(SVC_A, CTRL_1, claims(&[("k1", TENANT_1), ("k2", TENANT_2)]));

        assert!(result.tenants_before.contains(&TENANT_1));
        assert!(!result.tenants_before.contains(&TENANT_2));
        assert!(result.tenants_after.contains(&TENANT_1));
        assert!(result.tenants_after.contains(&TENANT_2));
    }

    #[test]
    fn pending_desires_cleared_on_successful_claim() {
        let reg = WorkloadClaimRegistry::new();

        // SVC_A claims k1
        reg.try_claim(SVC_A, CTRL_1, claims(&[("k1", TENANT_1)]));

        // SVC_B wants k1 — rejected
        reg.try_claim(SVC_B, CTRL_1, claims(&[("k1", TENANT_1)]));

        // SVC_B sends empty claim (no longer wants anything)
        reg.try_claim(SVC_B, CTRL_1, BTreeMap::new());

        // Pending desires should be cleared
        let pending = reg.find_pending_desires_for_keys(&["k1".to_string()].into());
        assert!(!pending.contains_key(&SVC_B));
    }
}
