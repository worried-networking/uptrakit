//! `declare_plugin!` macro — generates `PluginDescriptor` + `PluginMeta` + config
//! delegation functions from a concise declaration.
//!
//! # Usage
//!
//! ```rust,ignore
//! declare_plugin!(AptPlugin, AptConfig, "package_manager_apt", {
//!     display_name: "APT Package Manager",
//!     family: PluginFamily::Software,
//!     config_model: ConfigModel::PluginConfig,
//!     host_requirements: HostRequirements::POSIX,
//!     roles: [Discoverer, VersionDetector, ReleaseFetcher,
//!             PackageIndexer { host_requirements: HostRequirements::POSIX_PRIVILEGED },
//!             UpdateExecutor { host_requirements: HostRequirements::POSIX_PRIVILEGED }],
//! });
//! ```
//!
//! # What it generates
//!
//! 1. `impl PluginMeta for $plugin` — returns `PluginTypeId::from_static($type_id)`
//! 2. Private `mod __descriptor_impl` with config ops and per-role creation functions
//! 3. `pub static DESCRIPTOR: PluginDescriptor` — assembled from generated parts
//! 4. Compile-time trait assertions — plugin struct must implement all declared roles

/// Declare a plugin: generates `PluginMeta`, config operations, creation functions,
/// and a `pub static DESCRIPTOR: PluginDescriptor`.
///
/// See [module-level docs](self) for full usage.
#[macro_export]
macro_rules! declare_plugin {
    (
        $plugin:ty, $config:ty, $type_id:expr, {
            display_name: $display_name:expr,
            family: $family:expr,
            config_model: $config_model:expr,
            $( host_requirements: $default_hr:expr, )?
            $( config_test: [ $($ct_kind:expr),+ $(,)? ], )?
            $( type_settings: $ts_marker:tt, )?
            roles: [ $( $role:ident $( { host_requirements: $role_hr:expr } )? ),* $(,)? ]
            $(, extra_capabilities: [ $( $extra_cap:expr ),+ $(,)? ] )?
            $(, notification_transport: $transport_fn:expr )?
            $(, software_item_lifecycle: $lifecycle_fn:expr )?
            $(, controller_update_protection: $controller_protection_fn:expr )?
            $(, infra: {
                create: $infra_create_fn:expr,
                host_requirements: $infra_hr:expr,
                capabilities: $infra_caps:expr $(,)?
            } )?
            $(, owned_surface_ids: $surface_action_ids:expr )?
            $(, raw_settings_keys: $raw_keys:expr )?
            $(, global_provider_consumers: [ $( $global_provider_consumer:expr ),+ $(,)? ] )?
            $(, sudo: $sudo_fn:expr )?
            $(, surface_actions: {
                actions: $surface_actions_fn:expr,
                handle_action: $surface_handler_fn:expr $(,)?
            } )?
            $(, surfaces: {
                registrations: $surface_registrations_fn:expr $(,)?
            } )?
            $(, migrations: $migrations_fn:expr )?
            $(,)?
        }
    ) => {
        // ── 1. PluginMeta impl ──────────────────────────────────────────
        impl $crate::PluginMeta for $plugin {
            fn plugin_type_id(&self) -> $crate::PluginTypeId {
                $crate::PluginTypeId::from_static($type_id)
            }
        }

        // ── 2. Compile-time role trait assertions ───────────────────────
        $(
            $crate::__assert_role_impl!($plugin, $role);
        )*

        // ── 3. Private module with generated functions ──────────────────
        #[doc(hidden)]
        #[expect(
            clippy::allow_attributes,
            clippy::allow_attributes_without_reason,
            reason = "feature-conditional: macro-generated module — not all config functions are called by every plugin invocation; #[expect] would fail when all are referenced"
        )]
        #[allow(unused_imports, dead_code)]
        mod __descriptor_impl {
            use super::*;

            // Config operations — delegate to PluginConfig trait methods
            pub(super) fn validate(
                config: &serde_json::Value,
            ) -> std::result::Result<(), $crate::PluginConfigValidationError> {
                let typed: $config = serde_json::from_value(config.clone())
                    .map_err(|e| $crate::PluginConfigValidationError::Contract(format!(
                        "failed to parse config: {e}"
                    )))?;
                <$config as $crate::PluginConfig>::validate(&typed)
            }

            pub(super) fn mask_secrets(
                config: &serde_json::Value,
            ) -> serde_json::Value {
                let Ok(cfg) = serde_json::from_value::<$config>(config.clone()) else {
                    return config.clone();
                };
                let masked = <$config as $crate::PluginConfig>::with_secrets_masked(cfg);
                serde_json::to_value(masked).unwrap_or_else(|e| {
                    tracing::error!(error = %e, "failed to serialize masked plugin config");
                    config.clone()
                })
            }

            pub(super) fn restore_secrets(
                incoming: &mut serde_json::Value,
                stored: &serde_json::Value,
            ) {
                let (Ok(mut inc), Ok(ex)) = (
                    serde_json::from_value::<$config>(incoming.clone()),
                    serde_json::from_value::<$config>(stored.clone()),
                ) else {
                    return;
                };
                <$config as $crate::PluginConfig>::restore_secrets_from(&mut inc, &ex);
                if let Ok(v) = serde_json::to_value(&inc) {
                    *incoming = v;
                }
            }

            pub(super) fn sample() -> serde_json::Value {
                serde_json::to_value(<$config as Default>::default())
                    .unwrap_or_else(|_| serde_json::json!({}))
            }

            pub(super) fn form_schema() -> Vec<$crate::form_schema::FormFieldDescriptor> {
                <$config as $crate::PluginConfig>::form_schema()
            }

            pub(super) fn validate_identifier(
                value: &str,
            ) -> std::result::Result<(), $crate::PluginConfigValidationError> {
                <$config as $crate::PluginConfig>::validate_identifier(value)
            }

            // Per-role creation functions
            $(
                $crate::__define_role_creator!($plugin, $config, $role);
            )*
        }

        // ── 4. Optional static sections (inlined to avoid repetition-driver issues) ──

        // Config test static — $ct_kind drives this repetition
        $(
            $crate::__declare_config_test_static!( $($ct_kind),+ );
        )?

        // Type settings static — $ts_marker (captured as `true`) drives this repetition
        $(
            $crate::__declare_type_settings_static!($config, $ts_marker);
        )?

        // Surface action library static — $surface_action_ids drives this repetition
        $(
            $crate::__declare_surface_action_library_static!(
                $surface_action_ids, $surface_actions_fn, $surface_handler_fn
            );
        )?

        // Surface ops static — $surface_registrations_fn drives this repetition
        $(
            $crate::__declare_surface_ops_static!($surface_registrations_fn);
        )?

        // ── 5. Static descriptor ────────────────────────────────────────
        pub static DESCRIPTOR: $crate::PluginDescriptor = $crate::PluginDescriptor {
            type_id: $type_id,
            display_name: $display_name,
            family: $family,
            config_model: $config_model,
            capabilities: $crate::__compute_capabilities!(
                [ $( $role ),* ]
                $(, extra: [ $($extra_cap),+ ] )?
                $(, config_test: [ $($ct_kind),+ ] )?
            ),
            config: $crate::ConfigOps {
                validate: __descriptor_impl::validate,
                mask_secrets: __descriptor_impl::mask_secrets,
                restore_secrets: __descriptor_impl::restore_secrets,
                sample: __descriptor_impl::sample,
                form_schema: __descriptor_impl::form_schema,
                validate_identifier: __descriptor_impl::validate_identifier,
            },
            roles: {
                #[expect(
                    clippy::allow_attributes,
                    clippy::allow_attributes_without_reason,
                    reason = "feature-conditional: only referenced when role entries with per-role host_requirements are specified; #[expect] would fail when at least one such role is present"
                )]
                #[allow(unused)]
                const __DEFAULT_HR: $crate::HostRequirements =
                    $crate::__default_hr_value!( $( $default_hr )? );

                #[expect(
                    clippy::allow_attributes,
                    clippy::allow_attributes_without_reason,
                    reason = "feature-conditional: `mut` is required when role entries are specified; without roles this binding is never mutated and #[expect] would fail in that variant"
                )]
                #[allow(unused_mut)]
                let mut rc = $crate::RoleCreators {
                    discoverer: None,
                    version_detector: None,
                    release_fetcher: None,
                    package_indexer: None,
                    update_executor: None,
                    lifecycle_hook: None,
                    notification_transport: None,
                    software_item_lifecycle: None,
                    controller_update_protection: None,
                    infra: None,
                };
                $(
                    $crate::__set_role_field!(rc, $role,
                        $crate::__role_or_default_hr!(__DEFAULT_HR $(, $role_hr )? )
                    );
                )*
                $(
                    rc.notification_transport = Some($transport_fn);
                )?
                $(
                    rc.software_item_lifecycle = Some($lifecycle_fn);
                )?
                $(
                    rc.controller_update_protection = Some($controller_protection_fn);
                )?
                $(
                    #[cfg(feature = "agent-infra")]
                    {
                        rc.infra = Some($crate::InfraSlot {
                            create: $infra_create_fn,
                            host_requirements: $infra_hr,
                            capabilities: $infra_caps,
                        });
                    }
                    // Silence unused variable warnings when agent-infra is disabled.
                    #[cfg(not(feature = "agent-infra"))]
                    {
                        let _ = stringify!($infra_create_fn);
                    }
                )?
                rc
            },
            surface_actions: $crate::__optional_static_ref!(surface_actions
                $(, surface_actions: { owned_surface_ids: $surface_action_ids } )?
            ),
            surfaces: $crate::__optional_static_ref!(surfaces
                $(, surfaces: { registrations: $surface_registrations_fn } )?
            ),
            type_settings: $crate::__optional_static_ref!(type_settings
                $(, type_settings: $ts_marker )?
            ),
            config_test: $crate::__optional_static_ref!(config_test
                $(, config_test: [ $($ct_kind),+ ] )?
            ),
            sudo: $crate::__option_expr!( $( $sudo_fn )? ),
            raw_settings_keys: $crate::__or_empty_slice!( $( $raw_keys )? ),
            global_provider_consumers: $crate::__or_empty_slice!(
                $( &[ $( $crate::GlobalProviderConsumerDecl::new($global_provider_consumer) ),+ ] )?
            ),
            migrations: $crate::__option_expr!( $( $migrations_fn )? ),
        };
    };
}

