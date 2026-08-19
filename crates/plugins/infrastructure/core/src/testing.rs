//! Shared mock [`CommandExecutor`] implementations for plugin unit tests.
//!
//! Enable the `testing` Cargo feature to use these types in `[dev-dependencies]`:
//!
//! ```toml
//! [dev-dependencies]
//! uptrakit-plugin-infrastructure-core = { workspace = true, features = ["testing"] }
//! ```
//!
//! Then bring them into scope inside a test module:
//!
//! ```rust,ignore
//! use uptrakit_plugin_infrastructure_core::testing::{FixedOutputExecutor, RoutedOutputExecutor};
//! ```

use std::sync::Arc;

use async_trait::async_trait;

use crate::host_runtime::{HostRuntime, StandardHostRuntime};
use crate::{CommandExecutor, CommandOutput, CommandSpec, UpdateOutputLine};
use uptrakit_shared_types::HostCapabilities;

#[cfg(feature = "agent-infra")]
use crate::agent_infra::{InfraActionInvokeError, InfraActionInvoker};
#[cfg(feature = "agent-infra")]
use crate::surfaces::SurfaceActionResponse;
#[cfg(feature = "agent-infra")]
use std::collections::VecDeque;

/// Returns the same output and exit code for every command.
///
/// - [`execute`]: always returns `Ok(CommandOutput { output, exit_code })`
/// - [`execute_quiet`]: returns `Ok(...)` when `exit_code == 0`;
///   returns `Err(CommandError::CommandFailed(code))` otherwise
///
/// [`execute`]: CommandExecutor::execute
/// [`execute_quiet`]: CommandExecutor::execute_quiet
pub struct FixedOutputExecutor {
    output: String,
    exit_code: i32,
}

impl FixedOutputExecutor {
    /// Returns successful execution (exit code 0) with the given stdout.
    pub fn success(output: impl Into<String>) -> Arc<dyn CommandExecutor> {
        Arc::new(Self {
            output: output.into(),
            exit_code: 0,
        })
    }

    /// Returns empty output with the given exit code.
    ///
    /// `execute` returns `Ok` with that code; `execute_quiet` returns
    /// `Err(CommandFailed)` for any non-zero code.
    pub fn failure(exit_code: i32) -> Arc<dyn CommandExecutor> {
        Arc::new(Self {
            output: String::new(),
            exit_code,
        })
    }

    /// Returns the given output and exit code.
    ///
    /// Equivalent to [`success`] when `exit_code == 0`.
    ///
    /// [`success`]: FixedOutputExecutor::success
    #[expect(
        clippy::new_ret_no_self,
        reason = "constructor returns an Arc<dyn CommandExecutor> to hide the concrete type from callers"
    )]
    pub fn new(output: impl Into<String>, exit_code: i32) -> Arc<dyn CommandExecutor> {
        Arc::new(Self {
            output: output.into(),
            exit_code,
        })
    }
}

#[async_trait]
impl CommandExecutor for FixedOutputExecutor {
    async fn execute(
        &self,
        _spec: &CommandSpec,
        _output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_command::Result<CommandOutput> {
        Ok(CommandOutput {
            output: self.output.clone(),
            exit_code: self.exit_code,
        })
    }

    async fn execute_quiet(&self, _spec: &CommandSpec) -> uptrakit_command::Result<CommandOutput> {
        if self.exit_code == 0 {
            Ok(CommandOutput {
                output: self.output.clone(),
                exit_code: 0,
            })
        } else {
            use rootcause::prelude::*;
            bail!(uptrakit_command::CommandError::CommandFailed(
                self.exit_code
            ))
        }
    }
}

/// Routes `execute` and `execute_quiet` calls by the command program name.
///
/// Unrecognised programs (and shell-mode specs) return empty output with
/// exit code 0. Both methods always return `Ok(CommandOutput { ... })`
/// regardless of exit code — use this when the plugin logic inspects
/// `exit_code` directly rather than relying on the executor to propagate
/// failures.
///
/// # Construction
///
/// ```rust,ignore
/// // All routes succeed (exit code 0):
/// let exec = RoutedOutputExecutor::success([("rpm", rpm_output)]);
///
/// // Per-route exit codes:
/// let exec = RoutedOutputExecutor::new([("dnf", output, 100)]);
/// ```
pub struct RoutedOutputExecutor {
    routes: Vec<(&'static str, String, i32)>,
}

impl RoutedOutputExecutor {
    /// Build from `(program, stdout)` pairs; all routes return exit code 0.
    pub fn success(
        routes: impl IntoIterator<Item = (&'static str, &'static str)>,
    ) -> Arc<dyn CommandExecutor> {
        Arc::new(Self {
            routes: routes
                .into_iter()
                .map(|(p, o)| (p, o.to_string(), 0))
                .collect(),
        })
    }

