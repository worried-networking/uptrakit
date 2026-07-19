//! Single-source plugin surface + interaction registration types.
//!
//! Pairs a wire [`surfaces::InteractionDescriptor`] with the machinery that
//! actually executes it ([`InteractionDelivery`]) in one plugin-local struct
//! ([`RegisteredInteraction`]), so plugin authors declare an interaction
//! exactly once. The wire `InteractionTransport` is *derived* from the
//! delivery — never authored separately — eliminating the two-declaration
//! drift class this module replaces.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;

use uptrakit_surfaces as surfaces;

use crate::descriptor::{SurfaceActionContext, SurfaceActionError};

/// Per-interaction async handler.
///
/// Receives only `(ctx, params)`: the exact-id dispatch map built by
/// [`crate::catalog::PluginCatalog`] has already resolved the
/// `surface_id`/`action_id` before invoking the handler.
pub type InteractionHandler = for<'a> fn(
    &'a SurfaceActionContext<'a>,
    serde_json::Value,
) -> Pin<
    Box<dyn Future<Output = Result<serde_json::Value, SurfaceActionError>> + Send + 'a>,
>;

/// How an interaction is executed.
///
/// The wire [`surfaces::InteractionTransport`] is *derived* from this value —
/// never authored separately on the descriptor — via
/// [`RegisteredInteraction::new`]. A second author-settable transport would
/// recreate the two-declaration drift class this type eliminates.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum InteractionDelivery {
    /// Dispatched to the plugin's own handler (local-executor Tier 2/3; any
    /// controller-side audit wrap stays keyed in `local_executor`, untouched
    /// by this type).
    PluginHandled(InteractionHandler),
    /// Executed by controller-side code in local-executor Tier 1; the plugin
    /// declares the interaction but supplies no handler.
    ControllerExecutor,
}

impl InteractionDelivery {
    /// Returns the fieldless [`InteractionDeliveryKind`] mirror of this
    /// delivery.
    ///
    /// Matches exhaustively with no wildcard arm: adding a delivery variant
    /// without a matching `InteractionDeliveryKind` counterpart is a compile
    /// error here, rather than a silently-blind guard elsewhere (see
    /// `crate::catalog`'s `interaction_deliveries()` accessor).
    #[must_use]
    pub fn kind(&self) -> InteractionDeliveryKind {
        match self {
            InteractionDelivery::PluginHandled(_) => InteractionDeliveryKind::PluginHandled,
            InteractionDelivery::ControllerExecutor => InteractionDeliveryKind::ControllerExecutor,
        }
    }
}

/// Fieldless mirror of [`InteractionDelivery`].
///
/// The handler function pointer carried by [`InteractionDelivery::PluginHandled`]
/// is irrelevant to consumers that only need to know *how* an interaction is
/// delivered (e.g. the D5 executor-allowlist guard), so this kind-only copy
/// lets them iterate deliveries without exposing the fn pointer itself.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionDeliveryKind {
    /// Mirror of [`InteractionDelivery::PluginHandled`].
    PluginHandled,
    /// Mirror of [`InteractionDelivery::ControllerExecutor`].
    ControllerExecutor,
}

/// Pairs a wire [`surfaces::InteractionDescriptor`] with its
/// [`InteractionDelivery`] in one plugin-local, single-source declaration.
///
/// Fields are private: the only way to construct one is [`Self::new`], which
/// forces `descriptor.transport` to match `delivery` — a struct literal would
/// bypass that derivation and reintroduce an author-settable transport.
#[derive(Debug, Clone)]
pub struct RegisteredInteraction {
    descriptor: surfaces::InteractionDescriptor,
    delivery: InteractionDelivery,
}

impl RegisteredInteraction {
    /// Pairs `descriptor` with `delivery`, overwriting
    /// `descriptor.transport` to [`surfaces::InteractionTransport::ControllerLocal`]
    /// regardless of the delivery variant.
    ///
    /// Every catalog-registered interaction uses `ControllerLocal` transport
    /// today (verified: zero `ProviderProxied`-transport registrations in
    /// plugin crates — that transport comes exclusively from the
    /// service-registration path, served by its own type). One knob, one
    /// truth.
    #[must_use]
    pub fn new(
        mut descriptor: surfaces::InteractionDescriptor,
        delivery: InteractionDelivery,
    ) -> Self {
        descriptor.transport = surfaces::InteractionTransport::ControllerLocal;
        Self {
            descriptor,
            delivery,
        }
    }