// ═══════════════════════════════════════════════════════════════════════════
// Helper macros
// ═══════════════════════════════════════════════════════════════════════════

/// Assert that `$plugin` implements the role trait corresponding to `$role`.
#[macro_export]
#[doc(hidden)]
macro_rules! __assert_role_impl {
    ($plugin:ty, Discoverer) => {
        const _: () = {
            fn _assert<T: $crate::Discoverer>() {}
            fn _check() {
                _assert::<$plugin>();
            }
        };
    };
    ($plugin:ty, VersionDetector) => {
        const _: () = {
            fn _assert<T: $crate::VersionDetector>() {}
            fn _check() {
                _assert::<$plugin>();
            }
        };
    };
    ($plugin:ty, ReleaseFetcher) => {
        const _: () = {
            fn _assert<T: $crate::ReleaseFetcher>() {}
            fn _check() {
                _assert::<$plugin>();
            }
        };
    };
    ($plugin:ty, PackageIndexer) => {
        const _: () = {
            fn _assert<T: $crate::PackageIndexer>() {}
            fn _check() {
                _assert::<$plugin>();
            }
        };
    };
    ($plugin:ty, UpdateExecutor) => {
        const _: () = {
            fn _assert<T: $crate::UpdateExecutor>() {}
            fn _check() {
                _assert::<$plugin>();
            }
        };
    };
    ($plugin:ty, LifecycleHook) => {
        const _: () = {
            fn _assert<T: $crate::LifecycleHook>() {}
            fn _check() {
                _assert::<$plugin>();
            }
        };
    };
    ($plugin:ty, NotificationTransport) => {
        const _: () = {
            fn _assert<T: $crate::NotificationTransport>() {}
            fn _check() {
                _assert::<$plugin>();
            }
        };
    };
    ($plugin:ty, SoftwareItemLifecycle) => {
        const _: () = {
            fn _assert<T: $crate::SoftwareItemLifecycle>() {}
            fn _check() {
                _assert::<$plugin>();
            }
        };
    };
}

