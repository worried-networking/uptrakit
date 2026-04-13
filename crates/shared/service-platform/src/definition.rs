use std::collections::BTreeSet;

use uptrakit_internal_wire::Capability;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceKind {
    Agent,
    AgentSsh,
    Scheduler,
    Mqtt,
}

pub struct ServiceDefinition<R> {
    pub kind: ServiceKind,
    pub app_name: &'static str,
    pub capabilities: fn() -> BTreeSet<Capability>,
    pub scope: crate::context::ServiceScope,
    pub yield_policy: crate::yielding::YieldPolicy,
    pub build: fn() -> R,
}