    /// The wire interaction descriptor, with transport already normalized.
    #[must_use]
    pub fn descriptor(&self) -> &surfaces::InteractionDescriptor {
        &self.descriptor
    }

    /// How this interaction is executed.
    #[must_use]
    pub fn delivery(&self) -> &InteractionDelivery {
        &self.delivery
    }
}

/// A single surface plus its registered interactions and data sources,
/// declared by a plugin.
#[derive(Debug, Clone)]
pub struct PluginSurface {
    /// The surface's wire descriptor (slot, targeting, capabilities, node tree).
    pub descriptor: surfaces::SurfaceDescriptor,
    /// Interactions declared on this surface, each pairing a wire descriptor
    /// with its [`InteractionDelivery`].
    pub interactions: Vec<RegisteredInteraction>,
    /// Data sources backing this surface's nodes.
    pub data_sources: Vec<surfaces::DataSourceDescriptor>,
}

impl PluginSurface {
    /// Strips delivery information, producing the wire
    /// [`surfaces::RegisteredSurface`] shape.
    fn to_wire(&self) -> surfaces::RegisteredSurface {
        surfaces::RegisteredSurface {
            descriptor: self.descriptor.clone(),
            interactions: self
                .interactions
                .iter()
                .map(|interaction| interaction.descriptor().clone())
                .collect(),
            data_sources: self.data_sources.clone(),
        }
    }
}

/// A plugin's full set of registered surfaces — the single source consumed
/// by both the `PluginCatalog` (dispatch map, admission) and the wire
/// registration served to the controller.
#[derive(Debug, Clone, Default)]
pub struct PluginSurfaceRegistration {
    /// The surfaces this plugin registers under one provider identity.
    pub surfaces: Vec<PluginSurface>,
}