/// Generate a role creation function inside `__descriptor_impl`.
///
/// Per-instance roles get a function that deserializes config and constructs the plugin.
/// Singleton roles (NotificationTransport, SoftwareItemLifecycle) are no-ops here —
/// their creation functions are provided separately.
#[macro_export]
#[doc(hidden)]
macro_rules! __define_role_creator {
    ($plugin:ty, $config:ty, Discoverer) => {
        pub(super) fn create_discoverer(
            config: &serde_json::Value,
            runtime: std::sync::Arc<dyn $crate::HostRuntime>,
        ) -> $crate::error::Result<Box<dyn $crate::Discoverer>> {
            let cfg: $config = serde_json::from_value(config.clone()).map_err(|e| {
                rootcause::report!($crate::PluginError::Configuration(format!(
                    "failed to parse config: {e}"
                )))
            })?;
            let plugin = <$plugin>::new(cfg, runtime).map_err(|e| {
                rootcause::report!($crate::PluginError::Configuration(format!(
                    "plugin construction failed: {e}"
                )))
            })?;
            Ok(Box::new(plugin))
        }
    };
    ($plugin:ty, $config:ty, VersionDetector) => {
        pub(super) fn create_version_detector(
            config: &serde_json::Value,
            runtime: std::sync::Arc<dyn $crate::HostRuntime>,
        ) -> $crate::error::Result<Box<dyn $crate::VersionDetector>> {
            let cfg: $config = serde_json::from_value(config.clone()).map_err(|e| {
                rootcause::report!($crate::PluginError::Configuration(format!(
                    "failed to parse config: {e}"
                )))
            })?;
            let plugin = <$plugin>::new(cfg, runtime).map_err(|e| {
                rootcause::report!($crate::PluginError::Configuration(format!(
                    "plugin construction failed: {e}"
                )))
            })?;
            Ok(Box::new(plugin))
        }
    };
    ($plugin:ty, $config:ty, ReleaseFetcher) => {
        pub(super) fn create_release_fetcher(
            config: &serde_json::Value,
            runtime: std::sync::Arc<dyn $crate::HostRuntime>,
        ) -> $crate::error::Result<Box<dyn $crate::ReleaseFetcher>> {
            let cfg: $config = serde_json::from_value(config.clone()).map_err(|e| {
                rootcause::report!($crate::PluginError::Configuration(format!(
                    "failed to parse config: {e}"
                )))
            })?;
            let plugin = <$plugin>::new(cfg, runtime).map_err(|e| {
                rootcause::report!($crate::PluginError::Configuration(format!(
                    "plugin construction failed: {e}"
                )))
            })?;
            Ok(Box::new(plugin))
        }
    };
    ($plugin:ty, $config:ty, PackageIndexer) => {
        pub(super) fn create_package_indexer(
            config: &serde_json::Value,
            runtime: std::sync::Arc<dyn $crate::HostRuntime>,
        ) -> $crate::error::Result<Box<dyn $crate::PackageIndexer>> {
            let cfg: $config = serde_json::from_value(config.clone()).map_err(|e| {
                rootcause::report!($crate::PluginError::Configuration(format!(
                    "failed to parse config: {e}"
                )))
            })?;
            let plugin = <$plugin>::new(cfg, runtime).map_err(|e| {
                rootcause::report!($crate::PluginError::Configuration(format!(
                    "plugin construction failed: {e}"
                )))
            })?;
            Ok(Box::new(plugin))
        }
    };
    ($plugin:ty, $config:ty, UpdateExecutor) => {
        pub(super) fn create_update_executor(
            config: &serde_json::Value,
            runtime: std::sync::Arc<dyn $crate::HostRuntime>,
        ) -> $crate::error::Result<Box<dyn $crate::UpdateExecutor>> {
            let cfg: $config = serde_json::from_value(config.clone()).map_err(|e| {
                rootcause::report!($crate::PluginError::Configuration(format!(
                    "failed to parse config: {e}"
                )))
            })?;
            let plugin = <$plugin>::new(cfg, runtime).map_err(|e| {
                rootcause::report!($crate::PluginError::Configuration(format!(
                    "plugin construction failed: {e}"
                )))
            })?;
            Ok(Box::new(plugin))
        }
    };
    ($plugin:ty, $config:ty, LifecycleHook) => {
        pub(super) fn create_lifecycle_hook(
            config: &serde_json::Value,
            runtime: std::sync::Arc<dyn $crate::HostRuntime>,
        ) -> $crate::error::Result<Box<dyn $crate::LifecycleHook>> {
            let cfg: $config = serde_json::from_value(config.clone()).map_err(|e| {
                rootcause::report!($crate::PluginError::Configuration(format!(
                    "failed to parse config: {e}"
                )))
            })?;
            let plugin = <$plugin>::new(cfg, runtime).map_err(|e| {
                rootcause::report!($crate::PluginError::Configuration(format!(
                    "plugin construction failed: {e}"
                )))
            })?;
            Ok(Box::new(plugin))
        }
    };
    // Singleton roles — no creation function generated here
    ($plugin:ty, $config:ty, NotificationTransport) => {};
    ($plugin:ty, $config:ty, SoftwareItemLifecycle) => {};
}

