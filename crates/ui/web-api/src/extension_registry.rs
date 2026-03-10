//! Registry for tracking UI extensions contributed by plugins and connected services.
//!
//! The [`ExtensionRegistry`] merges compile-time plugin extensions with runtime
//! service-provided extensions, deduplicates by extension ID, and supports
//! provider routing for action dispatch.
//!
//! ## Action library
//!
//! Actions are stored in a separate catalogue keyed by source `app_name`.
//! When returning manifests via [`all_manifests_with_actions`], the registry
//! attaches the resolved action catalogue for each extension's source.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use parking_lot::Mutex;
use uuid::Uuid;

use uptrakit_internal_wire::extension::{ActionDef, ExtensionManifest};

// ── Private types ───────────────────────────────────────────────────────────

/// Internal bookkeeping for a service-provided extension.
struct ExtensionEntry {
    /// The extension manifest (kept from the first registering service).
    manifest: ExtensionManifest,
    /// The `app_name` shared by all services providing this extension.
    app_name: String,
    /// Set of service instance IDs currently providing this extension.
    providers: BTreeSet<Uuid>,
}

// ── Public types ────────────────────────────────────────────────────────────

/// An extension manifest together with its resolved action catalogue.
#[derive(Debug, Clone)]
pub struct ResolvedExtension {
    /// The extension manifest.
    pub manifest: ExtensionManifest,
    /// All actions available to this extension (from the source's action library).
    pub actions: Vec<ActionDef>,
}

/// Describes who owns a given extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionOwner {
    /// The extension is provided by a compiled-in plugin.
    Plugin,
    /// The extension is provided by one or more connected service instances.
    Service {
        /// Service instance IDs currently providing this extension.
        providers: Vec<Uuid>,
    },
    /// The extension is provided by the notification plugin registry.
    Notification,
    /// No extension with this ID is registered.
    NotFound,
}

/// Errors returned by [`ExtensionRegistry`] operations.
#[derive(Debug, Clone)]
pub enum ExtensionRegistryError {
    /// A service tried to register an extension ID that already exists under a
    /// different `app_name`.
    ConflictingAppName {
        /// The extension ID that caused the conflict.
        extension_id: String,
        /// The `app_name` already associated with this extension ID.
        existing_app_name: String,
        /// The `app_name` the incoming service tried to register.
        incoming_app_name: String,
    },
}

impl fmt::Display for ExtensionRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConflictingAppName {
                extension_id,
                existing_app_name,
                incoming_app_name,
            } => {
                write!(
                    f,
                    "extension '{extension_id}' is already registered by app '{existing_app_name}', \
                     but service with app name '{incoming_app_name}' attempted to register the same ID"
                )
            }
        }
    }
}

impl std::error::Error for ExtensionRegistryError {}

/// Registry that tracks UI extensions from both compile-time plugins and
/// runtime service instances.
///
/// Thread-safe: internal state is protected by [`parking_lot::Mutex`].
pub struct ExtensionRegistry {
    /// Extensions provided by compiled-in plugins (immutable after construction).
    plugin_extensions: Vec<ExtensionManifest>,
    /// Actions provided by compiled-in plugins (immutable after construction).
    plugin_actions: Vec<ActionDef>,
    /// Extensions provided by the notification plugin registry (immutable after construction).
    notification_extensions: Vec<ExtensionManifest>,
    /// Actions provided by the notification plugin registry (immutable after construction).
    notification_actions: Vec<ActionDef>,
    /// Service-provided extensions, keyed by extension ID.
    service_extensions: Mutex<HashMap<String, ExtensionEntry>>,
    /// Reverse index: service instance ID to the extension IDs it provides.
    service_index: Mutex<HashMap<Uuid, Vec<String>>>,
    /// Action libraries keyed by service `app_name`.
    action_catalogs: Mutex<HashMap<String, Vec<ActionDef>>>,
    /// Reverse index: service instance ID to its `app_name` (for cleanup).
    service_app_names: Mutex<HashMap<Uuid, String>>,
    /// Per-service-instance ECIES encryption public keys (base64-encoded P-256).
    ///
    /// Stored separately from `service_extensions` because the key is
    /// per-instance, not per-extension — one service may provide multiple
    /// extensions, all sharing the same key.
    encryption_keys: Mutex<HashMap<Uuid, String>>,
}