    /// Build from `(program, stdout, exit_code)` triples.
    #[expect(
        clippy::new_ret_no_self,
        reason = "constructor returns an Arc<dyn CommandExecutor> to hide the concrete type from callers"
    )]
    pub fn new(
        routes: impl IntoIterator<Item = (&'static str, &'static str, i32)>,
    ) -> Arc<dyn CommandExecutor> {
        Arc::new(Self {
            routes: routes
                .into_iter()
                .map(|(p, o, c)| (p, o.to_string(), c))
                .collect(),
        })
    }

    fn lookup(&self, spec: &CommandSpec) -> (String, i32) {
        if let crate::CommandMode::Exec { program, .. } = &spec.mode {
            for (name, out, code) in &self.routes {
                if program == *name {
                    return (out.clone(), *code);
                }
            }
        }
        (String::new(), 0)
    }
}

#[async_trait]
impl CommandExecutor for RoutedOutputExecutor {
    async fn execute(
        &self,
        spec: &CommandSpec,
        _output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_command::Result<CommandOutput> {
        let (output, exit_code) = self.lookup(spec);
        Ok(CommandOutput { output, exit_code })
    }

    async fn execute_quiet(&self, spec: &CommandSpec) -> uptrakit_command::Result<CommandOutput> {
        let (output, exit_code) = self.lookup(spec);
        Ok(CommandOutput { output, exit_code })
    }
}

/// Outcome closure for [`RecordingExecutor`] — aliased so the field type stays
/// below `clippy::type_complexity` (denied via `clippy::all`).
type OutcomeFn = Box<dyn Fn() -> uptrakit_command::Result<CommandOutput> + Send + Sync>;

/// Records every [`CommandSpec`] it is handed, then yields a caller-supplied
/// outcome — the double for tests that must assert *which* command reached the
/// executor (routing regressions), or drive the executor's `Err(CommandFailed)`
/// contract that [`FixedOutputExecutor`]/[`RoutedOutputExecutor`] cannot express
/// on their `execute` path.
///
/// Both [`execute`] and [`execute_quiet`] record the spec and return the
/// configured outcome. The outcome is a boxed closure (not a stored `Result`)
/// because [`uptrakit_command::CommandError`] is not `Clone`, so it is rebuilt
/// fresh on each call.
///
/// - [`ok`]: `Ok(CommandOutput { exit_code, .. })` — the command ran.
/// - [`failed`]: `Err(CommandError::CommandFailed(code))` — the real contract a
///   `LocalCommandExecutor`/SSH executor returns for a non-zero exit.
/// - [`erroring`]: any caller-supplied `Err` (e.g. `UnsupportedShell`,
///   `UnsupportedOperation`) for transport/spawn-failure cases.
///
/// [`execute`]: CommandExecutor::execute
/// [`execute_quiet`]: CommandExecutor::execute_quiet
/// [`ok`]: RecordingExecutor::ok
/// [`failed`]: RecordingExecutor::failed
/// [`erroring`]: RecordingExecutor::erroring
pub struct RecordingExecutor {
    recorded: parking_lot::Mutex<Vec<CommandSpec>>,
    outcome: OutcomeFn,
}

impl RecordingExecutor {
    /// Records every spec, always returning `Ok(CommandOutput { exit_code, .. })`.
    pub fn ok(exit_code: i32) -> Arc<Self> {
        Arc::new(Self {
            recorded: parking_lot::Mutex::new(Vec::new()),
            outcome: Box::new(move || {
                Ok(CommandOutput {
                    output: String::new(),
                    exit_code,
                })
            }),
        })
    }

