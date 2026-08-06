//! Live [`DynamicActionRegistry`] over the surface registry (M1.5, spec §8).

use std::sync::Arc;

use uptrakit_controller_core::access::DynamicActionRegistry;
use uptrakit_shared_types::access::{Action, Verb};

use crate::surface_registry::SurfaceRegistry;

/// `surface.<id>:use` is registered iff a surface with that id is currently
/// registered; every other dynamic action (other verbs on `surface.*`,
/// all of `plugin.*`) is unregistered and therefore denies — fail closed,
/// no dangling authority (spec §8; `05` §Dynamic namespaces).
///
/// Tenant- and visibility-blind by trait shape: a surface registered by any
/// provider makes the action decidable instance-wide, skipping the tenant
/// binding and provider-visibility filters the listing path applies
/// (accepted v1 residual — grants stay tenant-scoped).
pub struct SurfaceActionRegistry(pub Arc<SurfaceRegistry>);

impl DynamicActionRegistry for SurfaceActionRegistry {
    fn is_registered(&self, action: &Action) -> bool {
        if action.verb() != Verb::Use {
            return false;
        }
        match action.resource().surface_id() {
            Some(surface_id) => self.0.has_surface(surface_id),
            None => false,
        }
    }

    fn registered_actions(&self) -> Vec<Action> {
        // Grammar mismatch is real: `validate_surface_identifier` admits ids
        // (underscores, …) the Action resource grammar rejects. Build through
        // the same parse the read side uses and skip non-parsing ids — such a
        // surface's `:use` action is unparseable everywhere, so
        // `is_registered` can never return true for it either (fail-closed,
        // iff-contract preserved). Never normalize: a normalized id would
        // advertise an action the engine denies.
        self.0
            .surface_ids()
            .into_iter()
            .filter_map(|surface_id| format!("surface.{surface_id}:use").parse::<Action>().ok())
            .collect()
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test fixture: panics on setup failure are acceptable"
    )]

    use std::sync::Arc;

    use uuid::Uuid;

    use uptrakit_controller_core::access::{AccessEngine, DynamicActionRegistry};
    use uptrakit_shared_db::access_grants::{GrantSubject, NewGrant, insert_grant};
    use uptrakit_shared_types::access::{Action, ActionPattern, Decision, DenyReason, Selector};
    use uptrakit_wire::surfaces;

    use super::SurfaceActionRegistry;
    use crate::surface_registry::{SurfaceRegistry, SurfaceRegistryConfig};
    use crate::test_harness::fixtures::default_tenant_id;
    use crate::test_harness::setup_migrated_db;

    fn registration_for_test_stub(
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
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Tenant,
                tenant_id: Some(tenant_id.to_string()),
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("test.stub").expect("valid surface id"))
                    .label("Test Stub")
                    .priority(100)
                    .slot("software.tabs")
                    .scope(surfaces::Scope::Tenant)
                    .targeting(surfaces::Targeting::Targeted)
                    .required_action(
                        "surface.test.stub:use"
                            .parse::<Action>()
                            .expect("valid action"),
                    )
                    .provider_kind(surfaces::ProviderKind::Service)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::TargetedTargeting,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    })
                    .build(),
                interactions: vec![],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        }
    }

    #[tokio::test]
    async fn surface_action_registry_flips_across_service_register_and_unregister() {
        let db = setup_migrated_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let user_id = Uuid::now_v7();
        let registry = Arc::new(SurfaceRegistry::new(SurfaceRegistryConfig::default()));
        let registry_impl = SurfaceActionRegistry(Arc::clone(&registry));
        let engine = AccessEngine::new(db.clone())
            .with_registry(Arc::new(SurfaceActionRegistry(Arc::clone(&registry))));

        let patterns = vec![
            "surface.test.stub:use"
                .parse::<ActionPattern>()
                .expect("valid pattern"),
        ];
        insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(user_id),
                tenant_id: Some(tenant_id),
                patterns: &patterns,
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("insert grant");

        let stub_action = "surface.test.stub:use"
            .parse::<Action>()
            .expect("registered surface id must round-trip through the action grammar");

        // Grants never change across this test — one context covers every
        // `authorize()` call; only the registry's registration state moves.
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("context");

        // Before registration: the dynamic action is unregistered — fail
        // closed even though the grant already exists.
        assert_eq!(
            engine.authorize(&ctx, &stub_action, None),
            Decision::Deny(DenyReason::UnknownAction)
        );

        // After a service registers the surface: the same grant now allows.
        let service_id = Uuid::now_v7();
        registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id),
                registration_for_test_stub("service.test-stub", tenant_id),
            )
            .expect("valid registration must admit");
        assert_eq!(engine.authorize(&ctx, &stub_action, None), Decision::Allow);

        // Enumeration must agree with `is_registered` while registered.
        let enumerated = engine.dynamic_actions();
        assert_eq!(enumerated, vec![stub_action.clone()]);
        for action in &enumerated {
            assert!(
                registry_impl.is_registered(action),
                "every enumerated action must independently pass is_registered"
            );
        }

        // Everything-else-false leg: an unregistered `plugin.*` action denies
        // even with the registry wired.
        let plugin_action = "plugin.anything:use"
            .parse::<Action>()
            .expect("valid dynamic action");
        assert_eq!(
            engine.authorize(&ctx, &plugin_action, None),
            Decision::Deny(DenyReason::UnknownAction)
        );

        // A non-`use` verb on `surface.*` denies even though `test.stub` is
        // currently registered — the registry only ever decides `:use`.
        let read_action = "surface.test.stub:read"
            .parse::<Action>()
            .expect("valid dynamic action");
        assert_eq!(
            engine.authorize(&ctx, &read_action, None),
            Decision::Deny(DenyReason::UnknownAction)
        );

        // After the registering service unregisters: denies again.
        registry.unregister_service(&service_id);
        assert_eq!(
            engine.authorize(&ctx, &stub_action, None),
            Decision::Deny(DenyReason::UnknownAction)
        );
        assert!(
            engine.dynamic_actions().is_empty(),
            "enumeration must be empty once the surface is unregistered"
        );
    }

    /// Grammar mismatch (plan-review-resolved branch): `validate_surface_identifier`
    /// accepts `_` in a surface id while the Action resource grammar's
    /// `is_valid_segment` rejects it. Registration succeeds (surface-id
    /// admission is the surfaces crate's own grammar), but enumeration's
    /// `surface.<id>:use` parse fails and the id is silently dropped —
    /// fail-closed, never normalized.
    #[tokio::test]
    async fn underscore_surface_id_is_dropped_from_enumeration_fail_closed() {
        let db = setup_migrated_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let registry = Arc::new(SurfaceRegistry::new(SurfaceRegistryConfig::default()));
        let engine = AccessEngine::new(db.clone())
            .with_registry(Arc::new(SurfaceActionRegistry(Arc::clone(&registry))));

        let mut registration =
            registration_for_test_stub("service.test-stub-underscore", tenant_id);
        registration.surfaces[0].descriptor.surface_id =
            surfaces::SurfaceId::new("test.stub_underscore").expect("valid surface id");

        let service_id = Uuid::now_v7();
        registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id),
                registration,
            )
            .expect("underscore id must pass surface admission — precondition");

        assert!(
            registry.has_surface("test.stub_underscore"),
            "the surface IS registered"
        );

        let enumerated = engine.dynamic_actions();
        assert!(
            enumerated
                .iter()
                .all(|action| !action.to_string().contains("stub_underscore")),
            "the enumeration parse-skip must drop the unparseable surface id; its `:use` action \
             is unparseable everywhere, so is_registered can never be probed with it and the iff \
             contract holds vacuously"
        );
    }
}