impl ExtensionRegistry {
    /// Creates a new registry with the given compile-time plugin extensions,
    /// plugin actions, notification extensions, and notification actions.
    pub fn new(
        plugin_extensions: Vec<ExtensionManifest>,
        plugin_actions: Vec<ActionDef>,
        notification_extensions: Vec<ExtensionManifest>,
        notification_actions: Vec<ActionDef>,
    ) -> Self {
        Self {
            plugin_extensions,
            plugin_actions,
            notification_extensions,
            notification_actions,
            service_extensions: Mutex::new(HashMap::new()),
            service_index: Mutex::new(HashMap::new()),
            action_catalogs: Mutex::new(HashMap::new()),
            service_app_names: Mutex::new(HashMap::new()),
            encryption_keys: Mutex::new(HashMap::new()),
        }
    }

    /// Registers a service instance as a provider of the given extensions.
    ///
    /// For each manifest:
    /// - If the extension ID already exists with a **different** `app_name`, an
    ///   error is returned and no changes are applied.
    /// - If the extension ID already exists with the **same** `app_name`, the
    ///   service is added to the provider set.
    /// - If the extension ID is new, a new entry is created.
    pub fn register_service(
        &self,
        service_id: Uuid,
        service_app_name: &str,
        manifests: Vec<ExtensionManifest>,
        encryption_public_key: Option<String>,
    ) -> Result<(), ExtensionRegistryError> {
        let mut extensions = self.service_extensions.lock();

        // Pre-validate all manifests before mutating state.
        for manifest in &manifests {
            if let Some(entry) = extensions.get(&manifest.id)
                && entry.app_name != service_app_name
            {
                return Err(ExtensionRegistryError::ConflictingAppName {
                    extension_id: manifest.id.clone(),
                    existing_app_name: entry.app_name.clone(),
                    incoming_app_name: service_app_name.to_string(),
                });
            }
        }

        // All checks passed — apply mutations.
        let mut index = self.service_index.lock();
        let ext_ids = index.entry(service_id).or_default();

        for manifest in manifests {
            let ext_id = manifest.id.clone();

            extensions
                .entry(ext_id.clone())
                .and_modify(|entry| {
                    entry.providers.insert(service_id);
                })
                .or_insert_with(|| ExtensionEntry {
                    manifest,
                    app_name: service_app_name.to_string(),
                    providers: {
                        let mut set = BTreeSet::new();
                        set.insert(service_id);
                        set
                    },
                });

            if !ext_ids.contains(&ext_id) {
                ext_ids.push(ext_id);
            }
        }

        // Store the encryption public key (if provided).
        if let Some(key) = encryption_public_key {
            self.encryption_keys.lock().insert(service_id, key);
        }

        // Track app_name for this service instance (for cleanup).
        self.service_app_names
            .lock()
            .insert(service_id, service_app_name.to_string());

        Ok(())
    }

    /// Registers an action library for a service instance.
    ///
    /// The actions are stored under the service's `app_name`. Subsequent calls
    /// from any service with the same `app_name` replace the previous set.
    pub fn register_actions(
        &self,
        service_id: Uuid,
        service_app_name: &str,
        actions: Vec<ActionDef>,
    ) {
        self.action_catalogs
            .lock()
            .insert(service_app_name.to_string(), actions);
        self.service_app_names
            .lock()
            .insert(service_id, service_app_name.to_string());
    }

    /// Removes a service instance from all extensions it provides.
    ///
    /// Extensions with no remaining providers are removed entirely.
    /// If no other service with the same `app_name` remains, the action
    /// catalogue for that `app_name` is also removed.
    pub fn unregister_service(&self, service_id: &Uuid) {
        self.encryption_keys.lock().remove(service_id);

        let app_name = self.service_app_names.lock().remove(service_id);

        let mut index = self.service_index.lock();
        let Some(ext_ids) = index.remove(service_id) else {
            return;
        };

        let mut extensions = self.service_extensions.lock();
        for ext_id in ext_ids {
            let remove = if let Some(entry) = extensions.get_mut(&ext_id) {
                entry.providers.remove(service_id);
                entry.providers.is_empty()
            } else {
                false
            };

            if remove {
                extensions.remove(&ext_id);
            }
        }

        // Clean up action catalogue if no more services from this app_name.
        if let Some(app) = app_name {
            let any_remaining = self
                .service_app_names
                .lock()
                .values()
                .any(|name| *name == app);
            if !any_remaining {
                self.action_catalogs.lock().remove(&app);
            }
        }
    }

