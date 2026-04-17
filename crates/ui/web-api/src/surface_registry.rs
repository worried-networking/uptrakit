use std::collections::{BTreeSet, HashMap, HashSet};

use parking_lot::Mutex;
use uuid::Uuid;

use uptrakit_internal_wire::surfaces;
use uptrakit_shared_types::Permission;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum SurfaceProviderRejectionCode {
    UnsupportedGeneration,
    MissingCapability,
    InvalidSlot,
    InvalidTransport,
    SchemaOrLimitFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SurfaceProviderRejectionReason {
    pub code: SurfaceProviderRejectionCode,
    pub message: String,
    pub surface_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SurfaceProviderRejection {
    pub provider_id: String,
    pub reasons: Vec<SurfaceProviderRejectionReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
pub struct SurfaceRegistryConfig {
    pub supported_generation: surfaces::FrameworkGenerationRange,
    pub required_capabilities: surfaces::CapabilitySet,
    pub allowed_controller_queries: HashSet<String>,
    pub allowed_sse_topics: HashSet<String>,
    pub allowed_direct_builtin_operations: HashSet<String>,
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
            allowed_direct_builtin_operations: HashSet::new(),
            max_data_source_page_size: 1000,
            max_surfaces_per_batch: 64,
            max_interactions_per_batch: 256,
            max_contract_depth: 16,
            max_registration_payload_bytes: 512 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SurfaceCatalogItem {
    pub surface_id: String,
    pub slot: String,
    pub provider_id: String,
    pub targeting: surfaces::Targeting,
    pub descriptor: surfaces::SurfaceDescriptor,
}

#[derive(Debug, Clone)]
pub struct SurfaceProviderSummary {
    pub provider_id: String,
    pub provider_kind: surfaces::ProviderKind,
    pub tenant_compatible: bool,
    pub targeting: surfaces::Targeting,
    pub service_id: Option<Uuid>,
    pub service_app_name: Option<String>,
    pub encryption_metadata: Option<surfaces::ProviderEncryptionMetadata>,
}

#[derive(Debug, Clone)]
struct ProviderRegistration {
    registration: surfaces::SurfaceRegistration,
    service_id: Option<Uuid>,
    service_app_name: Option<String>,
}

#[derive(Default)]
struct SurfaceRegistryInner {
    providers: HashMap<String, ProviderRegistration>,
    service_to_provider: HashMap<Uuid, String>,
    surface_to_providers: HashMap<String, BTreeSet<String>>,
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
        registration: surfaces::SurfaceRegistration,
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
            },
        );
        Ok(())
    }

    pub fn bootstrap_builtin(
        &self,
        registration: surfaces::SurfaceRegistration,
    ) -> Result<(), SurfaceRegistryError> {
        self.validate_registration_basics(surfaces::ProviderKind::BuiltIn, &registration, None)?;
        let provider_id = registration.provider.provider_id.clone();
        let mut inner = self.inner.lock();
        self.validate_registration_admission_locked(&inner, &registration, None, None)?;

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
            },
        );
        Ok(())
    }

    pub fn bootstrap_plugin(
        &self,
        registration: surfaces::SurfaceRegistration,
    ) -> Result<(), SurfaceRegistryError> {
        self.validate_registration_basics(surfaces::ProviderKind::Plugin, &registration, None)?;
        let provider_id = registration.provider.provider_id.clone();
        let mut inner = self.inner.lock();
        self.validate_registration_admission_locked(&inner, &registration, None, None)?;

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
    ) -> Vec<SurfaceCatalogItem> {
        let inner = self.inner.lock();
        let mut items = Vec::new();

        for (provider_id, provider) in &inner.providers {
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
                    targeting: registered.descriptor.targeting.clone(),
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
                targeting: surface.descriptor.targeting.clone(),
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

    pub fn resolve_surface_action(
        &self,
        tenant_id: Uuid,
        surface_id: &str,
        interaction_id: &str,
        target_provider_id: Option<&str>,
    ) -> Result<ResolvedSurfaceAction, SurfaceRegistryLookupError> {
        let providers = self.list_targeted_providers_for_surface(surface_id, tenant_id);
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
        let surface = provider
            .registration
            .surfaces
            .iter()
            .find(|surface| surface.descriptor.surface_id.as_str() == surface_id)
            .ok_or(SurfaceRegistryLookupError::SurfaceNotFound)?;
        let interaction = surface
            .interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == interaction_id)
            .cloned()
            .ok_or(SurfaceRegistryLookupError::InteractionNotFound)?;

        Ok(ResolvedSurfaceAction {
            provider_id: selected_provider.provider_id.clone(),
            service_id: selected_provider.service_id,
            descriptor: surface.descriptor.clone(),
            interaction,
            encryption_metadata: selected_provider.encryption_metadata.clone(),
            provider_kind: selected_provider.provider_kind,
            service_app_name: provider.service_app_name.clone(),
        })
    }

    pub fn resolve_surface_read(
        &self,
        tenant_id: Uuid,
        surface_id: &str,
    ) -> Result<ResolvedSurfaceRead, SurfaceRegistryLookupError> {
        let provider_ids = {
            let inner = self.inner.lock();
            inner
                .surface_to_providers
                .get(surface_id)
                .cloned()
                .unwrap_or_default()
        };
        if provider_ids.is_empty() {
            return Err(SurfaceRegistryLookupError::SurfaceNotFound);
        }

        let candidates = preferred_provider_candidates(
            self.list_targeted_providers_for_surface(surface_id, tenant_id),
        )?;
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
        let surface = provider
            .registration
            .surfaces
            .iter()
            .find(|surface| surface.descriptor.surface_id.as_str() == surface_id)
            .ok_or(SurfaceRegistryLookupError::SurfaceNotFound)?;

        Ok(ResolvedSurfaceRead {
            descriptor: surface.descriptor.clone(),
            interactions: surface.interactions.clone(),
            data_sources: surface.data_sources.clone(),
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

            if let Some(permission) = &surface.descriptor.required_permission
                && permission.parse::<Permission>().is_err()
            {
                reasons.push(SurfaceProviderRejectionReason {
                    code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                    message: format!("invalid descriptor permission `{permission}`"),
                    surface_id: surface_id.clone(),
                });
            }

            for interaction in &surface.interactions {
                if let Some(permission) = &interaction.required_permission
                    && permission.parse::<Permission>().is_err()
                {
                    reasons.push(SurfaceProviderRejectionReason {
                        code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                        message: format!("invalid interaction permission `{permission}`"),
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
                        surfaces::InteractionTransport::DirectBuiltInApi { .. } => {
                            reasons.push(SurfaceProviderRejectionReason {
                                code: SurfaceProviderRejectionCode::InvalidTransport,
                                message:
                                    "sensitive fields are not supported on direct built-in API transport"
                                        .to_string(),
                                surface_id: surface_id.clone(),
                            });
                        }
                    }
                }

                if let surfaces::InteractionTransport::DirectBuiltInApi { operation_id } =
                    &interaction.transport
                {
                    if source_kind != surfaces::ProviderKind::BuiltIn {
                        reasons.push(SurfaceProviderRejectionReason {
                            code: SurfaceProviderRejectionCode::InvalidTransport,
                            message:
                                "non-built-in providers cannot use direct built-in API transport"
                                    .to_string(),
                            surface_id: surface_id.clone(),
                        });
                    }
                    if !self
                        .config
                        .allowed_direct_builtin_operations
                        .contains(operation_id.as_str())
                    {
                        reasons.push(SurfaceProviderRejectionReason {
                            code: SurfaceProviderRejectionCode::InvalidTransport,
                            message: format!(
                                "direct built-in operation `{}` is not allowlisted",
                                operation_id
                            ),
                            surface_id: surface_id.clone(),
                        });
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
        }

        if let Err(err) = registration.validate_against(&surfaces::SurfaceRegistrationPolicy {
            supported_generation: self.config.supported_generation.clone(),
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

    #[cfg(test)]
    pub(crate) fn register_provider_for_test(
        &self,
        registration: surfaces::SurfaceRegistration,
        service_id: Option<Uuid>,
        service_app_name: Option<&str>,
    ) {
        let provider_id = registration.provider.provider_id.clone();
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
            },
        );
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedSurfaceAction {
    pub provider_id: String,
    pub provider_kind: surfaces::ProviderKind,
    pub service_id: Option<Uuid>,
    pub service_app_name: Option<String>,
    pub descriptor: surfaces::SurfaceDescriptor,
    pub interaction: surfaces::InteractionDescriptor,
    pub encryption_metadata: Option<surfaces::ProviderEncryptionMetadata>,
}

#[derive(Debug, Clone)]
pub struct ResolvedSurfaceRead {
    pub descriptor: surfaces::SurfaceDescriptor,
    pub interactions: Vec<surfaces::InteractionDescriptor>,
    pub data_sources: Vec<surfaces::DataSourceDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceRegistryLookupError {
    SurfaceNotFound,
    InteractionNotFound,
    TargetProviderRequired,
    InvalidProvider(String),
    NoTenantCompatibleProvider,
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
                descriptor: surfaces::SurfaceDescriptor {
                    surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                    label: "SSH Guest Panel".to_string(),
                    priority: 100,
                    slot: "software.tabs".to_string(),
                    scope: surfaces::Scope::Tenant,
                    targeting: surfaces::Targeting::Targeted,
                    required_permission: Some("view_software".to_string()),
                    provider_kind: surfaces::ProviderKind::Service,
                    required_capabilities: surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::TargetedTargeting,
                        surfaces::Capability::MutationAction,
                    ]),
                    root_node: surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    },
                },
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: None,
                    required_permission: Some("update_software".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Object),
                    sensitive_fields: vec!["token".to_string()],
                    timeout_seconds: Some(30),
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ProviderProxied,
                    workflow_steps: vec![],
                    form_ui: None,
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
                descriptor: surfaces::SurfaceDescriptor {
                    surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                    label: "Plugin SSH Guest Panel".to_string(),
                    priority: 100,
                    slot: "software.tabs".to_string(),
                    scope: surfaces::Scope::Tenant,
                    targeting: surfaces::Targeting::Universal,
                    required_permission: Some("view_software".to_string()),
                    provider_kind: surfaces::ProviderKind::Plugin,
                    required_capabilities: surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::UniversalTargeting,
                        surfaces::Capability::MutationAction,
                    ]),
                    root_node: surfaces::SurfaceNode::TextBlock {
                        text: "plugin-fallback".to_string(),
                    },
                },
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: None,
                    required_permission: Some("update_software".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Object),
                    sensitive_fields: vec![],
                    timeout_seconds: Some(30),
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
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

    #[test]
    fn register_service_rejects_unsupported_generation_with_structured_reason() {
        let registry = registry();
        let mut registration = registration_for_service("provider-a", tenant_a());
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
        assert_eq!(rejection.provider_id, "provider-a");
        assert_eq!(rejection.reasons.len(), 1);
        assert_eq!(
            rejection.reasons[0].code,
            SurfaceProviderRejectionCode::UnsupportedGeneration
        );
    }

    #[test]
    fn register_service_is_batch_atomic_when_any_surface_is_invalid() {
        let registry = registry();
        let service_id = Uuid::now_v7();
        let mut registration = registration_for_service("provider-a", tenant_a());
        registration.surfaces.push(surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new("ssh.invalid").unwrap(),
                label: "Invalid".to_string(),
                priority: 100,
                slot: "invalid.slot".to_string(),
                scope: surfaces::Scope::Tenant,
                targeting: surfaces::Targeting::Targeted,
                required_permission: None,
                provider_kind: surfaces::ProviderKind::Service,
                required_capabilities: surfaces::CapabilitySet::default(),
                root_node: surfaces::SurfaceNode::TextBlock {
                    text: "x".to_string(),
                },
            },
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
        assert_eq!(registry.provider_surface_count("provider-a"), 0);
        assert!(
            registry
                .list_surfaces_for_tenant(tenant_a(), None, None)
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
                registration_for_service("provider-a", tenant_a()),
            )
            .expect("registration should succeed");

        let tenant_a_surfaces = registry.list_surfaces_for_tenant(tenant_a(), None, None);
        let tenant_b_surfaces = registry.list_surfaces_for_tenant(tenant_b(), None, None);

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
                registration_for_service("provider-a", tenant_a()),
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
        let mut registration = registration_for_service("provider-a", tenant_a());
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
                provider_id: "plugin.notifications_telegram".to_string(),
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
                descriptor: surfaces::SurfaceDescriptor {
                    surface_id: surfaces::SurfaceId::new("notifications.telegram.global_settings")
                        .unwrap(),
                    label: "Telegram".to_string(),
                    priority: 200,
                    slot: surfaces::SLOT_SETTINGS_BELOW_GLOBAL.to_string(),
                    scope: surfaces::Scope::Global,
                    targeting: surfaces::Targeting::Universal,
                    required_permission: None,
                    provider_kind: surfaces::ProviderKind::Plugin,
                    required_capabilities: surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::UniversalTargeting,
                        surfaces::Capability::MutationAction,
                    ]),
                    root_node: surfaces::SurfaceNode::TextBlock {
                        text: "Telegram settings".to_string(),
                    },
                },
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("save_global_telegram").unwrap(),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: None,
                    required_permission: None,
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Object),
                    sensitive_fields: vec!["bot_token".to_string()],
                    timeout_seconds: Some(30),
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
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
                provider_id: "controller.builtin_sensitive".to_string(),
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
                descriptor: surfaces::SurfaceDescriptor {
                    surface_id: surfaces::SurfaceId::new("controller.builtin.sensitive").unwrap(),
                    label: "Built-in sensitive".to_string(),
                    priority: 0,
                    slot: surfaces::SLOT_SETTINGS_BELOW_GLOBAL.to_string(),
                    scope: surfaces::Scope::Global,
                    targeting: surfaces::Targeting::Universal,
                    required_permission: None,
                    provider_kind: surfaces::ProviderKind::BuiltIn,
                    required_capabilities: surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::UniversalTargeting,
                        surfaces::Capability::MutationAction,
                    ]),
                    root_node: surfaces::SurfaceNode::TextBlock {
                        text: "Built-in sensitive action".to_string(),
                    },
                },
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("save_builtin_secret").unwrap(),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: None,
                    required_permission: None,
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Object),
                    sensitive_fields: vec!["secret".to_string()],
                    timeout_seconds: Some(30),
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
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
                provider_id: "controller.builtin".to_string(),
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
                descriptor: surfaces::SurfaceDescriptor {
                    surface_id: surfaces::SurfaceId::new("controller.status").unwrap(),
                    label: "Controller status".to_string(),
                    priority: 0,
                    slot: "settings.below.global".to_string(),
                    scope: surfaces::Scope::Global,
                    targeting: surfaces::Targeting::Universal,
                    required_permission: None,
                    provider_kind: surfaces::ProviderKind::BuiltIn,
                    required_capabilities: surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::UniversalTargeting,
                    ]),
                    root_node: surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    },
                },
                interactions: vec![],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        };

        registry
            .bootstrap_builtin(built_in_registration)
            .expect("bootstrap should succeed");

        let surfaces = registry.list_surfaces_for_tenant(tenant_a(), None, None);
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
                provider_id: "plugin.releases_docker".to_string(),
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
                descriptor: surfaces::SurfaceDescriptor {
                    surface_id: surfaces::SurfaceId::new("docker.item-host-actions").unwrap(),
                    label: "Docker".to_string(),
                    priority: 100,
                    slot: surfaces::SLOT_SOFTWARE_ITEM_HOST_CONTEXT_MENU.to_string(),
                    scope: surfaces::Scope::Tenant,
                    targeting: surfaces::Targeting::Targeted,
                    required_permission: Some("update_software".to_string()),
                    provider_kind: surfaces::ProviderKind::Plugin,
                    required_capabilities: surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::TargetedTargeting,
                    ]),
                    root_node: surfaces::SurfaceNode::TextBlock {
                        text: "Docker host actions".to_string(),
                    },
                },
                interactions: vec![],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        };

        registry
            .bootstrap_plugin(plugin_registration)
            .expect("plugin bootstrap should succeed");

        let surfaces = registry.list_surfaces_for_tenant(tenant_a(), None, None);
        assert!(
            surfaces
                .iter()
                .any(|surface| surface.surface_id == "docker.item-host-actions")
        );
    }

    #[test]
    fn bootstrap_plugin_catalog_keeps_proxmox_and_webhook_surfaces_visible() {
        let registry = registry();
        let mut saw_proxmox_provider = false;
        let mut saw_webhook_provider = false;

        for descriptor in uptrakit_plugin_infrastructure_registry::all_descriptors() {
            let Some(surface_ops) = descriptor.surfaces else {
                continue;
            };
            for registration in (surface_ops.registrations)() {
                let provider_id = registration.provider.provider_id.clone();
                registry
                    .bootstrap_plugin(registration)
                    .expect("catalog plugin registration should be admitted");
                if provider_id == "plugin.infrastructure_proxmox" {
                    saw_proxmox_provider = true;
                }
                if provider_id == "plugin.webhook" {
                    saw_webhook_provider = true;
                }
            }
        }

        assert!(
            saw_proxmox_provider,
            "proxmox provider should contribute shared-surface registrations"
        );
        assert!(
            saw_webhook_provider,
            "webhook provider should contribute shared-surface registrations"
        );

        let surfaces = registry.list_surfaces_for_tenant(tenant_a(), None, None);
        assert!(
            surfaces
                .iter()
                .any(|surface| surface.surface_id == "proxmox.hosts"),
            "proxmox.hosts should remain visible after registry admission filtering"
        );
        assert!(
            surfaces
                .iter()
                .any(|surface| surface.surface_id == "notifications.webhook"),
            "notifications.webhook should remain visible after registry admission filtering"
        );
    }

    #[test]
    fn registration_rejects_when_batch_interaction_limit_is_exceeded() {
        let config = SurfaceRegistryConfig {
            max_interactions_per_batch: 1,
            ..SurfaceRegistryConfig::default()
        };
        let registry = SurfaceRegistry::new(config);

        let mut registration = registration_for_service("provider-a", tenant_a());
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

        let mut first = registration_for_service("provider-a", tenant_a());
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

        let mut second = registration_for_service("provider-b", tenant_a());
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
                    provider_id: "controller.builtin".to_string(),
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
                    descriptor: surfaces::SurfaceDescriptor {
                        surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                        label: "Built-in".to_string(),
                        priority: 0,
                        slot: "software.tabs".to_string(),
                        scope: surfaces::Scope::Tenant,
                        targeting: surfaces::Targeting::Targeted,
                        required_permission: None,
                        provider_kind: surfaces::ProviderKind::BuiltIn,
                        required_capabilities: surfaces::CapabilitySet::from_capabilities([
                            surfaces::Capability::TextBlockNode,
                            surfaces::Capability::TargetedTargeting,
                        ]),
                        root_node: surfaces::SurfaceNode::TextBlock {
                            text: "built-in".to_string(),
                        },
                    },
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
                registration_for_service("provider-a", tenant_a()),
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
                registration_for_service("provider-a", tenant_a()),
            )
            .expect("first registration should succeed");

        let mut conflicting = registration_for_service("provider-b", tenant_a());
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
                registration_for_service("provider-a", tenant_a()),
            )
            .expect("initial registration should succeed");

        let mut conflicting = registration_for_service("provider-b", tenant_a());
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
            Some("provider-a".to_string())
        );
        assert_eq!(registry.provider_surface_count("provider-a"), 1);
        assert!(
            registry
                .provider_id_for_service(&incoming_service_id)
                .is_none()
        );
        assert_eq!(registry.provider_surface_count("provider-b"), 0);
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
                registration_for_service("provider-a", tenant_a()),
            )
            .expect("initial registration should succeed");

        registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_a()),
                registration_for_service("provider-b", tenant_a()),
            )
            .expect("provider rotation for same service should succeed");

        assert_eq!(
            registry.provider_id_for_service(&service_id),
            Some("provider-b".to_string())
        );
        assert_eq!(registry.provider_surface_count("provider-a"), 0);
        assert_eq!(registry.provider_surface_count("provider-b"), 1);
    }

    #[test]
    fn tenant_partition_visibility_accepts_non_canonical_tenant_uuid_string() {
        let registry = registry();
        let tenant = Uuid::parse_str("aaaaaaaa-1111-1111-1111-111111111111").unwrap();
        let mut registration = registration_for_service("provider-a", tenant);
        registration.effective_tenant_binding.tenant_id = Some(tenant.to_string().to_uppercase());

        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant),
                registration,
            )
            .expect("registration should succeed");

        let visible = registry.list_surfaces_for_tenant(tenant, None, None);
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
                registration_for_service("provider-a", tenant_a()),
            )
            .expect("tenant a registration should succeed");
        registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(tenant_b()),
                registration_for_service("provider-b", tenant_b()),
            )
            .expect("tenant b registration should succeed");

        let providers = registry.list_targeted_providers_for_surface("ssh.guest.panel", tenant_a());
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].provider_id, "provider-a");
        assert!(providers[0].tenant_compatible);
    }

    #[test]
    fn list_surfaces_page_filter_uses_slot_mapping_not_surface_id_substring() {
        let registry = registry();
        let mut registration = registration_for_service("provider-a", tenant_a());
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

        let settings_page = registry.list_surfaces_for_tenant(tenant_a(), None, Some("settings"));
        assert_eq!(settings_page.len(), 1);
        assert_eq!(settings_page[0].surface_id, "other.surface");

        let software_page = registry.list_surfaces_for_tenant(tenant_a(), None, Some("software"));
        assert_eq!(software_page.len(), 1);
        assert_eq!(software_page[0].surface_id, "settings.in.id");
    }

    #[test]
    fn list_surfaces_page_filter_includes_host_detail_slot_on_hosts_page() {
        let registry = registry();
        let mut registration = registration_for_service("provider-a", tenant_a());
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

        let hosts_page = registry.list_surfaces_for_tenant(tenant_a(), None, Some("hosts"));
        assert_eq!(hosts_page.len(), 1);
        assert_eq!(hosts_page[0].surface_id, "host.detail.surface");

        let settings_page = registry.list_surfaces_for_tenant(tenant_a(), None, Some("settings"));
        assert!(settings_page.is_empty());
    }

    #[test]
    fn list_surfaces_page_filter_includes_software_item_tabs_slot_on_software_page() {
        let registry = registry();
        let mut registration = registration_for_service("provider-a", tenant_a());
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

        let software_page = registry.list_surfaces_for_tenant(tenant_a(), None, Some("software"));
        assert_eq!(software_page.len(), 1);
        assert_eq!(software_page[0].surface_id, "software.item.tabs.surface");

        let hosts_page = registry.list_surfaces_for_tenant(tenant_a(), None, Some("hosts"));
        assert!(hosts_page.is_empty());
    }

    #[test]
    fn shared_surface_resolution_uses_default_provider_order() {
        let registry = registry();
        registry.register_provider_for_test(
            registration_for_service("provider-a", tenant_a()),
            Some(Uuid::now_v7()),
            Some("uptrakit-agent-ssh"),
        );
        registry.register_provider_for_test(
            registration_for_plugin_same_surface("plugin-a"),
            None,
            None,
        );

        let read = registry
            .resolve_surface_read(tenant_a(), "ssh.guest.panel")
            .expect("read resolution should succeed");
        assert_eq!(
            read.descriptor.provider_kind,
            surfaces::ProviderKind::Plugin
        );

        let action =
            registry.resolve_surface_action(tenant_a(), "ssh.guest.panel", "refresh", None);
        assert!(matches!(
            action,
            Err(SurfaceRegistryLookupError::TargetProviderRequired)
        ));

        let action = registry
            .resolve_surface_action(tenant_a(), "ssh.guest.panel", "refresh", Some("provider-a"))
            .expect("explicit service target should resolve");
        assert_eq!(action.provider_id, "provider-a");
        assert_eq!(action.provider_kind, surfaces::ProviderKind::Service);
    }

    #[test]
    fn registration_rejects_data_source_pagination_above_1000() {
        let registry = registry();
        let mut registration = registration_for_service("provider-a", tenant_a());
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
