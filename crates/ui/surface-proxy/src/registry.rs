#![expect(
    clippy::expect_used,
    reason = "expect used for infallible operations; message documents the invariant"
)]

use std::collections::{BTreeSet, HashMap, HashSet};

use parking_lot::Mutex;
use uuid::Uuid;

use uptrakit_shared_types::access::Action;
use uptrakit_wire::surfaces;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub enum SurfaceProviderRejectionCode {
    UnsupportedGeneration,
    MissingCapability,
    InvalidSlot,
    InvalidTransport,
    SchemaOrLimitFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub struct SurfaceProviderRejectionReason {
    pub code: SurfaceProviderRejectionCode,
    pub message: String,
    pub surface_id: Option<String>,
}

impl SurfaceProviderRejectionReason {
    /// Constructs a new [`SurfaceProviderRejectionReason`].
    ///
    /// External crates must use this constructor rather than a struct literal
    /// because the type is `#[non_exhaustive]`.
    pub fn new(
        code: SurfaceProviderRejectionCode,
        message: String,
        surface_id: Option<String>,
    ) -> Self {
        Self {
            code,
            message,
            surface_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub struct SurfaceProviderRejection {
    pub provider_id: String,
    pub reasons: Vec<SurfaceProviderRejectionReason>,
}

impl SurfaceProviderRejection {
    /// Constructs a new [`SurfaceProviderRejection`].
    ///
    /// External crates must use this constructor rather than a struct literal
    /// because the type is `#[non_exhaustive]`.
    pub fn new(provider_id: String, reasons: Vec<SurfaceProviderRejectionReason>) -> Self {
        Self {
            provider_id,
            reasons,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SurfaceRegistryError {
    ProviderRejected(SurfaceProviderRejection),
    ProviderConflict(String),
}

impl std::fmt::Display for SurfaceRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProviderRejected(rejection) => write!(
                f,
                "surface registration rejected for provider {} ({} reason(s))",
                rejection.provider_id,
                rejection.reasons.len()
            ),
            Self::ProviderConflict(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for SurfaceRegistryError {}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SurfaceRegistryConfig {
    pub supported_generation: surfaces::FrameworkGenerationRange,
    pub required_capabilities: surfaces::CapabilitySet,
    pub allowed_controller_queries: HashSet<String>,
    pub allowed_sse_topics: HashSet<String>,
    pub max_data_source_page_size: u16,
    pub max_surfaces_per_batch: usize,
    pub max_interactions_per_batch: usize,
    pub max_contract_depth: usize,
    pub max_registration_payload_bytes: usize,
}

impl Default for SurfaceRegistryConfig {
    fn default() -> Self {
        Self {
            supported_generation: surfaces::FrameworkGenerationRange {
                min: surfaces::FrameworkGeneration::new(1, 0),
                max: surfaces::FrameworkGeneration::new(1, 0),
            },
            required_capabilities: surfaces::CapabilitySet::default(),
            allowed_controller_queries: HashSet::new(),
            allowed_sse_topics: HashSet::new(),
            max_data_source_page_size: 1000,
            max_surfaces_per_batch: 64,
            max_interactions_per_batch: 256,
            max_contract_depth: 16,
            max_registration_payload_bytes: 512 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SurfaceCatalogItem {
    pub surface_id: String,
    pub slot: String,
    pub provider_id: String,
    pub targeting: surfaces::Targeting,
    pub descriptor: surfaces::SurfaceDescriptor,
}

impl SurfaceCatalogItem {
    /// Constructs a new [`SurfaceCatalogItem`].
    ///
    /// External crates must use this constructor rather than a struct literal
    /// because the type is `#[non_exhaustive]`.
    pub fn new(
        surface_id: String,
        slot: String,
        provider_id: String,
        targeting: surfaces::Targeting,
        descriptor: surfaces::SurfaceDescriptor,
    ) -> Self {
        Self {
            surface_id,
            slot,
            provider_id,
            targeting,
            descriptor,
        }
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SurfaceProviderSummary {
    pub provider_id: String,
    pub provider_kind: surfaces::ProviderKind,
    pub tenant_compatible: bool,
    pub targeting: surfaces::Targeting,
    pub service_id: Option<Uuid>,
    pub service_app_name: Option<String>,
    pub encryption_metadata: Option<surfaces::ProviderEncryptionMetadata>,
}

/// Typed `required_action`s parsed once at admission; index-aligned with
/// `registration.surfaces` (outer) and each surface's `interactions` (inner).
#[derive(Debug, Clone)]
struct SurfaceActionSet {
    descriptor: Option<Action>,
    interactions: Vec<Option<Action>>,
}

/// Parses every surface's `required_action` string into a typed [`Action`],
/// index-aligned with `registration.surfaces`/`interactions`. Defensive
/// fail-closed: a parse failure here is unreachable after admission's Step 2
/// validation, but this never silently downgrades to `None` — it maps to the
/// same `SchemaOrLimitFailure` rejection admission would have produced.
fn parse_surface_actions(
    registration: &surfaces::SurfaceRegistration,
) -> Result<Vec<SurfaceActionSet>, SurfaceRegistryError> {
    let provider_id = registration.provider.provider_id.clone();
    let mut reasons = Vec::new();
    let mut surface_actions = Vec::with_capacity(registration.surfaces.len());

    for surface in &registration.surfaces {
        let surface_id = Some(surface.descriptor.surface_id.to_string());
        let descriptor = match &surface.descriptor.required_action {
            Some(value) => match value.parse::<Action>() {
                Ok(action) => Some(action),
                Err(_) => {
                    reasons.push(SurfaceProviderRejectionReason {
                        code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                        message: format!("invalid descriptor required_action `{value}`"),
                        surface_id: surface_id.clone(),
                    });
                    None
                }
            },
            None => None,
        };

        let mut interactions = Vec::with_capacity(surface.interactions.len());
        for interaction in &surface.interactions {
            let parsed = match &interaction.required_action {
                Some(value) => match value.parse::<Action>() {
                    Ok(action) => Some(action),
                    Err(_) => {
                        reasons.push(SurfaceProviderRejectionReason {
                            code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                            message: format!("invalid interaction required_action `{value}`"),
                            surface_id: surface_id.clone(),
                        });
                        None
                    }
                },
                None => None,
            };
            interactions.push(parsed);
        }

        surface_actions.push(SurfaceActionSet {
            descriptor,
            interactions,
        });
    }

    if reasons.is_empty() {
        return Ok(surface_actions);
    }

    Err(SurfaceRegistryError::ProviderRejected(
        SurfaceProviderRejection {
            provider_id,
            reasons,
        },
    ))
}

#[derive(Debug, Clone)]
struct ProviderRegistration {
    registration: surfaces::SurfaceRegistration,
    service_id: Option<Uuid>,
    service_app_name: Option<String>,
    surface_actions: Vec<SurfaceActionSet>,
}

#[derive(Default)]
struct SurfaceRegistryInner {
    providers: HashMap<String, ProviderRegistration>,
    service_to_provider: HashMap<Uuid, String>,
    surface_to_providers: HashMap<String, BTreeSet<String>>,
}

/// Decides whether a plugin-backed surface provider is currently servable.
/// Service- and BuiltIn-kind providers are not consulted (spec 2026-07-27 D2);
/// the filter is a required parameter on every tenant-facing enumeration and
/// resolution method so no leg can resolve without deciding visibility.
pub trait SurfaceProviderVisibility: Send + Sync {
    /// Returns `true` when the Plugin-kind provider with this wire
    /// `provider_id` is effectively enabled.
    fn plugin_provider_visible(&self, provider_id: &str) -> bool;
}

/// Fail-closed default: hides every Plugin-kind provider. Production wiring
/// replaces it with the controller's effective-enablement filter; anything
/// constructed without that wiring must hide plugin surfaces, never serve
/// them ungated.
pub struct DenyAllPluginProviders;

impl SurfaceProviderVisibility for DenyAllPluginProviders {
    fn plugin_provider_visible(&self, _provider_id: &str) -> bool {
        false
    }
}

/// Permissive filter for tests that exercise registry/proxy mechanics
/// without plugin-enablement concerns.
#[cfg(any(test, feature = "testing"))]
pub struct AllProvidersVisible;

#[cfg(any(test, feature = "testing"))]
impl SurfaceProviderVisibility for AllProvidersVisible {
    fn plugin_provider_visible(&self, _provider_id: &str) -> bool {
        true
    }
}

pub struct SurfaceRegistry {
    config: SurfaceRegistryConfig,
    inner: Mutex<SurfaceRegistryInner>,
}

impl SurfaceRegistry {
    pub fn new(config: SurfaceRegistryConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(SurfaceRegistryInner::default()),
        }
    }

    pub fn register_service(
        &self,
        service_id: Uuid,
        service_app_name: &str,
        service_tenant_id: Option<Uuid>,
        mut registration: surfaces::SurfaceRegistration,
    ) -> Result<(), SurfaceRegistryError> {
        self.validate_registration_basics(
            surfaces::ProviderKind::Service,
            &registration,
            service_tenant_id,
        )?;

        let provider_id = registration.provider.provider_id.clone();
        let mut inner = self.inner.lock();
        let rotation_backup = inner
            .service_to_provider
            .get(&service_id)
            .cloned()
            .filter(|existing_provider_id| existing_provider_id != &provider_id)
            .and_then(|existing_provider_id| {
                let entry = inner.providers.get(&existing_provider_id).cloned()?;
                remove_provider(&mut inner, &existing_provider_id);
                Some((existing_provider_id, entry))
            });

        // Normalize BEFORE admission so the collision check compares effective
        // (normalized) contracts on both sides. Stored providers are always
        // normalized; comparing a raw incoming registration against them makes
        // byte-identical providers spuriously mismatch (e.g. a DataLoad's method
        // is rewritten POST -> GET only in the stored copy).
        normalize_interaction_methods(&mut registration);
        warn_dataloads_without_params(&registration);

        if let Err(error) = self.validate_registration_admission_locked(
            &inner,
            &registration,
            Some(service_id),
            Some(service_app_name),
        ) {
            if let Some((existing_provider_id, entry)) = rotation_backup.clone() {
                upsert_provider(&mut inner, existing_provider_id.clone(), entry);
                inner
                    .service_to_provider
                    .insert(service_id, existing_provider_id);
            }
            return Err(error);
        }

        // Computed after normalization (above) so indices align with the
        // stored (normalized) registration.
        let surface_actions = match parse_surface_actions(&registration) {
            Ok(surface_actions) => surface_actions,
            Err(error) => {
                if let Some((existing_provider_id, entry)) = rotation_backup.clone() {
                    upsert_provider(&mut inner, existing_provider_id.clone(), entry);
                    inner
                        .service_to_provider
                        .insert(service_id, existing_provider_id);
                }
                return Err(error);
            }
        };

        if let Some(existing) = inner.providers.get(&provider_id)
            && existing.service_id != Some(service_id)
        {
            return Err(SurfaceRegistryError::ProviderConflict(format!(
                "provider `{provider_id}` is already registered by a different service"
            )));
        }

        inner
            .service_to_provider
            .insert(service_id, provider_id.clone());
        upsert_provider(
            &mut inner,
            provider_id,
            ProviderRegistration {
                registration,
                service_id: Some(service_id),
                service_app_name: Some(service_app_name.to_string()),
                surface_actions,
            },
        );
        Ok(())
    }

    pub fn bootstrap_builtin(
        &self,
        mut registration: surfaces::SurfaceRegistration,
    ) -> Result<(), SurfaceRegistryError> {
        self.validate_registration_basics(surfaces::ProviderKind::BuiltIn, &registration, None)?;
        let provider_id = registration.provider.provider_id.clone();
        let mut inner = self.inner.lock();
        // Normalize before admission so collisions compare effective contracts.
        normalize_interaction_methods(&mut registration);
        warn_dataloads_without_params(&registration);
        self.validate_registration_admission_locked(&inner, &registration, None, None)?;
        // Computed after normalization (above) so indices align with the
        // stored (normalized) registration.
        let surface_actions = parse_surface_actions(&registration)?;

        if let Some(existing) = inner.providers.get(&provider_id)
            && existing.registration.provider.provider_kind != surfaces::ProviderKind::BuiltIn
        {
            return Err(SurfaceRegistryError::ProviderConflict(format!(
                "provider `{provider_id}` is already registered as non-built-in"
            )));
        }

        upsert_provider(
            &mut inner,
            provider_id,
            ProviderRegistration {
                registration,
                service_id: None,
                service_app_name: None,
                surface_actions,
            },
        );
        Ok(())
    }

    pub fn bootstrap_plugin(
        &self,
        mut registration: surfaces::SurfaceRegistration,
    ) -> Result<(), SurfaceRegistryError> {
        self.validate_registration_basics(surfaces::ProviderKind::Plugin, &registration, None)?;
        let provider_id = registration.provider.provider_id.clone();
        let mut inner = self.inner.lock();
        // Normalize before admission so collisions compare effective contracts.
        normalize_interaction_methods(&mut registration);
        warn_dataloads_without_params(&registration);
        self.validate_registration_admission_locked(&inner, &registration, None, None)?;
        // Computed after normalization (above) so indices align with the
        // stored (normalized) registration.
        let surface_actions = parse_surface_actions(&registration)?;

        if let Some(existing) = inner.providers.get(&provider_id)
            && existing.registration.provider.provider_kind != surfaces::ProviderKind::Plugin
        {
            return Err(SurfaceRegistryError::ProviderConflict(format!(
                "provider `{provider_id}` is already registered as non-plugin"
            )));
        }

        upsert_provider(
            &mut inner,
            provider_id,
            ProviderRegistration {
                registration,
                service_id: None,
                service_app_name: None,
                surface_actions,
            },
        );
        Ok(())
    }

    pub fn unregister_service(&self, service_id: &Uuid) {
        let mut inner = self.inner.lock();
        let provider_id = inner.service_to_provider.remove(service_id);
        if let Some(provider_id) = provider_id {
            remove_provider(&mut inner, &provider_id);
        }
    }

    pub fn list_surfaces_for_tenant(
        &self,
        tenant_id: Uuid,
        slot_filter: Option<&str>,
        page_filter: Option<&str>,
        visibility: &dyn SurfaceProviderVisibility,
    ) -> Vec<SurfaceCatalogItem> {
        let inner = self.inner.lock();
        let mut items = Vec::new();

        for (provider_id, provider) in &inner.providers {
            if provider.registration.provider.provider_kind == surfaces::ProviderKind::Plugin
                && !visibility.plugin_provider_visible(provider_id)
            {
                continue;
            }

            for registered in &provider.registration.surfaces {
                if !surface_visible_for_tenant(
                    &provider.registration.effective_tenant_binding,
                    &registered.descriptor,
                    tenant_id,
                ) {
                    continue;
                }

                if let Some(slot) = slot_filter
                    && registered.descriptor.slot != slot
                {
                    continue;
                }

                if let Some(page) = page_filter
                    && !surface_slot_matches_page(registered.descriptor.slot.as_str(), page)
                {
                    continue;
                }

                items.push(SurfaceCatalogItem {
                    surface_id: registered.descriptor.surface_id.to_string(),
                    slot: registered.descriptor.slot.clone(),
                    provider_id: provider_id.clone(),
                    targeting: registered.descriptor.targeting,
                    descriptor: registered.descriptor.clone(),
                });
            }
        }

        items.sort_by(|a, b| {
            a.slot
                .cmp(&b.slot)
                .then_with(|| a.surface_id.cmp(&b.surface_id))
                .then_with(|| a.provider_id.cmp(&b.provider_id))
        });
        items
    }

    pub fn list_targeted_providers_for_surface(
        &self,
        surface_id: &str,
        tenant_id: Uuid,
        visibility: &dyn SurfaceProviderVisibility,
    ) -> Vec<SurfaceProviderSummary> {
        let inner = self.inner.lock();
        let mut providers = Vec::new();
        let provider_ids = inner
            .surface_to_providers
            .get(surface_id)
            .cloned()
            .unwrap_or_default();

        for provider_id in provider_ids {
            let Some(provider) = inner.providers.get(&provider_id) else {
                continue;
            };
            if provider.registration.provider.provider_kind == surfaces::ProviderKind::Plugin
                && !visibility.plugin_provider_visible(&provider_id)
            {
                continue;
            }
            let Some(surface) = provider
                .registration
                .surfaces
                .iter()
                .find(|surface| surface.descriptor.surface_id.as_str() == surface_id)
            else {
                continue;
            };

            providers.push(SurfaceProviderSummary {
                provider_id,
                provider_kind: provider.registration.provider.provider_kind,
                tenant_compatible: surface_visible_for_tenant(
                    &provider.registration.effective_tenant_binding,
                    &surface.descriptor,
                    tenant_id,
                ),
                targeting: surface.descriptor.targeting,
                service_id: provider.service_id,
                service_app_name: provider.service_app_name.clone(),
                encryption_metadata: provider.registration.encryption_metadata.clone(),
            });
        }

        let mut tenant_visible: Vec<SurfaceProviderSummary> = providers
            .into_iter()
            .filter(|provider| provider.tenant_compatible)
            .collect();
        tenant_visible.sort_by(|a, b| a.provider_id.cmp(&b.provider_id));
        tenant_visible
    }

    /// Method-aware surface action resolution (REST method model, B1-B8).
    ///
    /// - `method: Some(m)` resolves the exact `(interaction_id, m)` pair;
    ///   if the interaction id exists but no registration matches `m`,
    ///   returns [`SurfaceRegistryLookupError::MethodNotAllowed`] listing
    ///   every method actually registered for that id.
    /// - `method: None` resolves iff the interaction id registers exactly
    ///   one method (it does **not** prefer POST); zero registrations is
    ///   [`SurfaceRegistryLookupError::InteractionNotFound`], more than one
    ///   is `MethodNotAllowed` listing all of them.
    pub fn resolve_surface_action_for_method(
        &self,
        tenant_id: Uuid,
        surface_id: &str,
        interaction_id: &str,
        method: Option<&surfaces::InteractionHttpMethod>,
        target_provider_id: Option<&str>,
        visibility: &dyn SurfaceProviderVisibility,
    ) -> Result<ResolvedSurfaceAction, SurfaceRegistryLookupError> {
        let providers = self.list_targeted_providers_for_surface(surface_id, tenant_id, visibility);
        if providers.is_empty() {
            return Err(SurfaceRegistryLookupError::SurfaceNotFound);
        }

        let selected_provider = if let Some(target_provider_id) = target_provider_id {
            providers
                .iter()
                .find(|provider| provider.provider_id == target_provider_id)
                .cloned()
                .ok_or_else(|| {
                    SurfaceRegistryLookupError::InvalidProvider(target_provider_id.to_string())
                })?
        } else {
            let candidate_providers = preferred_provider_candidates(providers)?;
            if candidate_providers
                .iter()
                .any(|provider| provider.targeting == surfaces::Targeting::Targeted)
            {
                return Err(SurfaceRegistryLookupError::TargetProviderRequired);
            }
            candidate_providers
                .into_iter()
                .next()
                .expect("preferred candidate resolution should yield at least one provider")
        };

        if !selected_provider.tenant_compatible {
            return Err(SurfaceRegistryLookupError::NoTenantCompatibleProvider);
        }

        let inner = self.inner.lock();
        let provider = inner
            .providers
            .get(&selected_provider.provider_id)
            .ok_or(SurfaceRegistryLookupError::SurfaceNotFound)?;
        let (surface_idx, surface) = provider
            .registration
            .surfaces
            .iter()
            .enumerate()
            .find(|(_, surface)| surface.descriptor.surface_id.as_str() == surface_id)
            .ok_or(SurfaceRegistryLookupError::SurfaceNotFound)?;

        let matching_interactions: Vec<(usize, &surfaces::InteractionDescriptor)> = surface
            .interactions
            .iter()
            .enumerate()
            .filter(|(_, interaction)| interaction.interaction_id.as_str() == interaction_id)
            .collect();

        let action_set = provider.surface_actions.get(surface_idx);

        let (interaction_idx, interaction) = match method {
            Some(requested_method) => matching_interactions
                .iter()
                .find(|(_, interaction)| &interaction.http_method == requested_method)
                .map(|(idx, interaction)| (*idx, (*interaction).clone()))
                .ok_or_else(|| {
                    if matching_interactions.is_empty() {
                        SurfaceRegistryLookupError::InteractionNotFound
                    } else {
                        method_not_allowed_error(action_set, &matching_interactions)
                    }
                })?,
            None => match matching_interactions.as_slice() {
                [] => return Err(SurfaceRegistryLookupError::InteractionNotFound),
                [(idx, single)] => (*idx, (*single).clone()),
                _ => {
                    return Err(method_not_allowed_error(action_set, &matching_interactions));
                }
            },
        };

        let descriptor_required_action = action_set.and_then(|s| s.descriptor.clone());
        let interaction_required_action =
            action_set.and_then(|s| s.interactions.get(interaction_idx).cloned().flatten());

        Ok(ResolvedSurfaceAction {
            provider_id: selected_provider.provider_id.clone(),
            service_id: selected_provider.service_id,
            descriptor: surface.descriptor.clone(),
            interaction,
            encryption_metadata: selected_provider.encryption_metadata.clone(),
            provider_kind: selected_provider.provider_kind,
            service_app_name: provider.service_app_name.clone(),
            descriptor_required_action,
            interaction_required_action,
        })
    }

    pub fn resolve_surface_read(
        &self,
        tenant_id: Uuid,
        surface_id: &str,
        visibility: &dyn SurfaceProviderVisibility,
    ) -> Result<ResolvedSurfaceRead, SurfaceRegistryLookupError> {
        let providers = self.list_targeted_providers_for_surface(surface_id, tenant_id, visibility);
        if providers.is_empty() {
            return Err(SurfaceRegistryLookupError::SurfaceNotFound);
        }
        let candidates = preferred_provider_candidates(providers)?;
        let selected_provider_id = candidates
            .first()
            .expect("preferred candidate resolution should yield at least one provider")
            .provider_id
            .clone();

        let inner = self.inner.lock();
        let provider = inner
            .providers
            .get(&selected_provider_id)
            .ok_or(SurfaceRegistryLookupError::SurfaceNotFound)?;
        let (surface_idx, surface) = provider
            .registration
            .surfaces
            .iter()
            .enumerate()
            .find(|(_, surface)| surface.descriptor.surface_id.as_str() == surface_id)
            .ok_or(SurfaceRegistryLookupError::SurfaceNotFound)?;

        Ok(ResolvedSurfaceRead {
            descriptor: surface.descriptor.clone(),
            interactions: surface.interactions.clone(),
            data_sources: surface.data_sources.clone(),
            required_action: provider
                .surface_actions
                .get(surface_idx)
                .and_then(|s| s.descriptor.clone()),
        })
    }

    pub fn provider_id_for_service(&self, service_id: &Uuid) -> Option<String> {
        self.inner
            .lock()
            .service_to_provider
            .get(service_id)
            .cloned()
    }

    fn validate_registration_basics(
        &self,
        source_kind: surfaces::ProviderKind,
        registration: &surfaces::SurfaceRegistration,
        service_tenant_id: Option<Uuid>,
    ) -> Result<(), SurfaceRegistryError> {
        let provider_id = registration.provider.provider_id.clone();
        let mut reasons = Vec::new();

        if registration.provider.provider_kind != source_kind {
            reasons.push(SurfaceProviderRejectionReason {
                code: SurfaceProviderRejectionCode::InvalidTransport,
                message: format!(
                    "provider_kind {:?} is not allowed for this registration source",
                    registration.provider.provider_kind
                ),
                surface_id: None,
            });
        }

        // Provider-id namespaces are enforced per registration source
        // (ADR-0034, fail-closed): a service can never occupy a plugin's
        // identity in the provider-keyed registry map, and `builtin.` is
        // reserved before any production BuiltIn provider exists.
        let namespace_violation = match source_kind {
            surfaces::ProviderKind::Service if !provider_id.starts_with("service.") => {
                Some("service surface registrations must use a `service.`-prefixed provider id")
            }
            surfaces::ProviderKind::BuiltIn if !provider_id.starts_with("builtin.") => {
                Some("built-in surface registrations must use a `builtin.`-prefixed provider id")
            }
            surfaces::ProviderKind::Plugin
                if provider_id.starts_with("service.") || provider_id.starts_with("builtin.") =>
            {
                Some("plugin surface registrations must not use a reserved provider-id namespace")
            }
            surfaces::ProviderKind::Service
            | surfaces::ProviderKind::BuiltIn
            | surfaces::ProviderKind::Plugin => None,
            _ => Some("unknown registration source kind is not admitted"),
        };
        if let Some(message) = namespace_violation {
            reasons.push(SurfaceProviderRejectionReason {
                code: SurfaceProviderRejectionCode::InvalidTransport,
                message: message.to_string(),
                surface_id: None,
            });
        }

        if source_kind == surfaces::ProviderKind::Service {
            match service_tenant_id {
                Some(expected_tenant_id) => {
                    if registration.effective_tenant_binding.scope != surfaces::Scope::Tenant {
                        reasons.push(SurfaceProviderRejectionReason {
                            code: SurfaceProviderRejectionCode::InvalidTransport,
                            message: "service surface registrations must be tenant-scoped"
                                .to_string(),
                            surface_id: None,
                        });
                    }
                    let claimed_tenant_id = registration
                        .effective_tenant_binding
                        .tenant_id
                        .as_deref()
                        .and_then(parse_uuid_like);
                    if claimed_tenant_id != Some(expected_tenant_id) {
                        reasons.push(SurfaceProviderRejectionReason {
                            code: SurfaceProviderRejectionCode::InvalidTransport,
                            message: "service surface registration tenant binding does not match authenticated service tenant".to_string(),
                            surface_id: None,
                        });
                    }
                }
                None => {
                    if registration.effective_tenant_binding.scope != surfaces::Scope::Global
                        || registration.effective_tenant_binding.tenant_id.is_some()
                    {
                        reasons.push(SurfaceProviderRejectionReason {
                            code: SurfaceProviderRejectionCode::InvalidTransport,
                            message: "system service surface registrations must be global-scoped with no tenant binding".to_string(),
                            surface_id: None,
                        });
                    }
                }
            }
        }

        if let Ok(payload) = serde_json::to_vec(registration)
            && payload.len() > self.config.max_registration_payload_bytes
        {
            reasons.push(SurfaceProviderRejectionReason {
                code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                message: format!(
                    "registration payload is {} bytes, max {} bytes",
                    payload.len(),
                    self.config.max_registration_payload_bytes
                ),
                surface_id: None,
            });
        }

        if registration.surfaces.len() > self.config.max_surfaces_per_batch {
            reasons.push(SurfaceProviderRejectionReason {
                code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                message: format!(
                    "registration contains {} surfaces, max {}",
                    registration.surfaces.len(),
                    self.config.max_surfaces_per_batch
                ),
                surface_id: None,
            });
        }

        let interaction_total: usize = registration
            .surfaces
            .iter()
            .map(|surface| surface.interactions.len())
            .sum();
        if interaction_total > self.config.max_interactions_per_batch {
            reasons.push(SurfaceProviderRejectionReason {
                code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                message: format!(
                    "registration contains {} interactions, max {}",
                    interaction_total, self.config.max_interactions_per_batch
                ),
                surface_id: None,
            });
        }

        for surface in &registration.surfaces {
            let surface_id = Some(surface.descriptor.surface_id.to_string());
            if surface_node_depth(&surface.descriptor.root_node) > self.config.max_contract_depth {
                reasons.push(SurfaceProviderRejectionReason {
                    code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                    message: format!(
                        "surface root depth exceeds max {}",
                        self.config.max_contract_depth
                    ),
                    surface_id: surface_id.clone(),
                });
            }

            if matches!(surface.descriptor.targeting, surfaces::Targeting::Targeted)
                && !matches!(surface.descriptor.scope, surfaces::Scope::Tenant)
            {
                reasons.push(SurfaceProviderRejectionReason {
                    code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                    message: "targeted surfaces must be tenant-scoped".to_string(),
                    surface_id: surface_id.clone(),
                });
            }

            if let Some(value) = &surface.descriptor.required_action
                && value.parse::<Action>().is_err()
            {
                tracing::warn!(
                    provider_id = %provider_id,
                    value = %value,
                    "surface registration rejected: invalid descriptor required_action"
                );
                reasons.push(SurfaceProviderRejectionReason {
                    code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                    message: format!("invalid descriptor required_action `{value}`"),
                    surface_id: surface_id.clone(),
                });
            }

            for interaction in &surface.interactions {
                if let Some(value) = &interaction.required_action
                    && value.parse::<Action>().is_err()
                {
                    tracing::warn!(
                        provider_id = %provider_id,
                        value = %value,
                        "surface registration rejected: invalid interaction required_action"
                    );
                    reasons.push(SurfaceProviderRejectionReason {
                        code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                        message: format!("invalid interaction required_action `{value}`"),
                        surface_id: surface_id.clone(),
                    });
                }

                if matches!(
                    interaction.transport,
                    surfaces::InteractionTransport::ControllerLocal
                ) && source_kind != surfaces::ProviderKind::Plugin
                {
                    reasons.push(SurfaceProviderRejectionReason {
                        code: SurfaceProviderRejectionCode::InvalidTransport,
                        message:
                            "controller_local is currently supported only for plugin providers"
                                .to_string(),
                        surface_id: surface_id.clone(),
                    });
                }

                if !interaction.sensitive_fields.is_empty() {
                    match &interaction.transport {
                        surfaces::InteractionTransport::ControllerLocal => {}
                        surfaces::InteractionTransport::ProviderProxied => {
                            if registration.encryption_metadata.is_none()
                                && source_kind != surfaces::ProviderKind::BuiltIn
                            {
                                reasons.push(SurfaceProviderRejectionReason {
                                    code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                                    message:
                                        "sensitive fields require provider encryption metadata"
                                            .to_string(),
                                    surface_id: surface_id.clone(),
                                });
                            }
                        }
                        &_ => {
                            tracing::warn!("unknown interaction transport — update match arm");
                        }
                    }
                }
            }

            for data_source in &surface.data_sources {
                if let Some(pagination) = &data_source.pagination {
                    if pagination.default_page_size > self.config.max_data_source_page_size {
                        reasons.push(SurfaceProviderRejectionReason {
                            code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                            message: format!(
                                "default_page_size {} exceeds max {}",
                                pagination.default_page_size, self.config.max_data_source_page_size
                            ),
                            surface_id: surface_id.clone(),
                        });
                    }
                    if pagination.max_page_size > self.config.max_data_source_page_size {
                        reasons.push(SurfaceProviderRejectionReason {
                            code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                            message: format!(
                                "max_page_size {} exceeds max {}",
                                pagination.max_page_size, self.config.max_data_source_page_size
                            ),
                            surface_id: surface_id.clone(),
                        });
                    }
                }

                match &data_source.kind {
                    surfaces::DataSourceKind::ControllerQuery { query_id } => {
                        if source_kind == surfaces::ProviderKind::Service {
                            reasons.push(SurfaceProviderRejectionReason {
                                code: SurfaceProviderRejectionCode::InvalidTransport,
                                message:
                                    "service providers cannot declare controller_query data sources"
                                        .to_string(),
                                surface_id: surface_id.clone(),
                            });
                        }
                        if !self
                            .config
                            .allowed_controller_queries
                            .contains(query_id.as_str())
                        {
                            reasons.push(SurfaceProviderRejectionReason {
                                code: SurfaceProviderRejectionCode::InvalidTransport,
                                message: format!(
                                    "controller query `{}` is not allowlisted",
                                    query_id
                                ),
                                surface_id: surface_id.clone(),
                            });
                        }
                    }
                    surfaces::DataSourceKind::Static { data } => {
                        if json_depth(data) > self.config.max_contract_depth {
                            reasons.push(SurfaceProviderRejectionReason {
                                code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                                message: format!(
                                    "static data depth exceeds max {}",
                                    self.config.max_contract_depth
                                ),
                                surface_id: surface_id.clone(),
                            });
                        }
                    }
                    surfaces::DataSourceKind::ProviderQuery { .. } => {}
                }

                if let surfaces::RefreshPolicy::Sse { topic } = &data_source.refresh_policy
                    && !sse_topic_allowlisted(topic, &self.config.allowed_sse_topics)
                {
                    reasons.push(SurfaceProviderRejectionReason {
                        code: SurfaceProviderRejectionCode::InvalidTransport,
                        message: format!("SSE topic {:?} is not allowlisted", topic),
                        surface_id: surface_id.clone(),
                    });
                }
            }

            validate_section_header_actions(
                &surface.descriptor.root_node,
                &surface.interactions,
                Some(surface.descriptor.surface_id.to_string()),
                &mut reasons,
            );
        }

        if let Err(err) = registration.validate_against(&surfaces::SurfaceRegistrationPolicy {
            supported_generation: self.config.supported_generation,
            required_capabilities: self.config.required_capabilities.clone(),
        }) {
            let code = match err.code {
                surfaces::SurfaceRegistrationErrorCode::UnsupportedGeneration => {
                    SurfaceProviderRejectionCode::UnsupportedGeneration
                }
                surfaces::SurfaceRegistrationErrorCode::MissingCapability => {
                    SurfaceProviderRejectionCode::MissingCapability
                }
                surfaces::SurfaceRegistrationErrorCode::InvalidSlot => {
                    SurfaceProviderRejectionCode::InvalidSlot
                }
                surfaces::SurfaceRegistrationErrorCode::InvalidContract => {
                    SurfaceProviderRejectionCode::SchemaOrLimitFailure
                }
            };
            reasons.push(SurfaceProviderRejectionReason {
                code,
                message: err.message,
                surface_id: None,
            });
        }

        if reasons.is_empty() {
            return Ok(());
        }

        Err(SurfaceRegistryError::ProviderRejected(
            SurfaceProviderRejection {
                provider_id,
                reasons,
            },
        ))
    }

    fn validate_registration_admission_locked(
        &self,
        inner: &SurfaceRegistryInner,
        registration: &surfaces::SurfaceRegistration,
        service_id: Option<Uuid>,
        service_app_name: Option<&str>,
    ) -> Result<(), SurfaceRegistryError> {
        let provider_id = registration.provider.provider_id.as_str();

        if let Some(service_id) = service_id {
            if let Some(existing) = inner.providers.get(provider_id)
                && let Some(existing_service_id) = existing.service_id
                && existing_service_id != service_id
            {
                return Err(SurfaceRegistryError::ProviderConflict(format!(
                    "provider `{provider_id}` is already bound to service {existing_service_id}"
                )));
            }
            if let Some(existing) = inner.providers.get(provider_id)
                && existing.service_app_name.as_deref() != service_app_name
            {
                return Err(SurfaceRegistryError::ProviderConflict(format!(
                    "provider `{provider_id}` app name conflict: existing={:?}, incoming={:?}",
                    existing.service_app_name, service_app_name
                )));
            }
        }

        validate_contract_collisions(inner, registration)
    }

    #[cfg(test)]
    fn provider_surface_count(&self, provider_id: &str) -> usize {
        self.inner
            .lock()
            .providers
            .get(provider_id)
            .map(|entry| entry.registration.surfaces.len())
            .unwrap_or(0)
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn register_provider_for_test(
        &self,
        registration: surfaces::SurfaceRegistration,
        service_id: Option<Uuid>,
        service_app_name: Option<&str>,
    ) {
        let provider_id = registration.provider.provider_id.clone();
        let surface_actions = parse_surface_actions(&registration)
            .expect("test-fixture registrations must carry parseable required_action values");
        let mut inner = self.inner.lock();
        if let Some(service_id) = service_id {
            inner
                .service_to_provider
                .insert(service_id, provider_id.clone());
        }
        upsert_provider(
            &mut inner,
            provider_id,
            ProviderRegistration {
                registration,
                service_id,
                service_app_name: service_app_name.map(ToOwned::to_owned),
                surface_actions,
            },
        );
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResolvedSurfaceAction {
    pub provider_id: String,
    pub provider_kind: surfaces::ProviderKind,
    pub service_id: Option<Uuid>,
    pub service_app_name: Option<String>,
    pub descriptor: surfaces::SurfaceDescriptor,
    pub interaction: surfaces::InteractionDescriptor,
    pub encryption_metadata: Option<surfaces::ProviderEncryptionMetadata>,
    /// Typed, parsed-at-admission `required_action` for `descriptor`.
    /// Authoritative for enforcement; `descriptor.required_action` (the wire
    /// string field) remains reachable but is display/wire data only — never
    /// consult it for authorization.
    pub descriptor_required_action: Option<Action>,
    /// Typed, parsed-at-admission `required_action` for `interaction`.
    /// Authoritative for enforcement; `interaction.required_action` (the
    /// wire string field) remains reachable but is display/wire data only —
    /// never consult it for authorization.
    pub interaction_required_action: Option<Action>,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResolvedSurfaceRead {
    pub descriptor: surfaces::SurfaceDescriptor,
    pub interactions: Vec<surfaces::InteractionDescriptor>,
    pub data_sources: Vec<surfaces::DataSourceDescriptor>,
    /// Typed, parsed-at-admission `required_action` for `descriptor`.
    /// Authoritative for enforcement; `descriptor.required_action` (the wire
    /// string field) remains reachable but is display/wire data only — never
    /// consult it for authorization.
    pub required_action: Option<Action>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SurfaceRegistryLookupError {
    SurfaceNotFound,
    InteractionNotFound,
    TargetProviderRequired,
    InvalidProvider(String),
    NoTenantCompatibleProvider,
    /// The interaction id exists but none of its registered `(id, method)`
    /// registrations match the requested method. Carries every HTTP method
    /// currently registered for that interaction id (for building an `Allow`
    /// header or a descriptive 405 message), plus the typed actions needed
    /// to keep 403-before-405 ordering in the handler.
    MethodNotAllowed {
        /// Every method registered for this interaction ID, in
        /// `KNOWN_INTERACTION_HTTP_METHODS` order (stable `Allow` header
        /// order), not registration/iteration order.
        allowed: Vec<surfaces::InteractionHttpMethod>,
        /// The surface descriptor's typed `required_action`.
        descriptor_required_action: Option<Action>,
        /// Each matching interaction's typed `required_action`, aligned
        /// index-for-index with `allowed`.
        interaction_required_actions: Vec<Option<Action>>,
    },
}

/// Builds the `MethodNotAllowed` payload for a set of same-id interaction
/// candidates: `allowed` ordered by `KNOWN_INTERACTION_HTTP_METHODS` (stable
/// `Allow` header order), with `interaction_required_actions` aligned
/// index-for-index to `allowed`.
fn method_not_allowed_error(
    action_set: Option<&SurfaceActionSet>,
    candidates: &[(usize, &surfaces::InteractionDescriptor)],
) -> SurfaceRegistryLookupError {
    let mut allowed = Vec::new();
    let mut interaction_required_actions = Vec::new();
    for known_method in surfaces::KNOWN_INTERACTION_HTTP_METHODS {
        if let Some((idx, _candidate)) = candidates
            .iter()
            .find(|(_, candidate)| &candidate.http_method == known_method)
        {
            allowed.push(known_method.clone());
            interaction_required_actions
                .push(action_set.and_then(|s| s.interactions.get(*idx).cloned().flatten()));
        }
    }
    SurfaceRegistryLookupError::MethodNotAllowed {
        allowed,
        descriptor_required_action: action_set.and_then(|s| s.descriptor.clone()),
        interaction_required_actions,
    }
}

/// Renders `allowed` (a `MethodNotAllowed::allowed` list) as a
/// `"get, post"`-style string for error messages. Shared by both surface-proxy
/// `map_lookup_error` copies to avoid duplicating the join logic.
pub(crate) fn format_allowed_methods(allowed: &[surfaces::InteractionHttpMethod]) -> String {
    allowed
        .iter()
        .map(surfaces::InteractionHttpMethod::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// DataLoads are GET-only (B1): rewrite the stored method so every consumer
/// (read model, dispatch, guard tests) sees the effective value.
fn normalize_interaction_methods(registration: &mut surfaces::SurfaceRegistration) {
    for surface in &mut registration.surfaces {
        for interaction in &mut surface.interactions {
            interaction.http_method = interaction.effective_http_method();
        }
    }
}

fn warn_dataloads_without_params(registration: &surfaces::SurfaceRegistration) {
    for surface in &registration.surfaces {
        for interaction in &surface.interactions {
            if interaction.kind == surfaces::InteractionKind::DataLoad
                && interaction.params.is_empty()
            {
                tracing::warn!(
                    provider_id = %registration.provider.provider_id,
                    surface_id = %surface.descriptor.surface_id,
                    interaction_id = %interaction.interaction_id,
                    "data-load interaction registered without param declarations"
                );
                metrics::counter!(
                    "uptrakit_surface_registration_dataload_missing_params_total",
                    "surface" => surface.descriptor.surface_id.as_str().to_string(),
                    "interaction" => interaction.interaction_id.as_str().to_string()
                )
                .increment(1);
            }
        }
    }
}

fn upsert_provider(
    inner: &mut SurfaceRegistryInner,
    provider_id: String,
    entry: ProviderRegistration,
) {
    remove_provider_from_surface_index(inner, &provider_id);
    for surface in &entry.registration.surfaces {
        inner
            .surface_to_providers
            .entry(surface.descriptor.surface_id.to_string())
            .or_default()
            .insert(provider_id.clone());
    }
    inner.providers.insert(provider_id, entry);
}

fn remove_provider(inner: &mut SurfaceRegistryInner, provider_id: &str) {
    remove_provider_from_surface_index(inner, provider_id);
    if let Some(existing) = inner.providers.remove(provider_id)
        && let Some(service_id) = existing.service_id
    {
        inner.service_to_provider.remove(&service_id);
    }
}

fn remove_provider_from_surface_index(inner: &mut SurfaceRegistryInner, provider_id: &str) {
    let mut empty_keys = Vec::new();
    for (surface_id, provider_ids) in &mut inner.surface_to_providers {
        provider_ids.remove(provider_id);
        if provider_ids.is_empty() {
            empty_keys.push(surface_id.clone());
        }
    }
    for surface_id in empty_keys {
        inner.surface_to_providers.remove(&surface_id);
    }
}

fn preferred_provider_candidates(
    providers: Vec<SurfaceProviderSummary>,
) -> Result<Vec<SurfaceProviderSummary>, SurfaceRegistryLookupError> {
    let tenant_compatible: Vec<_> = providers
        .into_iter()
        .filter(|provider| provider.tenant_compatible)
        .collect();
    if tenant_compatible.is_empty() {
        return Err(SurfaceRegistryLookupError::NoTenantCompatibleProvider);
    }

    Ok(tenant_compatible)
}

fn surface_visible_for_tenant(
    binding: &surfaces::EffectiveTenantBinding,
    descriptor: &surfaces::SurfaceDescriptor,
    tenant_id: Uuid,
) -> bool {
    let binding_tenant_id = binding.tenant_id.as_deref().and_then(parse_uuid_like);
    if matches!(binding.scope, surfaces::Scope::Tenant) && binding_tenant_id != Some(tenant_id) {
        return false;
    }

    match descriptor.scope {
        surfaces::Scope::Global => true,
        surfaces::Scope::Tenant => {
            matches!(binding.scope, surfaces::Scope::Tenant) && binding_tenant_id == Some(tenant_id)
        }
        _ => {
            tracing::warn!(scope = ?descriptor.scope, "unknown Scope variant; treating surface as not visible");
            false
        }
    }
}

fn parse_uuid_like(value: &str) -> Option<Uuid> {
    Uuid::parse_str(value).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum EffectiveScopeKey {
    Global,
    Tenant(Uuid),
}

fn effective_scope_key(binding: &surfaces::EffectiveTenantBinding) -> Option<EffectiveScopeKey> {
    match binding.scope {
        surfaces::Scope::Global => Some(EffectiveScopeKey::Global),
        surfaces::Scope::Tenant => binding
            .tenant_id
            .as_deref()
            .and_then(parse_uuid_like)
            .map(EffectiveScopeKey::Tenant),
        _ => {
            tracing::warn!(scope = ?binding.scope, "unknown Scope variant; treating binding as unresolvable");
            None
        }
    }
}

fn canonical_targeted_contract(
    surface: &surfaces::RegisteredSurface,
) -> (
    &surfaces::SurfaceDescriptor,
    &Vec<surfaces::InteractionDescriptor>,
    &Vec<surfaces::DataSourceDescriptor>,
) {
    (
        &surface.descriptor,
        &surface.interactions,
        &surface.data_sources,
    )
}

fn validate_contract_collisions(
    inner: &SurfaceRegistryInner,
    incoming: &surfaces::SurfaceRegistration,
) -> Result<(), SurfaceRegistryError> {
    let Some(incoming_scope_key) = effective_scope_key(&incoming.effective_tenant_binding) else {
        return Err(SurfaceRegistryError::ProviderConflict(
            "invalid effective tenant binding for registration".to_string(),
        ));
    };
    let incoming_provider_id = incoming.provider.provider_id.as_str();

    for incoming_surface in &incoming.surfaces {
        for (existing_provider_id, existing_provider) in &inner.providers {
            if existing_provider_id == incoming_provider_id {
                continue;
            }
            let Some(existing_scope_key) =
                effective_scope_key(&existing_provider.registration.effective_tenant_binding)
            else {
                continue;
            };
            if incoming_scope_key != existing_scope_key {
                continue;
            }

            for existing_surface in &existing_provider.registration.surfaces {
                if existing_surface.descriptor.surface_id != incoming_surface.descriptor.surface_id
                {
                    continue;
                }

                if existing_provider.registration.provider.provider_kind
                    == surfaces::ProviderKind::BuiltIn
                    && incoming.provider.provider_kind != surfaces::ProviderKind::BuiltIn
                {
                    return Err(SurfaceRegistryError::ProviderConflict(format!(
                        "surface `{}` is already owned by built-in provider `{existing_provider_id}`",
                        incoming_surface.descriptor.surface_id
                    )));
                }

                if incoming_surface.descriptor.targeting == surfaces::Targeting::Universal
                    || existing_surface.descriptor.targeting == surfaces::Targeting::Universal
                {
                    return Err(SurfaceRegistryError::ProviderConflict(format!(
                        "surface `{}` with universal targeting is already registered in the same effective scope",
                        incoming_surface.descriptor.surface_id
                    )));
                }

                let incoming_canonical = canonical_targeted_contract(incoming_surface);
                let existing_canonical = canonical_targeted_contract(existing_surface);
                if incoming_canonical != existing_canonical {
                    return Err(SurfaceRegistryError::ProviderConflict(format!(
                        "targeted surface `{}` contract mismatch across providers in the same effective scope",
                        incoming_surface.descriptor.surface_id
                    )));
                }
            }
        }
    }

    Ok(())
}

fn validate_section_header_actions(
    node: &surfaces::SurfaceNode,
    interactions: &[surfaces::InteractionDescriptor],
    surface_id: Option<String>,
    reasons: &mut Vec<SurfaceProviderRejectionReason>,
) {
    match node {
        surfaces::SurfaceNode::Section {
            header_action_ids,
            children,
            ..
        } => {
            // Count check here covers built-in/plugin providers that register programmatically
            // and bypass the wire-layer validation path. The wire-layer check (Task 3) is an
            // additional early rejection for service providers; this is the authoritative gate.
            if header_action_ids.len() > 3 {
                reasons.push(SurfaceProviderRejectionReason {
                    code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                    message: format!(
                        "section header_action_ids has {} entries, max 3",
                        header_action_ids.len()
                    ),
                    surface_id: surface_id.clone(),
                });
            }
            for action_id in header_action_ids {
                // Under `(id, method)` admission (Task 4) several interactions can
                // share an id, so every match — not just the first — must satisfy
                // the header-action kind gate.
                let matching_interactions: Vec<&surfaces::InteractionDescriptor> = interactions
                    .iter()
                    .filter(|i| &i.interaction_id == action_id)
                    .collect();
                if matching_interactions.is_empty() {
                    reasons.push(SurfaceProviderRejectionReason {
                        code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                        message: format!(
                            "section header_action_ids references unknown interaction `{action_id}`"
                        ),
                        surface_id: surface_id.clone(),
                    });
                    continue;
                }
                for interaction in matching_interactions {
                    let valid_kind = matches!(
                        interaction.kind,
                        surfaces::InteractionKind::Workflow
                            | surfaces::InteractionKind::MutationAction
                    );
                    if !valid_kind {
                        reasons.push(SurfaceProviderRejectionReason {
                            code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                            message: format!(
                                "interaction `{action_id}` in header_action_ids must be \
                                 kind Workflow or MutationAction (got {:?})",
                                interaction.kind
                            ),
                            surface_id: surface_id.clone(),
                        });
                    }
                    if interaction.form_ui.is_some() {
                        reasons.push(SurfaceProviderRejectionReason {
                            code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                            message: format!(
                                "interaction `{action_id}` in header_action_ids must not \
                                 have form_ui set"
                            ),
                            surface_id: surface_id.clone(),
                        });
                    }
                }
            }
            for child in children {
                validate_section_header_actions(child, interactions, surface_id.clone(), reasons);
            }
        }
        surfaces::SurfaceNode::Tabs { tabs } => {
            for tab in tabs {
                validate_section_header_actions(
                    &tab.root,
                    interactions,
                    surface_id.clone(),
                    reasons,
                );
            }
        }
        surfaces::SurfaceNode::ModalTrigger { modal_nodes, .. } => {
            for node in modal_nodes {
                validate_section_header_actions(node, interactions, surface_id.clone(), reasons);
            }
        }
        surfaces::SurfaceNode::WorkflowTrigger { step_nodes, .. } => {
            for node in step_nodes {
                validate_section_header_actions(node, interactions, surface_id.clone(), reasons);
            }
        }
        _ => {}
    }
}

fn surface_node_depth(node: &surfaces::SurfaceNode) -> usize {
    match node {
        surfaces::SurfaceNode::Section { children, .. } => {
            1 + children.iter().map(surface_node_depth).max().unwrap_or(0)
        }
        surfaces::SurfaceNode::Tabs { tabs } => {
            1 + tabs
                .iter()
                .map(|tab| surface_node_depth(&tab.root))
                .max()
                .unwrap_or(0)
        }
        surfaces::SurfaceNode::ModalTrigger { modal_nodes, .. } => {
            1 + modal_nodes
                .iter()
                .map(surface_node_depth)
                .max()
                .unwrap_or(0)
        }
        surfaces::SurfaceNode::WorkflowTrigger { step_nodes, .. } => {
            1 + step_nodes.iter().map(surface_node_depth).max().unwrap_or(0)
        }
        _ => 1,
    }
}

fn json_depth(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Array(items) => 1 + items.iter().map(json_depth).max().unwrap_or(0),
        serde_json::Value::Object(map) => 1 + map.values().map(json_depth).max().unwrap_or(0),
        _ => 1,
    }
}

fn sse_topic_allowlisted(
    topic: &surfaces::ControllerSseTopicId,
    allowlist: &HashSet<String>,
) -> bool {
    allowlist.iter().any(|entry| {
        surfaces::ControllerSseTopicId::new(entry.clone())
            .map(|candidate| &candidate == topic)
            .unwrap_or(false)
    })
}

fn surface_slot_matches_page(slot: &str, page: &str) -> bool {
    let Some(mapped_page) = surface_page_for_slot(slot) else {
        return false;
    };
    mapped_page.eq_ignore_ascii_case(page)
}

fn surface_page_for_slot(slot: &str) -> Option<&'static str> {
    if matches!(
        slot,
        surfaces::SLOT_SETTINGS_TABS | surfaces::SLOT_SETTINGS_BELOW_GLOBAL
    ) {
        return Some("settings");
    }
    if matches!(
        slot,
        surfaces::SLOT_SOFTWARE_TABS
            | surfaces::SLOT_SOFTWARE_ITEM_TABS
            | surfaces::SLOT_SOFTWARE_ITEM_HOST_CONTEXT_MENU
    ) {
        return Some("software");
    }
    if slot == surfaces::SLOT_HOST_DETAIL_TABS {
        return Some("hosts");
    }
    if slot == surfaces::SLOT_SURFACE_PAGE {
        return Some("surfaces");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_shared_types::access::actions;

    fn tenant_a() -> Uuid {
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
    }

    fn tenant_b() -> Uuid {
        Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap()
    }

    fn registration_for_service(
        provider_id: &str,
        tenant_id: Uuid,
    ) -> surfaces::SurfaceRegistration {
        surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: provider_id.to_string(),
                provider_kind: surfaces::ProviderKind::Service,
                provider_namespace: "service".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::TargetedTargeting,
                surfaces::Capability::ProviderInitiatedActions,
                surfaces::Capability::MutationAction,
                surfaces::Capability::SensitiveFields,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Tenant,
                tenant_id: Some(tenant_id.to_string()),
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("ssh.guest.panel").unwrap())
                    .label("SSH Guest Panel")
                    .priority(100)
                    .slot("software.tabs")
                    .scope(surfaces::Scope::Tenant)
                    .targeting(surfaces::Targeting::Targeted)
                    .required_action(actions::SOFTWARE_READ)
                    .provider_kind(surfaces::ProviderKind::Service)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::TargetedTargeting,
                        surfaces::Capability::MutationAction,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    })
                    .build(),
                interactions: vec![{
                    let mut i = surfaces::InteractionDescriptor::new(
                        surfaces::InteractionId::new("refresh").unwrap(),
                        surfaces::InteractionKind::MutationAction,
                        "Refresh",
                        surfaces::InteractionTransport::ProviderProxied,
                    );
                    i.required_action = Some(actions::SOFTWARE_UPDATE.to_string());
                    i.input_schema = Some(surfaces::SchemaContract::Object);
                    i.result_schema = Some(surfaces::SchemaContract::Object);
                    i.sensitive_fields = vec!["token".to_string()];
                    i.timeout_seconds = Some(30);
                    i
                }],
                data_sources: vec![],
            }],
            encryption_metadata: Some(surfaces::ProviderEncryptionMetadata {
                key_id: "key-1".to_string(),
                algorithm: surfaces::ProviderEncryptionAlgorithm::EciesP256,
                public_key: "pub-key".to_string(),
            }),
        }
    }

    fn registry() -> SurfaceRegistry {
        SurfaceRegistry::new(SurfaceRegistryConfig::default())
    }

    fn registration_for_plugin_same_surface(provider_id: &str) -> surfaces::SurfaceRegistration {
        surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: provider_id.to_string(),
                provider_kind: surfaces::ProviderKind::Plugin,
                provider_namespace: "plugin".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::UniversalTargeting,
                surfaces::Capability::MutationAction,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Tenant,
                tenant_id: Some(tenant_a().to_string()),
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("ssh.guest.panel").unwrap())
                    .label("Plugin SSH Guest Panel")
                    .priority(100)
                    .slot("software.tabs")
                    .scope(surfaces::Scope::Tenant)
                    .targeting(surfaces::Targeting::Universal)
                    .required_action(actions::SOFTWARE_READ)
                    .provider_kind(surfaces::ProviderKind::Plugin)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::UniversalTargeting,
                        surfaces::Capability::MutationAction,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "plugin-fallback".to_string(),
                    })
                    .build(),
                interactions: vec![{
                    let mut i = surfaces::InteractionDescriptor::new(
                        surfaces::InteractionId::new("refresh").unwrap(),
                        surfaces::InteractionKind::MutationAction,
                        "Save Global SMTP",
                        surfaces::InteractionTransport::ControllerLocal,
                    );
                    i.required_action = Some(actions::SOFTWARE_UPDATE.to_string());
                    i.input_schema = Some(surfaces::SchemaContract::Object);
                    i.result_schema = Some(surfaces::SchemaContract::Object);
                    i.timeout_seconds = Some(30);
                    i
                }],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        }
    }

    fn rejection(err: SurfaceRegistryError) -> SurfaceProviderRejection {
        match err {
            SurfaceRegistryError::ProviderRejected(rejection) => rejection,
            other => panic!("expected ProviderRejected, got {other:?}"),
        }
    }

    /// Builds a minimal plugin [`SurfaceRegistration`] carrying `interactions`
    /// on a single global/universal surface, with `extra_capabilities`
    /// merged in alongside the baseline node/targeting capabilities.
    fn registration_with_interactions(
        extra_capabilities: Vec<surfaces::Capability>,
        interactions: Vec<surfaces::InteractionDescriptor>,
    ) -> surfaces::SurfaceRegistration {
        let mut capabilities = vec![
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::UniversalTargeting,
        ];
        capabilities.extend(extra_capabilities);

        surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: "dataload_test_provider".to_string(),
                provider_kind: surfaces::ProviderKind::Plugin,
                provider_namespace: "plugin".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities(capabilities.clone()),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("test.dataload.surface").unwrap())
                    .label("Test DataLoad Surface")
                    .priority(100)
                    .slot(surfaces::SLOT_SETTINGS_BELOW_GLOBAL)
                    .scope(surfaces::Scope::Global)
                    .targeting(surfaces::Targeting::Universal)
                    .provider_kind(surfaces::ProviderKind::Plugin)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities(capabilities))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    })
                    .build(),
                interactions,
                data_sources: vec![],
            }],
            encryption_metadata: None,
        }
    }

    #[test]
    fn admission_normalizes_dataload_http_method_to_get() {
        let registry = registry();
        let interaction = surfaces::InteractionDescriptor::new(
            surfaces::InteractionId::new("load-data").unwrap(),
            surfaces::InteractionKind::DataLoad,
            "Load Data",
            surfaces::InteractionTransport::ProviderProxied,
        );
        // Wire default is POST (omitted on the wire); admission must rewrite
        // DataLoad interactions to GET regardless of the declared value.
        assert_eq!(
            interaction.http_method,
            surfaces::InteractionHttpMethod::Post
        );
        let registration = registration_with_interactions(
            vec![
                surfaces::Capability::DataLoad,
                surfaces::Capability::ProviderInitiatedActions,
            ],
            vec![interaction],
        );

        registry
            .bootstrap_plugin(registration)
            .expect("data-load registration should admit");

        let resolved = registry
            .resolve_surface_action_for_method(
                tenant_a(),
                "test.dataload.surface",
                "load-data",
                None,
                Some("dataload_test_provider"),
                &AllProvidersVisible,
            )
            .expect("stored interaction should resolve");
        assert_eq!(
            resolved.interaction.http_method,
            surfaces::InteractionHttpMethod::Get,
            "admission must normalize the stored DataLoad method to GET"
        );
    }

    #[test]
    fn dataload_without_params_still_admits() {
        let registry = registry();
        let interaction = surfaces::InteractionDescriptor::new(
            surfaces::InteractionId::new("load-data").unwrap(),
            surfaces::InteractionKind::DataLoad,
            "Load Data",
            surfaces::InteractionTransport::ProviderProxied,
        );
        assert!(interaction.params.is_empty());
        let registration = registration_with_interactions(
            vec![
                surfaces::Capability::DataLoad,
                surfaces::Capability::ProviderInitiatedActions,
            ],
            vec![interaction],
        );

        let result = registry.bootstrap_plugin(registration);
        assert!(
            result.is_ok(),
            "param-less DataLoad must remain admissible (advisory-only telemetry): {:?}",
            result.err()
        );
    }

    /// Builds a `dup-action` registration with a DataLoad (normalized to GET)
    /// and a MutationAction (default POST) sharing the same interaction id,
    /// bootstraps it, and returns the registry for method-aware resolve tests.
    fn registry_with_multi_method_dup_action() -> SurfaceRegistry {
        let registry = registry();
        let id = surfaces::InteractionId::new("dup-action").unwrap();
        let data_load = surfaces::InteractionDescriptor::new(
            id.clone(),
            surfaces::InteractionKind::DataLoad,
            "Load",
            surfaces::InteractionTransport::ProviderProxied,
        );
        let mutation = surfaces::InteractionDescriptor::new(
            id,
            surfaces::InteractionKind::MutationAction,
            "Mutate",
            surfaces::InteractionTransport::ProviderProxied,
        );
        let registration = registration_with_interactions(
            vec![
                surfaces::Capability::DataLoad,
                surfaces::Capability::MutationAction,
                surfaces::Capability::ProviderInitiatedActions,
            ],
            vec![data_load, mutation],
        );

        registry
            .bootstrap_plugin(registration)
            .expect("multi-method same-id registration is accepted at admission (Task 4)");
        registry
    }

    #[test]
    fn id_only_resolve_errors_on_multi_method_id() {
        let registry = registry_with_multi_method_dup_action();

        let result = registry.resolve_surface_action_for_method(
            tenant_a(),
            "test.dataload.surface",
            "dup-action",
            None,
            Some("dataload_test_provider"),
            &AllProvidersVisible,
        );
        assert!(
            matches!(
                result,
                Err(SurfaceRegistryLookupError::MethodNotAllowed { ref allowed, .. })
                    if allowed.len() == 2
            ),
            "id-only (method: None) resolve on an ambiguous multi-method id must \
             return MethodNotAllowed listing every registered method, got {result:?}"
        );
    }

    #[test]
    fn resolve_surface_action_for_method_matches_exact_method_among_siblings() {
        let registry = registry_with_multi_method_dup_action();

        let get_resolved = registry
            .resolve_surface_action_for_method(
                tenant_a(),
                "test.dataload.surface",
                "dup-action",
                Some(&surfaces::InteractionHttpMethod::Get),
                Some("dataload_test_provider"),
                &AllProvidersVisible,
            )
            .expect("GET should resolve the DataLoad sibling");
        assert_eq!(
            get_resolved.interaction.kind,
            surfaces::InteractionKind::DataLoad
        );

        let post_resolved = registry
            .resolve_surface_action_for_method(
                tenant_a(),
                "test.dataload.surface",
                "dup-action",
                Some(&surfaces::InteractionHttpMethod::Post),
                Some("dataload_test_provider"),
                &AllProvidersVisible,
            )
            .expect("POST should resolve the MutationAction sibling");
        assert_eq!(
            post_resolved.interaction.kind,
            surfaces::InteractionKind::MutationAction
        );
    }

    #[test]
    fn resolve_surface_action_for_method_rejects_unregistered_method_among_siblings() {
        let registry = registry_with_multi_method_dup_action();

        let result = registry.resolve_surface_action_for_method(
            tenant_a(),
            "test.dataload.surface",
            "dup-action",
            Some(&surfaces::InteractionHttpMethod::Delete),
            Some("dataload_test_provider"),
            &AllProvidersVisible,
        );
        match result {
            Err(SurfaceRegistryLookupError::MethodNotAllowed { allowed, .. }) => {
                assert!(allowed.contains(&surfaces::InteractionHttpMethod::Get));
                assert!(allowed.contains(&surfaces::InteractionHttpMethod::Post));
            }
            other => panic!("expected MethodNotAllowed listing GET and POST, got {other:?}"),
        }
    }

    #[test]
    fn resolve_surface_action_for_method_mismatch_reports_allowed_and_permissions_in_known_method_order()
     {
        let registry = registry();
        let id = surfaces::InteractionId::new("dup-action").unwrap();
        let mut mutation = surfaces::InteractionDescriptor::new(
            id.clone(),
            surfaces::InteractionKind::MutationAction,
            "Mutate",
            surfaces::InteractionTransport::ProviderProxied,
        );
        mutation.required_action = Some(actions::SOFTWARE_UPDATE.to_string());
        let mut data_load = surfaces::InteractionDescriptor::new(
            id,
            surfaces::InteractionKind::DataLoad,
            "Load",
            surfaces::InteractionTransport::ProviderProxied,
        );
        data_load.required_action = Some(actions::SOFTWARE_READ.to_string());

        // Register MutationAction (POST) before DataLoad (GET) to prove
        // `allowed` is ordered by `KNOWN_INTERACTION_HTTP_METHODS`, not
        // registration order.
        let mut registration = registration_with_interactions(
            vec![
                surfaces::Capability::DataLoad,
                surfaces::Capability::MutationAction,
                surfaces::Capability::ProviderInitiatedActions,
            ],
            vec![mutation, data_load],
        );
        registration.surfaces[0].descriptor.required_action =
            Some(actions::HOSTS_UPDATE.to_string());

        registry
            .bootstrap_plugin(registration)
            .expect("multi-method same-id registration is accepted at admission");

        let result = registry.resolve_surface_action_for_method(
            tenant_a(),
            "test.dataload.surface",
            "dup-action",
            Some(&surfaces::InteractionHttpMethod::Delete),
            Some("dataload_test_provider"),
            &AllProvidersVisible,
        );

        match result {
            Err(SurfaceRegistryLookupError::MethodNotAllowed {
                allowed,
                descriptor_required_action,
                interaction_required_actions,
            }) => {
                assert_eq!(
                    allowed,
                    vec![
                        surfaces::InteractionHttpMethod::Get,
                        surfaces::InteractionHttpMethod::Post,
                    ],
                    "allowed must be ordered GET-before-POST (KNOWN_INTERACTION_HTTP_METHODS \
                     order), regardless that POST (MutationAction) was registered first"
                );
                assert_eq!(descriptor_required_action, Some(actions::HOSTS_UPDATE));
                assert_eq!(
                    interaction_required_actions,
                    vec![Some(actions::SOFTWARE_READ), Some(actions::SOFTWARE_UPDATE)],
                    "interaction_required_actions must align index-for-index with allowed"
                );
            }
            other => panic!("expected MethodNotAllowed with actions, got {other:?}"),
        }
    }

    #[test]
    fn resolve_surface_action_for_method_none_resolves_single_method_interaction() {
        let registry = registry();
        registry.register_provider_for_test(
            registration_for_service("service.provider-a", tenant_a()),
            Some(Uuid::now_v7()),
            Some("uptrakit-agent-ssh"),
        );

        let resolved = registry
            .resolve_surface_action_for_method(
                tenant_a(),
                "ssh.guest.panel",
                "refresh",
                None,
                Some("service.provider-a"),
                &AllProvidersVisible,
            )
            .expect("single registered method must resolve when method is None");
        assert_eq!(
            resolved.interaction.http_method,
            surfaces::InteractionHttpMethod::Post
        );
    }

    #[test]
    fn resolve_surface_action_for_method_rejects_mismatched_method_on_single_registration() {
        let registry = registry();
        registry.register_provider_for_test(
            registration_for_service("service.provider-a", tenant_a()),
            Some(Uuid::now_v7()),
            Some("uptrakit-agent-ssh"),
        );

        let result = registry.resolve_surface_action_for_method(
            tenant_a(),
            "ssh.guest.panel",
            "refresh",
            Some(&surfaces::InteractionHttpMethod::Get),
            Some("service.provider-a"),
            &AllProvidersVisible,
        );
        assert!(
            matches!(
                result,
                Err(SurfaceRegistryLookupError::MethodNotAllowed { ref allowed, .. })
                    if allowed == &[surfaces::InteractionHttpMethod::Post]
            ),
            "GET against a POST-only registration must be MethodNotAllowed([Post]), got {result:?}"
        );
    }

    #[test]
    fn resolve_surface_action_for_method_unknown_id_is_interaction_not_found_regardless_of_method()
    {
        let registry = registry();
        registry.register_provider_for_test(
            registration_for_service("service.provider-a", tenant_a()),
            Some(Uuid::now_v7()),
            Some("uptrakit-agent-ssh"),
        );

        let none_result = registry.resolve_surface_action_for_method(
            tenant_a(),
            "ssh.guest.panel",
            "does-not-exist",
            None,
            Some("service.provider-a"),
            &AllProvidersVisible,
        );
        assert!(matches!(
            none_result,
            Err(SurfaceRegistryLookupError::InteractionNotFound)
        ));

        let some_result = registry.resolve_surface_action_for_method(
            tenant_a(),
            "ssh.guest.panel",
            "does-not-exist",
            Some(&surfaces::InteractionHttpMethod::Post),
            Some("service.provider-a"),
            &AllProvidersVisible,
        );
        assert!(
            matches!(
                some_result,
                Err(SurfaceRegistryLookupError::InteractionNotFound)
            ),
            "an unknown interaction id must stay 404-shaped (InteractionNotFound) even \
             when a concrete method is requested — 405 only applies once the id is known, \
             got {some_result:?}"
        );
    }

    #[test]
    fn register_service_rejects_unsupported_generation_with_structured_reason() {
        let registry = registry();
        let mut registration = registration_for_service("service.provider-a", tenant_a());
        registration.framework_generation = surfaces::FrameworkGeneration::new(2, 0);

        let err = registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect_err("registration should fail");
        let rejection = rejection(err);
        assert_eq!(rejection.provider_id, "service.provider-a");
        assert_eq!(rejection.reasons.len(), 1);
        assert_eq!(
            rejection.reasons[0].code,
            SurfaceProviderRejectionCode::UnsupportedGeneration
        );
    }

    #[test]
    fn register_service_rejects_unparseable_descriptor_required_action() {
        let registry = registry();
        let mut registration = registration_for_service("service.provider-a", tenant_a());
        registration.surfaces[0].descriptor.required_action = Some("update_hosts".to_string());

        let err = registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect_err("unparseable descriptor required_action must be rejected");
        let rejection = rejection(err);
        assert!(
            rejection.reasons.iter().any(|reason| reason.code
                == SurfaceProviderRejectionCode::SchemaOrLimitFailure
                && reason.message.contains("descriptor required_action")),
            "rejection reasons: {:?}",
            rejection.reasons
        );
    }

    #[test]
    fn bootstrap_plugin_rejects_unparseable_descriptor_required_action() {
        let registry = registry();
        let mut registration = registration_for_plugin_same_surface("plugin-a");
        registration.surfaces[0].descriptor.required_action = Some("update_hosts".to_string());

        let err = registry
            .bootstrap_plugin(registration)
            .expect_err("unparseable descriptor required_action must be rejected");
        let rejection = rejection(err);
        assert!(
            rejection.reasons.iter().any(|reason| reason.code
                == SurfaceProviderRejectionCode::SchemaOrLimitFailure
                && reason.message.contains("descriptor required_action")),
            "rejection reasons: {:?}",
            rejection.reasons
        );
    }

    #[test]
    fn register_service_rejects_unparseable_interaction_required_action() {
        let registry = registry();
        let mut registration = registration_for_service("service.provider-a", tenant_a());
        registration.surfaces[0].interactions[0].required_action =
            Some("not-an-action".to_string());

        let err = registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect_err("unparseable interaction required_action must be rejected");
        let rejection = rejection(err);
        assert!(
            rejection.reasons.iter().any(|reason| reason.code
                == SurfaceProviderRejectionCode::SchemaOrLimitFailure
                && reason.message.contains("interaction required_action")),
            "rejection reasons: {:?}",
            rejection.reasons
        );
    }

    #[test]
    fn bootstrap_plugin_rejects_unparseable_interaction_required_action() {
        let registry = registry();
        let mut registration = registration_for_plugin_same_surface("plugin-a");
        registration.surfaces[0].interactions[0].required_action =
            Some("not-an-action".to_string());

        let err = registry
            .bootstrap_plugin(registration)
            .expect_err("unparseable interaction required_action must be rejected");
        let rejection = rejection(err);
        assert!(
            rejection.reasons.iter().any(|reason| reason.code
                == SurfaceProviderRejectionCode::SchemaOrLimitFailure
                && reason.message.contains("interaction required_action")),
            "rejection reasons: {:?}",
            rejection.reasons
        );
    }

    #[test]
    fn registration_with_parseable_unregistered_dynamic_action_admits() {
        let registry = registry();
        let mut registration = registration_for_service("service.provider-a", tenant_a());
        registration.surfaces[0].descriptor.required_action = Some("surface.ghost:use".to_string());

        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect(
                "a parseable dynamic-namespace action must admit even though no such \
                 surface is registered — registry membership is a decision-time concern, \
                 not an admission-time one",
            );

        let read = registry
            .resolve_surface_read(tenant_a(), "ssh.guest.panel", &AllProvidersVisible)
            .expect("read resolution should succeed");
        assert_eq!(
            read.required_action,
            Some(
                "surface.ghost:use"
                    .parse::<Action>()
                    .expect("dynamic surface action must parse")
            )
        );
    }

    #[test]
    fn legacy_required_permission_key_deserializes_and_rejects_invalid_value() {
        let registration = registration_for_service("service.provider-a", tenant_a());
        let mut value = serde_json::to_value(&registration).expect("registration should serialize");
        let interaction = &mut value["surfaces"][0]["interactions"][0];
        let obj = interaction
            .as_object_mut()
            .expect("interaction is an object");
        obj.remove("required_action");
        obj.insert(
            "required_permission".to_string(),
            serde_json::Value::String("update_hosts".to_string()),
        );

        let registration: surfaces::SurfaceRegistration =
            serde_json::from_value(value).expect("legacy key must deserialize via alias");
        assert_eq!(
            registration.surfaces[0].interactions[0].required_action,
            Some("update_hosts".to_string()),
            "legacy `required_permission` key must land in `required_action` via serde alias"
        );

        let registry = registry();
        let err = registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect_err("unparseable legacy-key value must be rejected");
        let rejection = rejection(err);
        assert!(
            rejection
                .reasons
                .iter()
                .any(|reason| reason.code == SurfaceProviderRejectionCode::SchemaOrLimitFailure),
            "rejection reasons: {:?}",
            rejection.reasons
        );
    }

    #[test]
    fn legacy_required_permission_key_admits_valid_value_and_stores_typed() {
        let registration = registration_for_service("service.provider-a", tenant_a());
        let mut value = serde_json::to_value(&registration).expect("registration should serialize");
        let interaction = &mut value["surfaces"][0]["interactions"][0];
        let obj = interaction
            .as_object_mut()
            .expect("interaction is an object");
        obj.remove("required_action");
        obj.insert(
            "required_permission".to_string(),
            serde_json::Value::String("hosts:update".to_string()),
        );

        let registration: surfaces::SurfaceRegistration =
            serde_json::from_value(value).expect("legacy key must deserialize via alias");
        assert_eq!(
            registration.surfaces[0].interactions[0].required_action,
            Some("hosts:update".to_string()),
            "legacy `required_permission` key must land in `required_action` via serde alias"
        );

        let registry = registry();
        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect("valid legacy-key value must admit");

        let resolved = registry
            .resolve_surface_action_for_method(
                tenant_a(),
                "ssh.guest.panel",
                "refresh",
                None,
                Some("service.provider-a"),
                &AllProvidersVisible,
            )
            .expect("resolve should succeed");
        assert_eq!(
            resolved.interaction_required_action,
            Some(actions::HOSTS_UPDATE)
        );
    }

    #[test]
    fn register_service_rejects_bare_provider_id_namespace() {
        let registry = registry();
        let registration = registration_for_service("bare-provider", tenant_a());

        let err = registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect_err("bare provider id should be rejected");
        let rejection = rejection(err);
        assert_eq!(rejection.provider_id, "bare-provider");
        assert_eq!(
            rejection.reasons[0].code,
            SurfaceProviderRejectionCode::InvalidTransport
        );
        assert!(
            rejection.reasons[0].message.contains("service."),
            "rejection must name the required namespace, got: {}",
            rejection.reasons[0].message
        );
    }

    #[test]
    fn bootstrap_builtin_rejects_unprefixed_provider_id() {
        let registry = registry();
        let registration = surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: "controller-unprefixed".to_string(),
                provider_kind: surfaces::ProviderKind::BuiltIn,
                provider_namespace: "controller".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::UniversalTargeting,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("controller.status").unwrap())
                    .label("Controller status")
                    .priority(0)
                    .slot("settings.below.global")
                    .scope(surfaces::Scope::Global)
                    .targeting(surfaces::Targeting::Universal)
                    .provider_kind(surfaces::ProviderKind::BuiltIn)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::UniversalTargeting,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    })
                    .build(),
                interactions: vec![],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        };

        let err = registry
            .bootstrap_builtin(registration)
            .expect_err("unprefixed built-in provider id should be rejected");
        let rejection = rejection(err);
        assert_eq!(rejection.provider_id, "controller-unprefixed");
        assert!(
            rejection.reasons.iter().any(|reason| {
                reason.code == SurfaceProviderRejectionCode::InvalidTransport
                    && reason.message.contains("builtin.")
            }),
            "rejection must name the required namespace, got: {:?}",
            rejection.reasons
        );
    }

    #[test]
    fn bootstrap_plugin_rejects_reserved_namespace_provider_id() {
        let registry = registry();
        let mut registration = registration_for_plugin_same_surface("provider-a");
        registration.provider.provider_id = "service.squatter".to_string();

        let err = registry
            .bootstrap_plugin(registration)
            .expect_err("reserved-namespace plugin provider id should be rejected");
        let rejection = rejection(err);
        assert_eq!(rejection.provider_id, "service.squatter");
        assert!(
            rejection.reasons.iter().any(|reason| {
                reason.code == SurfaceProviderRejectionCode::InvalidTransport
                    && reason.message.contains("reserved")
            }),
            "rejection must name the reserved namespace, got: {:?}",
            rejection.reasons
        );
    }

    #[test]
    fn two_service_providers_with_identical_dataload_contract_coexist() {
        // Regression: a targeted surface may be offered by multiple providers
        // when their contracts match. The `ssh-agent.hosts` surface carries a
        // DataLoad interaction declared with the default method (POST), which
        // admission normalizes to GET. The collision check must compare the
        // NORMALIZED form on both sides — otherwise the second (e.g. external
        // uptrakit-agent-ssh) provider mismatches the first (embedded) provider
        // whose stored copy was already rewritten to GET.
        let registry = registry();

        // Build a registration whose surface includes a DataLoad interaction
        // created with the wire-default method (POST).
        let with_dataload = |provider_id: &str| {
            let mut registration = registration_for_service(provider_id, tenant_a());
            registration
                .capabilities
                .0
                .insert(surfaces::Capability::DataLoad);
            let surface = &mut registration.surfaces[0];
            surface
                .descriptor
                .required_capabilities
                .0
                .insert(surfaces::Capability::DataLoad);
            let dataload = surfaces::InteractionDescriptor::new(
                surfaces::InteractionId::new("hosts").unwrap(),
                surfaces::InteractionKind::DataLoad,
                "List Hosts",
                surfaces::InteractionTransport::ProviderProxied,
            );
            assert_eq!(
                dataload.http_method,
                surfaces::InteractionHttpMethod::Post,
                "DataLoad interaction is authored with the default POST method"
            );
            surface.interactions.push(dataload);
            registration
        };

        // First provider (embedded ssh-agent) registers and is stored normalized.
        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                with_dataload("service.uptrakit-agent-ssh.embedded"),
            )
            .expect("first provider should register");

        // Second provider (external agent-ssh) with a byte-identical contract
        // body must be admitted, not rejected as a contract mismatch.
        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                with_dataload("service.uptrakit-agent-ssh.external"),
            )
            .expect("identical second provider must coexist, not mismatch");
    }

    #[test]
    fn register_service_is_batch_atomic_when_any_surface_is_invalid() {
        let registry = registry();
        let service_id = Uuid::now_v7();
        let mut registration = registration_for_service("service.provider-a", tenant_a());
        registration.surfaces.push(surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor::builder()
                .surface_id(surfaces::SurfaceId::new("ssh.invalid").unwrap())
                .label("Invalid")
                .priority(100)
                .slot("invalid.slot")
                .scope(surfaces::Scope::Tenant)
                .targeting(surfaces::Targeting::Targeted)
                .provider_kind(surfaces::ProviderKind::Service)
                .required_capabilities(surfaces::CapabilitySet::default())
                .root_node(surfaces::SurfaceNode::TextBlock {
                    text: "x".to_string(),
                })
                .build(),
            interactions: vec![],
            data_sources: vec![],
        });

        let err = registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect_err("registration should fail");
        let rejection = rejection(err);
        assert!(rejection.reasons.iter().any(|reason| {
            matches!(
                reason.code,
                SurfaceProviderRejectionCode::InvalidSlot
                    | SurfaceProviderRejectionCode::SchemaOrLimitFailure
            )
        }));
        assert_eq!(registry.provider_surface_count("service.provider-a"), 0);
        assert!(
            registry
                .list_surfaces_for_tenant(tenant_a(), None, None, &AllProvidersVisible)
                .is_empty()
        );
    }

    #[test]
    fn tenant_partitioning_keeps_tenant_surface_outside_other_tenants() {
        let registry = registry();
        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration_for_service("service.provider-a", tenant_a()),
            )
            .expect("registration should succeed");

        let tenant_a_surfaces =
            registry.list_surfaces_for_tenant(tenant_a(), None, None, &AllProvidersVisible);
        let tenant_b_surfaces =
            registry.list_surfaces_for_tenant(tenant_b(), None, None, &AllProvidersVisible);

        assert_eq!(tenant_a_surfaces.len(), 1);
        assert!(tenant_b_surfaces.is_empty());
    }

    #[test]
    fn registration_rejects_service_tenant_binding_that_does_not_match_connection_tenant() {
        let registry = registry();
        let err = registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_b()),
                registration_for_service("service.provider-a", tenant_a()),
            )
            .expect_err("mismatched tenant binding must fail");
        let rejection = rejection(err);
        assert!(rejection.reasons.iter().any(|reason| {
            reason.code == SurfaceProviderRejectionCode::InvalidTransport
                && reason.message.contains("tenant binding")
        }));
    }

    #[test]
    fn registration_rejects_sensitive_fields_without_encryption_metadata() {
        let registry = registry();
        let mut registration = registration_for_service("service.provider-a", tenant_a());
        registration.encryption_metadata = None;

        let err = registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect_err("registration should fail");
        let rejection = rejection(err);
        assert!(rejection.reasons.iter().any(|reason| {
            matches!(
                reason.code,
                SurfaceProviderRejectionCode::InvalidTransport
                    | SurfaceProviderRejectionCode::SchemaOrLimitFailure
            )
        }));
    }

    #[test]
    fn bootstrap_plugin_allows_controller_local_sensitive_fields_without_encryption_metadata() {
        let registry = registry();
        let registration = surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: "notifications.telegram".to_string(),
                provider_kind: surfaces::ProviderKind::Plugin,
                provider_namespace: "plugin".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::UniversalTargeting,
                surfaces::Capability::MutationAction,
                surfaces::Capability::SensitiveFields,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(
                        surfaces::SurfaceId::new("notifications.telegram.global-settings").unwrap(),
                    )
                    .label("Telegram")
                    .priority(200)
                    .slot(surfaces::SLOT_SETTINGS_BELOW_GLOBAL)
                    .scope(surfaces::Scope::Global)
                    .targeting(surfaces::Targeting::Universal)
                    .provider_kind(surfaces::ProviderKind::Plugin)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::UniversalTargeting,
                        surfaces::Capability::MutationAction,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "Telegram settings".to_string(),
                    })
                    .build(),
                interactions: vec![{
                    let mut i = surfaces::InteractionDescriptor::new(
                        surfaces::InteractionId::new("settings").unwrap(),
                        surfaces::InteractionKind::MutationAction,
                        "Refresh",
                        surfaces::InteractionTransport::ControllerLocal,
                    );
                    i.input_schema = Some(surfaces::SchemaContract::Object);
                    i.result_schema = Some(surfaces::SchemaContract::Object);
                    i.sensitive_fields = vec!["bot_token".to_string()];
                    i.timeout_seconds = Some(30);
                    i
                }],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        };

        registry
            .bootstrap_plugin(registration)
            .expect("controller-local sensitive plugin interactions should be admissible");
    }

    #[test]
    fn bootstrap_builtin_rejects_controller_local_sensitive_fields_without_runtime_support() {
        let registry = registry();
        let registration = surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: "builtin.controller-sensitive".to_string(),
                provider_kind: surfaces::ProviderKind::BuiltIn,
                provider_namespace: "controller".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::UniversalTargeting,
                surfaces::Capability::MutationAction,
                surfaces::Capability::SensitiveFields,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("controller.builtin.sensitive").unwrap())
                    .label("Built-in sensitive")
                    .priority(0)
                    .slot(surfaces::SLOT_SETTINGS_BELOW_GLOBAL)
                    .scope(surfaces::Scope::Global)
                    .targeting(surfaces::Targeting::Universal)
                    .provider_kind(surfaces::ProviderKind::BuiltIn)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::UniversalTargeting,
                        surfaces::Capability::MutationAction,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "Built-in sensitive action".to_string(),
                    })
                    .build(),
                interactions: vec![{
                    let mut i = surfaces::InteractionDescriptor::new(
                        surfaces::InteractionId::new("save_builtin_secret").unwrap(),
                        surfaces::InteractionKind::MutationAction,
                        "Save Built-in Secret",
                        surfaces::InteractionTransport::ControllerLocal,
                    );
                    i.input_schema = Some(surfaces::SchemaContract::Object);
                    i.result_schema = Some(surfaces::SchemaContract::Object);
                    i.sensitive_fields = vec!["secret".to_string()];
                    i.timeout_seconds = Some(30);
                    i
                }],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        };

        let err = registry
            .bootstrap_builtin(registration)
            .expect_err(
                "built-in controller-local sensitive interactions should be rejected until a built-in local executor exists",
            );
        let rejection = rejection(err);
        assert!(rejection.reasons.iter().any(|reason| {
            reason.code == SurfaceProviderRejectionCode::InvalidTransport
                && reason
                    .message
                    .contains("controller_local is currently supported only for plugin providers")
        }));
    }

    #[test]
    fn bootstrap_builtin_registers_surface_through_registry_path() {
        let registry = registry();
        let built_in_registration = surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: "builtin.controller".to_string(),
                provider_kind: surfaces::ProviderKind::BuiltIn,
                provider_namespace: "controller".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::UniversalTargeting,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("controller.status").unwrap())
                    .label("Controller status")
                    .priority(0)
                    .slot("settings.below.global")
                    .scope(surfaces::Scope::Global)
                    .targeting(surfaces::Targeting::Universal)
                    .provider_kind(surfaces::ProviderKind::BuiltIn)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::UniversalTargeting,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    })
                    .build(),
                interactions: vec![],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        };

        registry
            .bootstrap_builtin(built_in_registration)
            .expect("bootstrap should succeed");

        let surfaces =
            registry.list_surfaces_for_tenant(tenant_a(), None, None, &AllProvidersVisible);
        assert!(
            surfaces
                .iter()
                .any(|surface| surface.surface_id == "controller.status")
        );
    }

    #[test]
    fn bootstrap_plugin_registers_surface_through_registry_path() {
        let registry = registry();
        let plugin_registration = surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: "releases.docker".to_string(),
                provider_kind: surfaces::ProviderKind::Plugin,
                provider_namespace: "plugin".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::TargetedTargeting,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Tenant,
                tenant_id: Some(tenant_a().to_string()),
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("docker.item-host-actions").unwrap())
                    .label("Docker")
                    .priority(100)
                    .slot(surfaces::SLOT_SOFTWARE_ITEM_HOST_CONTEXT_MENU)
                    .scope(surfaces::Scope::Tenant)
                    .targeting(surfaces::Targeting::Targeted)
                    .required_action(actions::SOFTWARE_UPDATE)
                    .provider_kind(surfaces::ProviderKind::Plugin)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::TargetedTargeting,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "Docker host actions".to_string(),
                    })
                    .build(),
                interactions: vec![],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        };

        registry
            .bootstrap_plugin(plugin_registration)
            .expect("plugin bootstrap should succeed");

        let surfaces =
            registry.list_surfaces_for_tenant(tenant_a(), None, None, &AllProvidersVisible);
        assert!(
            surfaces
                .iter()
                .any(|surface| surface.surface_id == "docker.item-host-actions")
        );
    }

    #[test]
    fn bootstrap_plugin_catalog_keeps_proxmox_and_webhook_surfaces_visible() {
        let registry = registry();
        let mut saw_webhook_provider = false;

        for descriptor in uptrakit_plugin_infrastructure_registry::all_descriptors() {
            // Mirror the production bootstrap (controller-runtime boot): the
            // single `surfaces` ops field on each descriptor (ADR-0028).
            let mut registrations: Vec<surfaces::SurfaceRegistration> = Vec::new();
            if let Some(surface_ops) = descriptor.surfaces {
                registrations.extend(
                    (surface_ops.registrations)()
                        .into_iter()
                        .map(|registration| registration.to_wire(descriptor.type_id)),
                );
            }
            for registration in registrations {
                let provider_id = registration.provider.provider_id.clone();
                registry
                    .bootstrap_plugin(registration)
                    .expect("catalog plugin registration should be admitted");
                if provider_id == "notifications.webhook" {
                    saw_webhook_provider = true;
                }
            }
        }

        // Webhook has no feature gate — always contributes controller surfaces.
        assert!(
            saw_webhook_provider,
            "webhook provider should contribute shared-surface registrations"
        );

        let surfaces =
            registry.list_surfaces_for_tenant(tenant_a(), None, None, &AllProvidersVisible);
        assert!(
            surfaces
                .iter()
                .any(|surface| surface.surface_id == "notifications.webhook"),
            "notifications.webhook should remain visible after registry admission filtering"
        );

        // Proxmox registrations are feature-invariant (ADR-0032); assert visibility unconditionally.
        assert!(
            surfaces
                .iter()
                .any(|surface| surface.surface_id == "proxmox.hosts"),
            "proxmox.hosts should remain visible after registry admission filtering"
        );
    }

    #[test]
    fn registration_rejects_when_batch_interaction_limit_is_exceeded() {
        let config = SurfaceRegistryConfig {
            max_interactions_per_batch: 1,
            ..SurfaceRegistryConfig::default()
        };
        let registry = SurfaceRegistry::new(config);

        let mut registration = registration_for_service("service.provider-a", tenant_a());
        let duplicate = registration.surfaces[0].interactions[0].clone();
        registration.surfaces[0].interactions.push(duplicate);

        let err = registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect_err("registration should fail");
        let rejection = rejection(err);
        assert!(
            rejection.reasons.iter().any(|reason| {
                reason.code == SurfaceProviderRejectionCode::SchemaOrLimitFailure
            })
        );
    }

    #[test]
    fn registration_rejects_duplicate_universal_surface_in_same_scope() {
        let registry = registry();

        let mut first = registration_for_service("service.provider-a", tenant_a());
        first.surfaces[0].descriptor.targeting = surfaces::Targeting::Universal;
        first.surfaces[0].interactions.clear();
        first.surfaces[0].data_sources.clear();
        first.capabilities = surfaces::CapabilitySet::from_capabilities([
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::UniversalTargeting,
        ]);
        first.surfaces[0].descriptor.required_capabilities =
            surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::UniversalTargeting,
            ]);
        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                first,
            )
            .expect("first registration should succeed");

        let mut second = registration_for_service("service.provider-b", tenant_a());
        second.surfaces[0].descriptor.targeting = surfaces::Targeting::Universal;
        second.surfaces[0].interactions.clear();
        second.surfaces[0].data_sources.clear();
        second.capabilities = surfaces::CapabilitySet::from_capabilities([
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::UniversalTargeting,
        ]);
        second.surfaces[0].descriptor.required_capabilities =
            surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::UniversalTargeting,
            ]);
        let err = registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                second,
            )
            .expect_err("duplicate universal surface registration must fail");
        assert!(matches!(err, SurfaceRegistryError::ProviderConflict(_)));
    }

    #[test]
    fn registration_rejects_service_surface_when_built_in_owns_surface_id() {
        let registry = registry();

        registry
            .bootstrap_builtin(surfaces::SurfaceRegistration {
                provider: surfaces::ProviderIdentity {
                    provider_id: "builtin.controller".to_string(),
                    provider_kind: surfaces::ProviderKind::BuiltIn,
                    provider_namespace: "controller".to_string(),
                },
                framework_generation: surfaces::FrameworkGeneration::new(1, 0),
                capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::UniversalTargeting,
                    surfaces::Capability::TargetedTargeting,
                ]),
                effective_tenant_binding: surfaces::EffectiveTenantBinding {
                    scope: surfaces::Scope::Tenant,
                    tenant_id: Some(tenant_a().to_string()),
                },
                surfaces: vec![surfaces::RegisteredSurface {
                    descriptor: surfaces::SurfaceDescriptor::builder()
                        .surface_id(surfaces::SurfaceId::new("ssh.guest.panel").unwrap())
                        .label("Built-in")
                        .priority(0)
                        .slot("software.tabs")
                        .scope(surfaces::Scope::Tenant)
                        .targeting(surfaces::Targeting::Targeted)
                        .provider_kind(surfaces::ProviderKind::BuiltIn)
                        .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                            surfaces::Capability::TextBlockNode,
                            surfaces::Capability::TargetedTargeting,
                        ]))
                        .root_node(surfaces::SurfaceNode::TextBlock {
                            text: "built-in".to_string(),
                        })
                        .build(),
                    interactions: vec![],
                    data_sources: vec![],
                }],
                encryption_metadata: None,
            })
            .expect("built-in registration should succeed");

        let err = registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration_for_service("service.provider-a", tenant_a()),
            )
            .expect_err("service registration must fail when built-in already owns the surface");
        assert!(matches!(err, SurfaceRegistryError::ProviderConflict(_)));
    }

    #[test]
    fn targeted_shared_surface_requires_canonical_contract_match() {
        let registry = registry();
        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration_for_service("service.provider-a", tenant_a()),
            )
            .expect("first registration should succeed");

        let mut conflicting = registration_for_service("service.provider-b", tenant_a());
        conflicting.surfaces[0].descriptor.label = "Different label".to_string();

        let err = registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                conflicting,
            )
            .expect_err("conflicting targeted contract must fail");
        assert!(matches!(err, SurfaceRegistryError::ProviderConflict(_)));
    }

    #[test]
    fn conflicting_registration_does_not_mutate_existing_provider_state() {
        let registry = registry();
        let existing_service_id = Uuid::now_v7();
        registry
            .register_service(
                existing_service_id,
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration_for_service("service.provider-a", tenant_a()),
            )
            .expect("initial registration should succeed");

        let mut conflicting = registration_for_service("service.provider-b", tenant_a());
        conflicting.surfaces[0].descriptor.label = "Different label".to_string();
        let incoming_service_id = Uuid::now_v7();
        let err = registry
            .register_service(
                incoming_service_id,
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                conflicting,
            )
            .expect_err("conflicting registration should fail");

        assert!(matches!(err, SurfaceRegistryError::ProviderConflict(_)));
        assert_eq!(
            registry.provider_id_for_service(&existing_service_id),
            Some("service.provider-a".to_string())
        );
        assert_eq!(registry.provider_surface_count("service.provider-a"), 1);
        assert!(
            registry
                .provider_id_for_service(&incoming_service_id)
                .is_none()
        );
        assert_eq!(registry.provider_surface_count("service.provider-b"), 0);
    }

    #[test]
    fn register_service_can_atomically_rotate_provider_id_for_same_service() {
        let registry = registry();
        let service_id = Uuid::now_v7();
        registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration_for_service("service.provider-a", tenant_a()),
            )
            .expect("initial registration should succeed");

        registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration_for_service("service.provider-b", tenant_a()),
            )
            .expect("provider rotation for same service should succeed");

        assert_eq!(
            registry.provider_id_for_service(&service_id),
            Some("service.provider-b".to_string())
        );
        assert_eq!(registry.provider_surface_count("service.provider-a"), 0);
        assert_eq!(registry.provider_surface_count("service.provider-b"), 1);
    }

    #[test]
    fn tenant_partition_visibility_accepts_non_canonical_tenant_uuid_string() {
        let registry = registry();
        let tenant = Uuid::parse_str("aaaaaaaa-1111-1111-1111-111111111111").unwrap();
        let mut registration = registration_for_service("service.provider-a", tenant);
        registration.effective_tenant_binding.tenant_id = Some(tenant.to_string().to_uppercase());

        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant),
                registration,
            )
            .expect("registration should succeed");

        let visible = registry.list_surfaces_for_tenant(tenant, None, None, &AllProvidersVisible);
        assert_eq!(
            visible.len(),
            1,
            "tenant-scoped surface should be visible despite non-canonical UUID string"
        );
    }

    #[test]
    fn targeted_provider_discovery_hides_other_tenant_provider_metadata() {
        let registry = registry();
        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration_for_service("service.provider-a", tenant_a()),
            )
            .expect("tenant a registration should succeed");
        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_b()),
                registration_for_service("service.provider-b", tenant_b()),
            )
            .expect("tenant b registration should succeed");

        let providers = registry.list_targeted_providers_for_surface(
            "ssh.guest.panel",
            tenant_a(),
            &AllProvidersVisible,
        );
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].provider_id, "service.provider-a");
        assert!(providers[0].tenant_compatible);
    }

    #[test]
    fn list_surfaces_page_filter_uses_slot_mapping_not_surface_id_substring() {
        let registry = registry();
        let mut registration = registration_for_service("service.provider-a", tenant_a());
        registration.surfaces[0].descriptor.surface_id =
            surfaces::SurfaceId::new("settings.in.id").unwrap();
        registration.surfaces[0].descriptor.slot = surfaces::SLOT_SOFTWARE_TABS.to_string();

        let mut settings_surface = registration.surfaces[0].clone();
        settings_surface.descriptor.surface_id = surfaces::SurfaceId::new("other.surface").unwrap();
        settings_surface.descriptor.slot = surfaces::SLOT_SETTINGS_TABS.to_string();
        settings_surface.interactions[0].interaction_id =
            surfaces::InteractionId::new("refresh_settings").unwrap();
        registration.surfaces.push(settings_surface);

        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect("registration should succeed");

        let settings_page = registry.list_surfaces_for_tenant(
            tenant_a(),
            None,
            Some("settings"),
            &AllProvidersVisible,
        );
        assert_eq!(settings_page.len(), 1);
        assert_eq!(settings_page[0].surface_id, "other.surface");

        let software_page = registry.list_surfaces_for_tenant(
            tenant_a(),
            None,
            Some("software"),
            &AllProvidersVisible,
        );
        assert_eq!(software_page.len(), 1);
        assert_eq!(software_page[0].surface_id, "settings.in.id");
    }

    #[test]
    fn list_surfaces_page_filter_includes_host_detail_slot_on_hosts_page() {
        let registry = registry();
        let mut registration = registration_for_service("service.provider-a", tenant_a());
        registration.surfaces[0].descriptor.surface_id =
            surfaces::SurfaceId::new("host.detail.surface").unwrap();
        registration.surfaces[0].descriptor.slot = "host_detail.tabs".to_string();

        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect("registration should succeed");

        let hosts_page = registry.list_surfaces_for_tenant(
            tenant_a(),
            None,
            Some("hosts"),
            &AllProvidersVisible,
        );
        assert_eq!(hosts_page.len(), 1);
        assert_eq!(hosts_page[0].surface_id, "host.detail.surface");

        let settings_page = registry.list_surfaces_for_tenant(
            tenant_a(),
            None,
            Some("settings"),
            &AllProvidersVisible,
        );
        assert!(settings_page.is_empty());
    }

    #[test]
    fn list_surfaces_page_filter_includes_software_item_tabs_slot_on_software_page() {
        let registry = registry();
        let mut registration = registration_for_service("service.provider-a", tenant_a());
        registration.surfaces[0].descriptor.surface_id =
            surfaces::SurfaceId::new("software.item.tabs.surface").unwrap();
        registration.surfaces[0].descriptor.slot = surfaces::SLOT_SOFTWARE_ITEM_TABS.to_string();

        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect("registration should succeed");

        let software_page = registry.list_surfaces_for_tenant(
            tenant_a(),
            None,
            Some("software"),
            &AllProvidersVisible,
        );
        assert_eq!(software_page.len(), 1);
        assert_eq!(software_page[0].surface_id, "software.item.tabs.surface");

        let hosts_page = registry.list_surfaces_for_tenant(
            tenant_a(),
            None,
            Some("hosts"),
            &AllProvidersVisible,
        );
        assert!(hosts_page.is_empty());
    }

    #[test]
    fn shared_surface_resolution_uses_default_provider_order() {
        let registry = registry();
        registry.register_provider_for_test(
            registration_for_service("service.provider-a", tenant_a()),
            Some(Uuid::now_v7()),
            Some("uptrakit-agent-ssh"),
        );
        registry.register_provider_for_test(
            registration_for_plugin_same_surface("plugin-a"),
            None,
            None,
        );

        let read = registry
            .resolve_surface_read(tenant_a(), "ssh.guest.panel", &AllProvidersVisible)
            .expect("read resolution should succeed");
        assert_eq!(
            read.descriptor.provider_kind,
            surfaces::ProviderKind::Plugin
        );

        let action = registry.resolve_surface_action_for_method(
            tenant_a(),
            "ssh.guest.panel",
            "refresh",
            None,
            None,
            &AllProvidersVisible,
        );
        assert!(matches!(
            action,
            Err(SurfaceRegistryLookupError::TargetProviderRequired)
        ));

        let action = registry
            .resolve_surface_action_for_method(
                tenant_a(),
                "ssh.guest.panel",
                "refresh",
                None,
                Some("service.provider-a"),
                &AllProvidersVisible,
            )
            .expect("explicit service target should resolve");
        assert_eq!(action.provider_id, "service.provider-a");
        assert_eq!(action.provider_kind, surfaces::ProviderKind::Service);
    }

    #[test]
    fn plugin_provider_hidden_by_filter_is_absent_everywhere() {
        let registry = registry();
        registry.register_provider_for_test(
            registration_for_plugin_same_surface("plugin-a"),
            None,
            None,
        );

        let surfaces =
            registry.list_surfaces_for_tenant(tenant_a(), None, None, &DenyAllPluginProviders);
        assert!(
            surfaces
                .iter()
                .all(|surface| surface.provider_id != "plugin-a"),
            "a hidden plugin provider must not appear in the tenant catalog"
        );

        let providers = registry.list_targeted_providers_for_surface(
            "ssh.guest.panel",
            tenant_a(),
            &DenyAllPluginProviders,
        );
        assert!(
            providers.is_empty(),
            "the only registered provider is a hidden plugin, so no targeted providers should remain"
        );

        let read_result =
            registry.resolve_surface_read(tenant_a(), "ssh.guest.panel", &DenyAllPluginProviders);
        assert!(
            matches!(
                read_result,
                Err(SurfaceRegistryLookupError::SurfaceNotFound)
            ),
            "a surface with only a hidden plugin provider must resolve as not found, got {read_result:?}"
        );

        let action_result = registry.resolve_surface_action_for_method(
            tenant_a(),
            "ssh.guest.panel",
            "refresh",
            None,
            None,
            &DenyAllPluginProviders,
        );
        assert!(
            matches!(
                action_result,
                Err(SurfaceRegistryLookupError::SurfaceNotFound)
            ),
            "a surface with only a hidden plugin provider must resolve as not found, got {action_result:?}"
        );
    }

    #[test]
    fn service_provider_unaffected_by_deny_filter() {
        let registry = registry();
        registry.register_provider_for_test(
            registration_for_service("service.provider-a", tenant_a()),
            Some(Uuid::now_v7()),
            Some("uptrakit-agent-ssh"),
        );

        let surfaces =
            registry.list_surfaces_for_tenant(tenant_a(), None, None, &DenyAllPluginProviders);
        assert!(
            surfaces
                .iter()
                .any(|surface| surface.provider_id == "service.provider-a"),
            "a Service-kind provider must stay listed under the deny-all-plugins filter"
        );

        let providers = registry.list_targeted_providers_for_surface(
            "ssh.guest.panel",
            tenant_a(),
            &DenyAllPluginProviders,
        );
        assert!(
            providers
                .iter()
                .any(|provider| provider.provider_id == "service.provider-a"),
            "a Service-kind provider must remain a targeted candidate under the deny-all-plugins filter"
        );
    }

    /// Builds a minimal plugin [`SurfaceRegistration`] with the given root node and
    /// interaction list. Used to test per-surface validation in `validate_registration_basics`.
    fn make_minimal_plugin_registration_with_root(
        root_node: surfaces::SurfaceNode,
        interactions: Vec<surfaces::InteractionDescriptor>,
    ) -> surfaces::SurfaceRegistration {
        surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: "test_provider".to_string(),
                provider_kind: surfaces::ProviderKind::Plugin,
                provider_namespace: "plugin".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::SectionNode,
                surfaces::Capability::UniversalTargeting,
                surfaces::Capability::MutationAction,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("test.surface").unwrap())
                    .label("Test Surface")
                    .priority(100)
                    .slot(surfaces::SLOT_SETTINGS_BELOW_GLOBAL)
                    .scope(surfaces::Scope::Global)
                    .targeting(surfaces::Targeting::Universal)
                    .provider_kind(surfaces::ProviderKind::Plugin)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::SectionNode,
                        surfaces::Capability::UniversalTargeting,
                        surfaces::Capability::MutationAction,
                    ]))
                    .root_node(root_node)
                    .build(),
                interactions,
                data_sources: vec![],
            }],
            encryption_metadata: None,
        }
    }

    #[test]
    fn header_action_ids_unknown_interaction_id_is_rejected() {
        let registration = make_minimal_plugin_registration_with_root(
            surfaces::SurfaceNode::section_with_header_actions(
                None::<String>,
                vec![surfaces::InteractionId::new("nonexistent").unwrap()],
                vec![],
            ),
            vec![/* no interactions */],
        );
        let result = registry().bootstrap_plugin(registration);
        assert!(result.is_err(), "unknown header_action_id must be rejected");
        let rejection = rejection(result.unwrap_err());
        assert!(rejection.reasons.iter().any(|reason| {
            reason.code == SurfaceProviderRejectionCode::SchemaOrLimitFailure
                && reason.message.contains("unknown interaction")
                && reason.message.contains("nonexistent")
        }));
    }

    #[test]
    fn header_action_ids_form_submit_kind_is_rejected() {
        let interaction_id = surfaces::InteractionId::new("submit-action").unwrap();
        let registration = make_minimal_plugin_registration_with_root(
            surfaces::SurfaceNode::section_with_header_actions(
                None::<String>,
                vec![interaction_id.clone()],
                vec![],
            ),
            vec![surfaces::InteractionDescriptor::new(
                interaction_id.clone(),
                surfaces::InteractionKind::FormSubmit,
                "Submit",
                surfaces::InteractionTransport::ControllerLocal,
            )],
        );
        let result = registry().bootstrap_plugin(registration);
        assert!(
            result.is_err(),
            "FormSubmit kind must be rejected in header_action_ids"
        );
        let rejection = rejection(result.unwrap_err());
        assert!(rejection.reasons.iter().any(|reason| {
            reason.code == SurfaceProviderRejectionCode::SchemaOrLimitFailure
                && reason.message.contains("header_action_ids")
                && reason.message.contains("Workflow or MutationAction")
        }));
    }

    #[test]
    fn header_action_ids_mutation_action_with_form_ui_is_rejected() {
        let interaction_id = surfaces::InteractionId::new("form-action").unwrap();
        let registration = make_minimal_plugin_registration_with_root(
            surfaces::SurfaceNode::section_with_header_actions(
                None::<String>,
                vec![interaction_id.clone()],
                vec![],
            ),
            vec![{
                let mut i = surfaces::InteractionDescriptor::new(
                    interaction_id.clone(),
                    surfaces::InteractionKind::MutationAction,
                    "Save",
                    surfaces::InteractionTransport::ControllerLocal,
                );
                i.form_ui = Some(surfaces::FormUiDescriptor {
                    fields: vec![],
                    pre_load_interaction_id: None,
                });
                i
            }],
        );
        let result = registry().bootstrap_plugin(registration);
        assert!(
            result.is_err(),
            "MutationAction with form_ui must be rejected in header_action_ids"
        );
        let rejection = rejection(result.unwrap_err());
        assert!(rejection.reasons.iter().any(|reason| {
            reason.code == SurfaceProviderRejectionCode::SchemaOrLimitFailure
                && reason.message.contains("header_action_ids")
                && reason.message.contains("must not have form_ui set")
        }));
    }

    #[test]
    fn header_action_ids_mutation_action_without_form_ui_is_accepted() {
        let interaction_id = surfaces::InteractionId::new("btn-action").unwrap();
        let registration = make_minimal_plugin_registration_with_root(
            surfaces::SurfaceNode::section_with_header_actions(
                None::<String>,
                vec![interaction_id.clone()],
                vec![],
            ),
            vec![surfaces::InteractionDescriptor::new(
                interaction_id.clone(),
                surfaces::InteractionKind::MutationAction,
                "Refresh",
                surfaces::InteractionTransport::ControllerLocal,
            )],
        );
        let result = registry().bootstrap_plugin(registration);
        assert!(
            result.is_ok(),
            "MutationAction without form_ui must be accepted: {:?}",
            result.err()
        );
    }

    #[test]
    fn registration_rejects_data_source_pagination_above_1000() {
        let registry = registry();
        let mut registration = registration_for_service("service.provider-a", tenant_a());
        registration.capabilities = surfaces::CapabilitySet::from_capabilities([
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::TargetedTargeting,
            surfaces::Capability::ProviderInitiatedActions,
            surfaces::Capability::MutationAction,
            surfaces::Capability::SensitiveFields,
            surfaces::Capability::ProviderQueryDataSource,
        ]);
        registration.surfaces[0]
            .data_sources
            .push(surfaces::DataSourceDescriptor {
                data_source_id: surfaces::DataSourceId::new("provider.items").unwrap(),
                kind: surfaces::DataSourceKind::ProviderQuery {
                    operation_id: "list-items".to_string(),
                },
                result_schema: surfaces::SchemaContract::Array,
                pagination: Some(surfaces::DataSourcePagination {
                    default_page_size: 1001,
                    max_page_size: 1001,
                }),
                sorting: None,
                filtering: None,
                refresh_policy: surfaces::RefreshPolicy::Manual,
                empty_state: None,
            });

        let err = registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration,
            )
            .expect_err("pagination above 1000 must fail");
        let rejection = rejection(err);
        assert!(rejection.reasons.iter().any(|reason| {
            reason.code == SurfaceProviderRejectionCode::SchemaOrLimitFailure
                && reason.message.contains("page_size")
        }));
    }
}