    /// Records every spec, always returning `Err(CommandError::CommandFailed(code))`.
    pub fn failed(code: i32) -> Arc<Self> {
        Arc::new(Self {
            recorded: parking_lot::Mutex::new(Vec::new()),
            outcome: Box::new(move || {
                use rootcause::prelude::*;
                bail!(uptrakit_command::CommandError::CommandFailed(code))
            }),
        })
    }

    /// Records every spec, returning whatever `f` produces on each call.
    pub fn erroring<F>(f: F) -> Arc<Self>
    where
        F: Fn() -> uptrakit_command::Result<CommandOutput> + Send + Sync + 'static,
    {
        Arc::new(Self {
            recorded: parking_lot::Mutex::new(Vec::new()),
            outcome: Box::new(f),
        })
    }

    /// Snapshot of every [`CommandSpec`] recorded so far, in call order.
    pub fn recorded(&self) -> Vec<CommandSpec> {
        self.recorded.lock().clone()
    }
}

#[async_trait]
impl CommandExecutor for RecordingExecutor {
    async fn execute(
        &self,
        spec: &CommandSpec,
        _output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_command::Result<CommandOutput> {
        self.recorded.lock().push(spec.clone());
        (self.outcome)()
    }

    async fn execute_quiet(&self, spec: &CommandSpec) -> uptrakit_command::Result<CommandOutput> {
        self.recorded.lock().push(spec.clone());
        (self.outcome)()
    }
}

/// Build a [`HostRuntime`] backed by [`LocalCommandExecutor`] for unit tests.
///
/// Use this when the test needs a real executor (e.g., tests that test
/// host-compatibility detection against the actual host environment).
pub fn test_runtime() -> Arc<dyn HostRuntime> {
    Arc::new(StandardHostRuntime::new(
        Arc::new(crate::LocalCommandExecutor),
        HostCapabilities::default(),
    ))
}

/// Build a [`HostRuntime`] backed by the provided executor for unit tests.
///
/// Use this in the majority of unit tests where you control command output via
/// [`FixedOutputExecutor`] or [`RoutedOutputExecutor`].
pub fn test_runtime_with_executor(executor: Arc<dyn CommandExecutor>) -> Arc<dyn HostRuntime> {
    Arc::new(StandardHostRuntime::new(
        executor,
        HostCapabilities::default(),
    ))
}

/// Recording test double for [`crate::agent_infra::InfraActionInvoker`].
///
/// Records every `(surface_id, action_id, params)` invocation and replays
/// queued responses FIFO; an empty queue yields a generic success response.
#[cfg(feature = "agent-infra")]
pub struct RecordingActionInvoker {
    calls: parking_lot::Mutex<Vec<(String, String, serde_json::Value)>>,
    responses: parking_lot::Mutex<VecDeque<Result<SurfaceActionResponse, InfraActionInvokeError>>>,
}

#[cfg(feature = "agent-infra")]
impl RecordingActionInvoker {
    pub fn new() -> Self {
        Self {
            calls: parking_lot::Mutex::new(Vec::new()),
            responses: parking_lot::Mutex::new(VecDeque::new()),
        }
    }

    /// Queue the response returned by the next `invoke` call (FIFO).
    pub fn push_response(&self, response: Result<SurfaceActionResponse, InfraActionInvokeError>) {
        self.responses.lock().push_back(response);
    }