    /// Returns all known extension manifests (plugin + service), deduplicated
    /// by extension ID. Plugin extensions take precedence.
    ///
    /// Manifests are sorted by `(priority, label)`.
    pub fn all_manifests(&self) -> Vec<ExtensionManifest> {
        let mut seen: HashMap<&str, ()> = HashMap::new();
        let mut result = Vec::new();

        // Plugin extensions first.
        for manifest in &self.plugin_extensions {
            seen.insert(&manifest.id, ());
            result.push(manifest.clone());
        }

        // Notification extensions next.
        for manifest in &self.notification_extensions {
            if !seen.contains_key(manifest.id.as_str()) {
                seen.insert(&manifest.id, ());
                result.push(manifest.clone());
            }
        }

        // Service extensions, skipping IDs already covered.
        let extensions = self.service_extensions.lock();
        for (ext_id, entry) in &*extensions {
            if !seen.contains_key(ext_id.as_str()) {
                result.push(entry.manifest.clone());
            }
        }

        result.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| a.label.cmp(&b.label))
        });

        result
    }

    /// Returns all known extensions with their resolved action catalogues,
    /// sorted by `(priority, label)`.
    pub fn all_manifests_with_actions(&self) -> Vec<ResolvedExtension> {
        let mut seen: HashMap<String, ()> = HashMap::new();
        let mut result = Vec::new();

        // Plugin extensions first (use plugin_actions).
        for manifest in &self.plugin_extensions {
            seen.insert(manifest.id.clone(), ());
            result.push(ResolvedExtension {
                manifest: manifest.clone(),
                actions: self.plugin_actions.clone(),
            });
        }

        // Notification extensions (use notification_actions).
        for manifest in &self.notification_extensions {
            if !seen.contains_key(manifest.id.as_str()) {
                seen.insert(manifest.id.clone(), ());
                result.push(ResolvedExtension {
                    manifest: manifest.clone(),
                    actions: self.notification_actions.clone(),
                });
            }
        }

        // Service extensions, skipping IDs already covered.
        let extensions = self.service_extensions.lock();
        let catalogs = self.action_catalogs.lock();
        for (ext_id, entry) in &*extensions {
            if !seen.contains_key(ext_id.as_str()) {
                let actions = catalogs.get(&entry.app_name).cloned().unwrap_or_default();
                result.push(ResolvedExtension {
                    manifest: entry.manifest.clone(),
                    actions,
                });
            }
        }

        result.sort_by(|a, b| {
            a.manifest
                .priority
                .cmp(&b.manifest.priority)
                .then_with(|| a.manifest.label.cmp(&b.manifest.label))
        });

        result
    }

    /// Determines the owner of the given extension ID.
    pub fn find_owner(&self, extension_id: &str) -> ExtensionOwner {
        // Check plugin extensions first.
        if self.plugin_extensions.iter().any(|m| m.id == extension_id) {
            return ExtensionOwner::Plugin;
        }

        // Check notification extensions.
        if self
            .notification_extensions
            .iter()
            .any(|m| m.id == extension_id)
        {
            return ExtensionOwner::Notification;
        }

        // Check service extensions.
        let extensions = self.service_extensions.lock();
        if let Some(entry) = extensions.get(extension_id) {
            return ExtensionOwner::Service {
                providers: entry.providers.iter().copied().collect(),
            };
        }

        ExtensionOwner::NotFound
    }

    /// Returns the list of service instance IDs providing the given extension.
    ///
    /// Returns an empty list if the extension is not provided by any service
    /// (including if it is a plugin-only extension).
    pub fn providers(&self, extension_id: &str) -> Vec<Uuid> {
        let extensions = self.service_extensions.lock();
        extensions
            .get(extension_id)
            .map(|entry| entry.providers.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Picks a single service provider for the given extension.
    ///
    /// If `preferred` is `Some` and that service is in the provider set, it is
    /// returned. Otherwise the first provider (by `BTreeSet` ordering) is used.
    ///
    /// Returns `None` if no service provides this extension.
    pub fn pick_provider(&self, extension_id: &str, preferred: Option<Uuid>) -> Option<Uuid> {
        let extensions = self.service_extensions.lock();
        let entry = extensions.get(extension_id)?;

        if let Some(pref) = preferred
            && entry.providers.contains(&pref)
        {
            return Some(pref);
        }

        entry.providers.iter().next().copied()
    }

    /// Returns the ECIES encryption public key for a given service instance.
    ///
    /// Returns `None` if the service has not registered a key (or is not known).
    pub fn encryption_public_key(&self, service_id: &Uuid) -> Option<String> {
        self.encryption_keys.lock().get(service_id).cloned()
    }

    /// Returns the action catalogue for the source that owns the given extension.
    ///
    /// For plugin extensions, returns the plugin action catalogue.
    /// For service extensions, returns the service app_name's action catalogue.
    /// Returns an empty vec if the extension is not found.
    pub fn actions_for_extension(&self, extension_id: &str) -> Vec<ActionDef> {
        // Check plugin extensions first.
        if self.plugin_extensions.iter().any(|m| m.id == extension_id) {
            return self.plugin_actions.clone();
        }

        // Check notification extensions.
        if self
            .notification_extensions
            .iter()
            .any(|m| m.id == extension_id)
        {
            return self.notification_actions.clone();
        }

        // Check service extensions.
        let extensions = self.service_extensions.lock();
        if let Some(entry) = extensions.get(extension_id) {
            let catalogs = self.action_catalogs.lock();
            return catalogs.get(&entry.app_name).cloned().unwrap_or_default();
        }

        vec![]
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a simple test manifest with the given ID.
    fn test_manifest(id: &str) -> ExtensionManifest {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "label": format!("Test {id}"),
            "priority": 0,
            "placement": {
                "type": "page",
                "nav_section": "test"
            },
            "targeting": "universal",
            "ui": {
                "type": "data_table",
                "columns": [{ "key": "col", "label": "Column" }],
                "data_action": "list"
            }
        }))
        .expect("test manifest JSON should be valid")
    }

    /// Helper to create a unique UUID for tests.
    fn test_uuid() -> Uuid {
        Uuid::now_v7()
    }

    #[test]
    fn register_single_service_with_extensions() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        let svc = test_uuid();

        registry
            .register_service(
                svc,
                "my-app",
                vec![test_manifest("ext.one"), test_manifest("ext.two")],
                None,
            )
            .expect("register should succeed");

        let manifests = registry.all_manifests();
        assert_eq!(manifests.len(), 2);

        let providers = registry.providers("ext.one");
        assert_eq!(providers, vec![svc]);
    }

    #[test]
    fn register_multiple_services_same_app_name() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        let svc1 = test_uuid();
        let svc2 = test_uuid();

        registry
            .register_service(svc1, "my-app", vec![test_manifest("ext.shared")], None)
            .expect("register should succeed");
        registry
            .register_service(svc2, "my-app", vec![test_manifest("ext.shared")], None)
            .expect("register should succeed");

        // Only one manifest (deduplicated).
        let manifests = registry.all_manifests();
        assert_eq!(manifests.len(), 1);

        // Both services are providers.
        let mut providers = registry.providers("ext.shared");
        providers.sort();
        let mut expected = vec![svc1, svc2];
        expected.sort();
        assert_eq!(providers, expected);
    }

    #[test]
    fn register_conflicting_app_name_returns_error() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        let svc1 = test_uuid();
        let svc2 = test_uuid();

        registry
            .register_service(svc1, "app-alpha", vec![test_manifest("ext.conflict")], None)
            .expect("register should succeed");

        let err = registry
            .register_service(svc2, "app-beta", vec![test_manifest("ext.conflict")], None)
            .unwrap_err();

        match err {
            ExtensionRegistryError::ConflictingAppName {
                extension_id,
                existing_app_name,
                incoming_app_name,
            } => {
                assert_eq!(extension_id, "ext.conflict");
                assert_eq!(existing_app_name, "app-alpha");
                assert_eq!(incoming_app_name, "app-beta");
            }
        }

        // Original registration should be intact.
        assert_eq!(registry.providers("ext.conflict"), vec![svc1]);
    }

    #[test]
    fn unregister_removes_provider_and_cleans_empty_entries() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        let svc1 = test_uuid();
        let svc2 = test_uuid();

        registry
            .register_service(svc1, "app", vec![test_manifest("ext.a")], None)
            .expect("register should succeed");
        registry
            .register_service(svc2, "app", vec![test_manifest("ext.a")], None)
            .expect("register should succeed");

        // Remove one provider — entry should remain.
        registry.unregister_service(&svc1);
        assert_eq!(registry.providers("ext.a"), vec![svc2]);
        assert_eq!(registry.all_manifests().len(), 1);

        // Remove last provider — entry should be cleaned up.
        registry.unregister_service(&svc2);
        assert!(registry.providers("ext.a").is_empty());
        assert!(registry.all_manifests().is_empty());
    }

    #[test]
    fn unregister_nonexistent_service_is_noop() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        registry.unregister_service(&test_uuid());
        assert!(registry.all_manifests().is_empty());
    }

    #[test]
    fn all_manifests_includes_plugin_and_service_extensions() {
        let plugin_manifest = test_manifest("plugin.ext");
        let registry =
            ExtensionRegistry::new(vec![plugin_manifest.clone()], vec![], vec![], vec![]);
        let svc = test_uuid();

        registry
            .register_service(svc, "svc-app", vec![test_manifest("svc.ext")], None)
            .expect("register should succeed");

        let manifests = registry.all_manifests();
        assert_eq!(manifests.len(), 2);

        let ids: Vec<&str> = manifests.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"plugin.ext"));
        assert!(ids.contains(&"svc.ext"));
    }

    #[test]
    fn all_manifests_deduplicates_plugin_over_service() {
        let manifest = test_manifest("shared.ext");
        let registry = ExtensionRegistry::new(vec![manifest.clone()], vec![], vec![], vec![]);
        let svc = test_uuid();

        // Service registers the same extension ID.
        registry
            .register_service(svc, "svc-app", vec![test_manifest("shared.ext")], None)
            .expect("register should succeed");

        // Should only appear once (plugin takes precedence).
        let manifests = registry.all_manifests();
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].id, "shared.ext");
    }

    #[test]
    fn find_owner_plugin() {
        let registry =
            ExtensionRegistry::new(vec![test_manifest("plugin.only")], vec![], vec![], vec![]);
        assert_eq!(registry.find_owner("plugin.only"), ExtensionOwner::Plugin);
    }

    #[test]
    fn find_owner_service() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        let svc = test_uuid();

        registry
            .register_service(svc, "app", vec![test_manifest("svc.only")], None)
            .expect("register should succeed");

        assert_eq!(
            registry.find_owner("svc.only"),
            ExtensionOwner::Service {
                providers: vec![svc],
            }
        );
    }

    #[test]
    fn find_owner_not_found() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        assert_eq!(registry.find_owner("nonexistent"), ExtensionOwner::NotFound);
    }

    #[test]
    fn find_owner_plugin_takes_precedence_over_service() {
        let registry =
            ExtensionRegistry::new(vec![test_manifest("dual.ext")], vec![], vec![], vec![]);
        let svc = test_uuid();

        registry
            .register_service(svc, "app", vec![test_manifest("dual.ext")], None)
            .expect("register should succeed");

        // Plugin ownership takes precedence.
        assert_eq!(registry.find_owner("dual.ext"), ExtensionOwner::Plugin);
    }

    #[test]
    fn pick_provider_with_preferred() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        let svc1 = test_uuid();
        let svc2 = test_uuid();

        registry
            .register_service(svc1, "app", vec![test_manifest("ext.pick")], None)
            .expect("register should succeed");
        registry
            .register_service(svc2, "app", vec![test_manifest("ext.pick")], None)
            .expect("register should succeed");

        // Preferred is in the set.
        assert_eq!(registry.pick_provider("ext.pick", Some(svc2)), Some(svc2));
    }

    #[test]
    fn pick_provider_preferred_not_in_set() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        let svc = test_uuid();
        let unknown = test_uuid();

        registry
            .register_service(svc, "app", vec![test_manifest("ext.pick")], None)
            .expect("register should succeed");

        // Preferred is not in the set — falls back to first.
        assert_eq!(registry.pick_provider("ext.pick", Some(unknown)), Some(svc));
    }

    #[test]
    fn pick_provider_no_preferred() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        let svc = test_uuid();

        registry
            .register_service(svc, "app", vec![test_manifest("ext.pick")], None)
            .expect("register should succeed");

        assert_eq!(registry.pick_provider("ext.pick", None), Some(svc));
    }

    #[test]
    fn pick_provider_nonexistent_extension() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        assert_eq!(registry.pick_provider("nonexistent", None), None);
    }

    #[test]
    fn error_display_is_descriptive() {
        let err = ExtensionRegistryError::ConflictingAppName {
            extension_id: "ext.x".to_string(),
            existing_app_name: "app-a".to_string(),
            incoming_app_name: "app-b".to_string(),
        };

        let msg = err.to_string();
        assert!(msg.contains("ext.x"));
        assert!(msg.contains("app-a"));
        assert!(msg.contains("app-b"));
    }

    #[test]
    fn register_service_conflict_does_not_mutate_state() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        let svc1 = test_uuid();
        let svc2 = test_uuid();

        // svc1 registers ext.a.
        registry
            .register_service(svc1, "app-one", vec![test_manifest("ext.a")], None)
            .expect("register should succeed");

        // svc2 tries to register ext.a (conflict) and ext.b in the same call.
        // The conflict should prevent ext.b from being registered too.
        let result = registry.register_service(
            svc2,
            "app-two",
            vec![test_manifest("ext.b"), test_manifest("ext.a")],
            None,
        );
        assert!(result.is_err());

        // ext.b should NOT have been registered (atomic validation).
        assert_eq!(registry.find_owner("ext.b"), ExtensionOwner::NotFound);
        // ext.a should still only have svc1.
        assert_eq!(registry.providers("ext.a"), vec![svc1]);
    }

    #[test]
    fn register_and_resolve_actions() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        let svc = test_uuid();

        registry
            .register_service(svc, "ssh-agent", vec![test_manifest("ssh.hosts")], None)
            .expect("register should succeed");

        let action = ActionDef::new("list-hosts", "List Hosts");
        registry.register_actions(svc, "ssh-agent", vec![action]);

        let resolved = registry.all_manifests_with_actions();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].actions.len(), 1);
        assert_eq!(resolved[0].actions[0].action_id, "list-hosts");
    }

    #[test]
    fn unregister_cleans_action_catalog_when_last_provider() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        let svc = test_uuid();

        registry
            .register_service(svc, "ssh-agent", vec![test_manifest("ssh.hosts")], None)
            .expect("register should succeed");
        registry.register_actions(svc, "ssh-agent", vec![ActionDef::new("list", "List")]);

        // Before unregister: actions should resolve.
        assert_eq!(registry.actions_for_extension("ssh.hosts").len(), 1);

        // After unregister: actions should be gone.
        registry.unregister_service(&svc);
        assert!(registry.actions_for_extension("ssh.hosts").is_empty());
    }

    #[test]
    fn plugin_actions_resolved_for_plugin_extensions() {
        let manifest = test_manifest("proxmox.hosts");
        let actions = vec![
            ActionDef::new("list", "List"),
            ActionDef::new("discover", "Discover"),
        ];
        let registry = ExtensionRegistry::new(vec![manifest], actions, vec![], vec![]);

        let resolved = registry.all_manifests_with_actions();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].actions.len(), 2);
    }

    #[test]
    fn manifests_sorted_by_priority_then_label() {
        let registry = ExtensionRegistry::new(vec![], vec![], vec![], vec![]);
        let svc = test_uuid();

        let mut m1 = test_manifest("ext.b");
        m1.priority = 200;
        m1.label = "Beta".to_string();

        let mut m2 = test_manifest("ext.a");
        m2.priority = 200;
        m2.label = "Alpha".to_string();

        let mut m3 = test_manifest("ext.c");
        m3.priority = 100;
        m3.label = "Charlie".to_string();

        registry
            .register_service(svc, "app", vec![m1, m2, m3], None)
            .expect("register should succeed");

        let manifests = registry.all_manifests();
        let labels: Vec<&str> = manifests.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, vec!["Charlie", "Alpha", "Beta"]);
    }

    #[test]
    fn find_owner_notification() {
        let notif_manifest = test_manifest("notifications.webhook");
        let registry = ExtensionRegistry::new(vec![], vec![], vec![notif_manifest], vec![]);
        assert_eq!(
            registry.find_owner("notifications.webhook"),
            ExtensionOwner::Notification
        );
    }

    #[test]
    fn notification_extensions_included_in_all_manifests() {
        let notif_manifest = test_manifest("notifications.webhook");
        let notif_action = ActionDef::new("list", "List");
        let registry =
            ExtensionRegistry::new(vec![], vec![], vec![notif_manifest], vec![notif_action]);

        let manifests = registry.all_manifests();
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].id, "notifications.webhook");

        let resolved = registry.all_manifests_with_actions();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].actions.len(), 1);
        assert_eq!(resolved[0].actions[0].action_id, "list");
    }

    #[test]
    fn notification_actions_for_extension() {
        let notif_manifest = test_manifest("notifications.webhook");
        let notif_action = ActionDef::new("list", "List");
        let registry =
            ExtensionRegistry::new(vec![], vec![], vec![notif_manifest], vec![notif_action]);

        let actions = registry.actions_for_extension("notifications.webhook");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_id, "list");
    }
}