/// Set a role field on a mutable `RoleCreators` instance.
///
/// Per-instance roles set their field to `Some(RoleSlot { ... })`.
/// Singleton roles are no-ops (they are set separately on the descriptor).
#[macro_export]
#[doc(hidden)]
macro_rules! __set_role_field {
    ($rc:ident, Discoverer, $hr:expr) => {
        $rc.discoverer = Some($crate::RoleSlot {
            create: __descriptor_impl::create_discoverer,
            host_requirements: $hr,
        });
    };
    ($rc:ident, VersionDetector, $hr:expr) => {
        $rc.version_detector = Some($crate::RoleSlot {
            create: __descriptor_impl::create_version_detector,
            host_requirements: $hr,
        });
    };
    ($rc:ident, ReleaseFetcher, $hr:expr) => {
        $rc.release_fetcher = Some($crate::RoleSlot {
            create: __descriptor_impl::create_release_fetcher,
            host_requirements: $hr,
        });
    };
    ($rc:ident, PackageIndexer, $hr:expr) => {
        $rc.package_indexer = Some($crate::RoleSlot {
            create: __descriptor_impl::create_package_indexer,
            host_requirements: $hr,
        });
    };
    ($rc:ident, UpdateExecutor, $hr:expr) => {
        $rc.update_executor = Some($crate::RoleSlot {
            create: __descriptor_impl::create_update_executor,
            host_requirements: $hr,
        });
    };
    ($rc:ident, LifecycleHook, $hr:expr) => {
        $rc.lifecycle_hook = Some($crate::RoleSlot {
            create: __descriptor_impl::create_lifecycle_hook,
            host_requirements: $hr,
        });
    };
    // Singleton roles — handled separately via dedicated macro keys.
    ($rc:ident, NotificationTransport, $hr:expr) => {};
    ($rc:ident, SoftwareItemLifecycle, $hr:expr) => {};
}

