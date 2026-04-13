use std::{
    collections::BTreeSet,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};

use uptrakit_internal_wire::Capability;
use uptrakit_service_platform::{
    RuntimeControl, RuntimeYieldState, ServiceContext, ServiceDefinition, ServiceKind,
    ServiceRuntime, ServiceScope, ServiceSession, YieldHook, YieldPolicy,
};
use uuid::Uuid;

#[derive(Default)]
struct DummyRuntime {
    activated: bool,
    drained: bool,
    aborted: bool,
}

impl DummyRuntime {
    fn build() -> Self {
        Self::default()
    }
}

struct DummySession {
    id: Uuid,
}

impl DummySession {
    fn new() -> Self {
        Self { id: Uuid::nil() }
    }
}

impl ServiceSession for DummySession {
    fn id(&self) -> Uuid {
        self.id
    }
}

struct DummyControl {
    stop_requested: bool,
}

impl RuntimeControl for DummyControl {
    fn stop_requested(&self) -> bool {
        self.stop_requested
    }
}

#[derive(Default)]
struct DummyYieldHook {
    starts: usize,
    stops: usize,
}

impl YieldHook for DummyYieldHook {
    fn on_yield_start(&mut self) {
        self.starts += 1;
    }

    fn on_yield_stop(&mut self) {
        self.stops += 1;
    }
}

fn no_capabilities() -> BTreeSet<Capability> {
    BTreeSet::new()
}

fn assert_runtime_definition<R: ServiceRuntime>(_: &ServiceDefinition<R>) {}

fn block_on<F: Future>(future: F) -> F::Output {
    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = Pin::from(Box::new(future));

    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

#[async_trait::async_trait]
impl ServiceRuntime for DummyRuntime {
    async fn activate(
        &mut self,
        session: &mut dyn ServiceSession,
        ctx: &mut ServiceContext,
    ) -> rootcause::Result<()> {
        self.activated = true;
        assert_eq!(session.id(), Uuid::nil());
        assert!(!ctx.yield_state.is_yielded());
        Ok(())
    }

    async fn run_until_stopped(
        &mut self,
        session: &mut dyn ServiceSession,
        ctx: &mut ServiceContext,
        control: &mut dyn RuntimeControl,
    ) -> rootcause::Result<()> {
        assert!(self.activated);
        assert_eq!(session.id(), Uuid::nil());
        assert_eq!(ctx.instance_id, Uuid::nil());
        assert!(control.stop_requested());
        Ok(())
    }

    async fn drain(&mut self, session: &mut dyn ServiceSession, ctx: &mut ServiceContext) {
        self.drained = true;
        assert_eq!(session.id(), Uuid::nil());
        ctx.yield_state.set_yielded(true);
    }

    async fn abort(&mut self, ctx: &mut ServiceContext) {
        self.aborted = true;
        assert!(ctx.yield_state.is_yielded());
    }
}

#[test]
fn platform_types_are_constructible() {
    let definition = ServiceDefinition {
        kind: ServiceKind::Agent,
        app_name: "uptrakit-agent",
        capabilities: no_capabilities,
        scope: ServiceScope::System,
        yield_policy: YieldPolicy::SameServiceSameHost,
        build: DummyRuntime::build,
    };
    assert_runtime_definition(&definition);

    let mut runtime = (definition.build)();
    let mut session = DummySession::new();
    let mut ctx = ServiceContext::new(Uuid::nil());
    let default_ctx = ServiceContext::default();
    let mut control = DummyControl {
        stop_requested: true,
    };
    let mut hook = DummyYieldHook::default();
    let yield_state = RuntimeYieldState::default();

    assert_eq!(definition.kind, ServiceKind::Agent);
    assert_eq!(definition.app_name, "uptrakit-agent");
    assert_eq!((definition.capabilities)(), BTreeSet::new());
    assert_eq!(definition.scope, ServiceScope::System);
    assert!(matches!(
        definition.yield_policy,
        YieldPolicy::SameServiceSameHost
    ));
    assert_eq!(session.id(), Uuid::nil());
    assert_ne!(default_ctx.instance_id, Uuid::nil());
    assert!(!yield_state.is_yielded());
    yield_state.set_yielded(true);
    assert!(yield_state.is_yielded());

    hook.on_yield_start();
    hook.on_yield_stop();
    assert_eq!(hook.starts, 1);
    assert_eq!(hook.stops, 1);

    block_on(runtime.activate(&mut session, &mut ctx)).expect("activate should succeed");
    block_on(runtime.run_until_stopped(&mut session, &mut ctx, &mut control))
        .expect("run should succeed");
    block_on(runtime.drain(&mut session, &mut ctx));
    assert!(ctx.yield_state.is_yielded());
    block_on(runtime.abort(&mut ctx));
    assert!(runtime.drained);
    assert!(runtime.aborted);
}

#[test]
fn standalone_runner_symbol_is_available() {
    let _ = uptrakit_service_platform::standalone::run_standalone::<DummyRuntime>;
}