    /// All invocations recorded so far.
    pub fn calls(&self) -> Vec<(String, String, serde_json::Value)> {
        self.calls.lock().clone()
    }
}

#[cfg(feature = "agent-infra")]
impl Default for RecordingActionInvoker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "agent-infra")]
#[async_trait]
impl InfraActionInvoker for RecordingActionInvoker {
    async fn invoke(
        &self,
        surface_id: &str,
        action_id: &str,
        params: serde_json::Value,
    ) -> std::result::Result<SurfaceActionResponse, InfraActionInvokeError> {
        self.calls
            .lock()
            .push((surface_id.to_string(), action_id.to_string(), params));
        self.responses.lock().pop_front().unwrap_or_else(|| {
            Ok(SurfaceActionResponse {
                request_id: uuid::Uuid::nil(),
                success: true,
                result: None,
                error: None,
            })
        })
    }
}

/// Synthetic Instance-scoped, surface-bearing plugin fixture (spec 2026-07-27 §5.1).
///
/// Consumed by the catalog enablement guard tests and by web-api's
/// effective-enablement route tests. Test-local only: it is never listed in
/// the registry crate's `all_descriptors()`, so ADR-0032 monotonicity guards
/// (which read descriptor-level builders) never see it.
pub mod instance_surface_fixture {
    #![expect(
        clippy::expect_used,
        reason = "infallible literal surface ID and value constructions; panic would indicate a programming error in the surface manifest"
    )]

    use uptrakit_surfaces as surfaces;

    use crate::descriptor::{
        ConfigModel, PluginFamily, PluginScope, SurfaceActionContext, SurfaceActionError,
    };
    use crate::{
        InteractionDelivery, PluginSurface, PluginSurfaceRegistration, RegisteredInteraction,
    };

    /// Plugin type id of the fixture.
    pub const TYPE_ID: &str = "__test_instance_surface_plugin";
    /// Surface id of the fixture's single surface.
    pub const SURFACE_ID: &str = "test-instance.surface";
    /// Interaction id of the fixture's single `PluginHandled` interaction.
    pub const INTERACTION_ID: &str = "ping";

    fn ping_handler<'a>(
        _ctx: &'a SurfaceActionContext<'a>,
        _params: serde_json::Value,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<serde_json::Value, SurfaceActionError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async { Ok(serde_json::json!({ "pong": true })) })
    }

    fn fixture_surface_descriptor() -> surfaces::SurfaceDescriptor {
        surfaces::SurfaceDescriptor::builder()
            .surface_id(surfaces::SurfaceId::new(SURFACE_ID).expect("literal surface id is valid"))
            .label("Test Instance Surface")
            .priority(100)
            .slot(surfaces::SLOT_SETTINGS_TABS)
            .scope(surfaces::Scope::Global)
            .targeting(surfaces::Targeting::Universal)
            .provider_kind(surfaces::ProviderKind::Plugin)
            .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::UniversalTargeting,
                surfaces::Capability::MutationAction,
            ]))
            .root_node(surfaces::SurfaceNode::TextBlock {
                text: "ok".to_string(),
            })
            .build()
    }

    fn registrations() -> Vec<PluginSurfaceRegistration> {
        vec![PluginSurfaceRegistration {
            surfaces: vec![PluginSurface {
                descriptor: fixture_surface_descriptor(),
                interactions: vec![RegisteredInteraction::new(
                    surfaces::InteractionDescriptor::new(
                        surfaces::InteractionId::new(INTERACTION_ID)
                            .expect("literal interaction id is valid"),
                        surfaces::InteractionKind::MutationAction,
                        "Ping",
                        surfaces::InteractionTransport::ControllerLocal,
                    ),
                    InteractionDelivery::PluginHandled(ping_handler),
                )],
                data_sources: vec![],
            }],
        }]
    }

    #[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
    struct Config;

    impl crate::PluginConfig for Config {}

    #[expect(
        dead_code,
        reason = "constructed by declare_plugin! generated code; not directly instantiated in tests"
    )]
    struct Plugin;

    crate::declare_plugin!(
        Plugin,
        Config,
        TYPE_ID,
        {
            display_name: "Test Instance Surface Plugin",
            family: PluginFamily::Enhancement,
            config_model: ConfigModel::None,
            scope: PluginScope::Instance,
            roles: [],
            surfaces: {
                registrations: registrations,
            },
        }
    );
}