/// Resolve the descriptor-level default host requirements.
/// Expanded ONCE outside the per-role loop to avoid repetition-driver conflicts.
#[macro_export]
#[doc(hidden)]
macro_rules! __default_hr_value {
    ($hr:expr) => {
        $hr
    };
    () => {
        $crate::HostRequirements::CONTROLLER_ONLY
    };
}

/// Pick per-role host requirements: role override > descriptor default.
/// `$default` is a const ident resolved outside the role loop.
/// `$role_hr` is the per-role override (present only if specified).
#[macro_export]
#[doc(hidden)]
macro_rules! __role_or_default_hr {
    ($default:ident, $role_hr:expr) => {
        $role_hr
    };
    ($default:ident) => {
        $default
    };
}

/// Compute the capabilities array from declared roles + optional config_test.
///
/// Uses `__accumulate_role_caps!` to flatten multi-capability roles (e.g., Discoverer
/// expands to both `DiscoverLocalSoftware` and `DetectHostCompatibility`) into a single
/// const slice. Direct nesting of `__expand_role_caps!` in array position would fail
/// because macro invocations in expression context must produce a single expression.
#[macro_export]
#[doc(hidden)]
macro_rules! __compute_capabilities {
    ( [ $( $role:ident ),* ] , extra: [ $($extra_cap:expr),+ ] $(, config_test: [ $($ct_kind:expr),+ ] )? ) => {{
        const CAPS: &[$crate::PluginCapability] = &$crate::__accumulate_role_caps!(
            []
            $(, $role )*
            ; extra: $($extra_cap),+
            $(; config_test: $($ct_kind),+ )?
        );
        CAPS
    }};
    ( [ $( $role:ident ),* ] $(, config_test: [ $($ct_kind:expr),+ ] )? ) => {{
        const CAPS: &[$crate::PluginCapability] = &$crate::__accumulate_role_caps!(
            []
            $(, $role )*
            $(; config_test: $($ct_kind),+ )?
        );
        CAPS
    }};
}