impl PluginSurfaceRegistration {
    /// Strips deliveries and derives the provider boilerplate
    /// (`framework_generation`, tenant binding, capabilities) that is
    /// identical across every plugin registration today, producing the wire
    /// [`surfaces::SurfaceRegistration`].
    ///
    /// `provider_id` is genuinely per-plugin (e.g. `"plugin.releases_docker"`)
    /// and is threaded through as a parameter rather than a hand-authored
    /// field on this type.
    #[must_use]
    pub fn to_wire(&self, provider_id: &str) -> surfaces::SurfaceRegistration {
        surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: provider_id.to_string(),
                provider_kind: surfaces::ProviderKind::Plugin,
                provider_namespace: "plugin".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: self.union_required_capabilities(),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: self.surfaces.iter().map(PluginSurface::to_wire).collect(),
            encryption_metadata: None,
        }
    }

    fn union_required_capabilities(&self) -> surfaces::CapabilitySet {
        let mut capabilities = BTreeSet::new();
        for surface in &self.surfaces {
            capabilities.extend(surface.descriptor.required_capabilities.0.iter().copied());
        }
        surfaces::CapabilitySet(capabilities)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_handler<'a>(
        _ctx: &'a crate::descriptor::SurfaceActionContext<'a>,
        _params: serde_json::Value,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<serde_json::Value, crate::descriptor::SurfaceActionError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async { Ok(serde_json::Value::Null) })
    }

    /// Builds a minimal but well-formed `SurfaceDescriptor`, mirroring the
    /// webhook plugin's builder chain (`notifications/webhook/src/plugin.rs`).
    fn minimal_surface_descriptor(
        surface_id: &str,
        capability: surfaces::Capability,
    ) -> surfaces::SurfaceDescriptor {
        surfaces::SurfaceDescriptor::builder()
            .surface_id(surfaces::SurfaceId::new(surface_id).expect("literal surface id is valid"))
            .label("Test Surface")
            .priority(100)
            .slot(surfaces::SLOT_SETTINGS_TABS)
            .scope(surfaces::Scope::Global)
            .targeting(surfaces::Targeting::Universal)
            .provider_kind(surfaces::ProviderKind::Plugin)
            .required_capabilities(surfaces::CapabilitySet::from_capabilities([capability]))
            .root_node(surfaces::SurfaceNode::section(None::<String>, vec![]))
            .build()
    }

    #[test]
    fn new_overwrites_transport_for_every_delivery_variant() {
        let deliveries = vec![
            InteractionDelivery::PluginHandled(test_handler),
            InteractionDelivery::ControllerExecutor,
        ];

        assert_eq!(
            deliveries.len(),
            2,
            "must cover every InteractionDelivery variant"
        );

        for delivery in deliveries {
            let descriptor = surfaces::InteractionDescriptor::new(
                surfaces::InteractionId::new("sample").expect("literal interaction id is valid"),
                surfaces::InteractionKind::MutationAction,
                "Sample",
                // Deliberately wrong initial transport — new() must overwrite it.
                surfaces::InteractionTransport::ProviderProxied,
            );

            let registered = RegisteredInteraction::new(descriptor, delivery);

            assert_eq!(
                registered.descriptor().transport,
                surfaces::InteractionTransport::ControllerLocal
            );
        }
    }

    #[test]
    fn kind_maps_each_delivery_variant() {
        assert_eq!(
            InteractionDelivery::PluginHandled(test_handler).kind(),
            InteractionDeliveryKind::PluginHandled
        );
        assert_eq!(
            InteractionDelivery::ControllerExecutor.kind(),
            InteractionDeliveryKind::ControllerExecutor
        );
    }

    #[test]
    fn to_wire_strips_deliveries_and_derives_provider_boilerplate() {
        let interaction_a = RegisteredInteraction::new(
            surfaces::InteractionDescriptor::new(
                surfaces::InteractionId::new("list").expect("literal interaction id is valid"),
                surfaces::InteractionKind::DataLoad,
                "List",
                surfaces::InteractionTransport::ControllerLocal,
            ),
            InteractionDelivery::PluginHandled(test_handler),
        );
        let interaction_b = RegisteredInteraction::new(
            surfaces::InteractionDescriptor::new(
                surfaces::InteractionId::new("create").expect("literal interaction id is valid"),
                surfaces::InteractionKind::FormSubmit,
                "Create",
                surfaces::InteractionTransport::ControllerLocal,
            ),
            InteractionDelivery::ControllerExecutor,
        );

        let surface_one = PluginSurface {
            descriptor: minimal_surface_descriptor(
                "plugin.sample.one",
                surfaces::Capability::TableNode,
            ),
            interactions: vec![interaction_a, interaction_b],
            data_sources: vec![],
        };
        let surface_two = PluginSurface {
            descriptor: minimal_surface_descriptor(
                "plugin.sample.two",
                surfaces::Capability::FormNode,
            ),
            interactions: vec![],
            data_sources: vec![],
        };

        let registration = PluginSurfaceRegistration {
            surfaces: vec![surface_one, surface_two],
        };

        let wire = registration.to_wire("plugin.sample");

        assert_eq!(wire.provider.provider_id, "plugin.sample");
        assert_eq!(wire.provider.provider_kind, surfaces::ProviderKind::Plugin);
        assert_eq!(wire.provider.provider_namespace, "plugin");
        assert_eq!(
            wire.framework_generation,
            surfaces::FrameworkGeneration::new(1, 0)
        );
        assert_eq!(wire.effective_tenant_binding.scope, surfaces::Scope::Global);
        assert_eq!(wire.effective_tenant_binding.tenant_id, None);
        assert_eq!(wire.encryption_metadata, None);

        // Union of both surfaces' required_capabilities — not a straight copy of one.
        assert_eq!(
            wire.capabilities,
            surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TableNode,
                surfaces::Capability::FormNode,
            ])
        );

        assert_eq!(wire.surfaces.len(), 2, "must not silently drop surfaces");
        let first = wire.surfaces.first().expect("first surface present");
        assert_eq!(
            first.interactions.len(),
            2,
            "must not silently drop interactions"
        );
        assert_eq!(first.interactions[0].interaction_id.as_str(), "list");
        assert_eq!(first.interactions[1].interaction_id.as_str(), "create");
        // The wire type has no delivery field at all — stripping is structural.
        assert_eq!(
            first.interactions[0].transport,
            surfaces::InteractionTransport::ControllerLocal
        );
    }
}