/// Recursive TT muncher that accumulates `PluginCapability` items from role identifiers.
///
/// Each role arm prepends its capabilities to the accumulator and recurses.
/// The base case emits the final `[...]` array body.
#[macro_export]
#[doc(hidden)]
macro_rules! __accumulate_role_caps {
    // ── Base cases (no more roles) ──────────────────────────────────────
    // extra + config_test
    ( [ $($acc:expr),* ] ; extra: $($extra:expr),+ ; config_test: $($ct_kind:expr),+ ) => {
        [ $($acc,)* $($extra,)+ $crate::PluginCapability::ConfigTest ]
    };
    // extra only
    ( [ $($acc:expr),* ] ; extra: $($extra:expr),+ ) => {
        [ $($acc,)* $($extra),+ ]
    };
    // config_test only
    ( [ $($acc:expr),* ] ; config_test: $($ct_kind:expr),+ ) => {
        [ $($acc,)* $crate::PluginCapability::ConfigTest ]
    };
    // nothing
    ( [ $($acc:expr),* ] ) => {
        [ $($acc),* ]
    };
    // ── Role arms ───────────────────────────────────────────────────────
    ( [ $($acc:expr),* ], Discoverer $($rest:tt)* ) => {
        $crate::__accumulate_role_caps!(
            [ $($acc,)*
              $crate::PluginCapability::DiscoverLocalSoftware,
              $crate::PluginCapability::DetectHostCompatibility ]
            $($rest)*
        )
    };
    ( [ $($acc:expr),* ], VersionDetector $($rest:tt)* ) => {
        $crate::__accumulate_role_caps!(
            [ $($acc,)* $crate::PluginCapability::VersionDetection ]
            $($rest)*
        )
    };
    ( [ $($acc:expr),* ], ReleaseFetcher $($rest:tt)* ) => {
        $crate::__accumulate_role_caps!(
            [ $($acc,)* $crate::PluginCapability::ReleaseFetching ]
            $($rest)*
        )
    };
    ( [ $($acc:expr),* ], PackageIndexer $($rest:tt)* ) => {
        $crate::__accumulate_role_caps!(
            [ $($acc,)* $crate::PluginCapability::RefreshPackageIndex ]
            $($rest)*
        )
    };
    ( [ $($acc:expr),* ], UpdateExecutor $($rest:tt)* ) => {
        $crate::__accumulate_role_caps!(
            [ $($acc,)* $crate::PluginCapability::UpdateExecution ]
            $($rest)*
        )
    };
    ( [ $($acc:expr),* ], LifecycleHook $($rest:tt)* ) => {
        $crate::__accumulate_role_caps!(
            [ $($acc,)* $crate::PluginCapability::UpdateLifecycle ]
            $($rest)*
        )
    };
    ( [ $($acc:expr),* ], NotificationTransport $($rest:tt)* ) => {
        $crate::__accumulate_role_caps!(
            [ $($acc,)* $crate::PluginCapability::NotificationDelivery ]
            $($rest)*
        )
    };
    ( [ $($acc:expr),* ], SoftwareItemLifecycle $($rest:tt)* ) => {
        $crate::__accumulate_role_caps!(
            [ $($acc,)* $crate::PluginCapability::SoftwareItemLifecycle ]
            $($rest)*
        )
    };
}

/// Expand a single role to its capabilities (used inside a const array literal).
#[macro_export]
#[doc(hidden)]
macro_rules! __expand_role_caps {
    (Discoverer) => {
        $crate::PluginCapability::DiscoverLocalSoftware,
        $crate::PluginCapability::DetectHostCompatibility
    };
    (VersionDetector) => {
        $crate::PluginCapability::VersionDetection
    };
    (ReleaseFetcher) => {
        $crate::PluginCapability::ReleaseFetching
    };
    (PackageIndexer) => {
        $crate::PluginCapability::RefreshPackageIndex
    };
    (UpdateExecutor) => {
        $crate::PluginCapability::UpdateExecution
    };
    (LifecycleHook) => {
        $crate::PluginCapability::UpdateLifecycle
    };
    (NotificationTransport) => {
        $crate::PluginCapability::NotificationDelivery
    };
    (SoftwareItemLifecycle) => {
        $crate::PluginCapability::SoftwareItemLifecycle
    };
}

/// Emit `ConfigTest` capability. The argument is just used for presence detection.
#[macro_export]
#[doc(hidden)]
macro_rules! __config_test_cap_marker {
    ( $($kind:expr),+ ) => {
        $crate::PluginCapability::ConfigTest
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __declare_config_test_static {
    ( $first_kind:expr $(, $rest_kind:expr )* ) => {
        #[doc(hidden)]
        static __PLUGIN_CONFIG_TEST: $crate::ConfigTestOps = $crate::ConfigTestOps {
            supported_kinds: &[ $first_kind $(, $rest_kind )* ],
            default_kind: $first_kind,
        };
    };
}

/// Declare the type-settings static. The `$_marker` is the captured `true` token
/// — used only to drive the optional repetition in `declare_plugin!`.
#[macro_export]
#[doc(hidden)]
macro_rules! __declare_type_settings_static {
    ($config:ty, $_marker:tt) => {
        #[doc(hidden)]
        static __PLUGIN_TYPE_SETTINGS: $crate::TypeSettingsOps = $crate::TypeSettingsOps {
            form_schema: <$config as $crate::TypeSettings>::type_settings_form_schema,
            sample: <$config as $crate::TypeSettings>::type_settings_sample,
        };
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __declare_surface_action_library_static {
    ($surface_action_ids:expr, $actions_fn:expr, $handler_fn:expr) => {
        #[doc(hidden)]
        static __PLUGIN_SURFACE_ACTIONS: $crate::SurfaceActionLibrary =
            $crate::SurfaceActionLibrary {
                actions: $actions_fn,
                owned_surface_ids: $surface_action_ids,
                handle_action: $handler_fn,
            };
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __declare_surface_ops_static {
    ($registrations_fn:expr) => {
        #[doc(hidden)]
        static __PLUGIN_SURFACES: $crate::SurfaceRegistrationOps = $crate::SurfaceRegistrationOps {
            registrations: $registrations_fn,
        };
    };
}

/// Reference to an optional static section. Returns `Some(&STATIC)` or `None`.
#[macro_export]
#[doc(hidden)]
macro_rules! __optional_static_ref {
    (config_test, config_test: [ $($ct_kind:expr),+ ]) => {
        Some(&__PLUGIN_CONFIG_TEST)
    };
    (config_test) => {
        None
    };
    (type_settings, type_settings: $ts:tt) => {
        Some(&__PLUGIN_TYPE_SETTINGS)
    };
    (type_settings) => {
        None
    };
    (surface_actions, surface_actions: { owned_surface_ids: $surface_action_ids:expr }) => {
        Some(&__PLUGIN_SURFACE_ACTIONS)
    };
    (surface_actions) => {
        None
    };
    (surfaces, surfaces: { registrations: $surface_registrations_fn:expr }) => {
        Some(&__PLUGIN_SURFACES)
    };
    (surfaces) => {
        None
    };
}

/// Wrap an expression in `Some()`, or produce `None` if absent.
#[macro_export]
#[doc(hidden)]
macro_rules! __option_expr {
    ($expr:expr) => {
        Some($expr)
    };
    () => {
        None
    };
}

/// Use the provided slice expression, or `&[]` if absent.
#[macro_export]
#[doc(hidden)]
macro_rules! __or_empty_slice {
    ($expr:expr) => {
        $expr
    };
    () => {
        &[]
    };
}
