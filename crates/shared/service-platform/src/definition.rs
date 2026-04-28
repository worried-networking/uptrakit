use std::collections::BTreeSet;

use uptrakit_wire::Capability;

use crate::runtime::ServiceRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceKind {
    Agent,
    AgentSsh,
    Scheduler,
    Mqtt,
}

pub struct ServiceDefinition<R: ServiceRuntime> {
    pub kind: ServiceKind,
    pub app_name: &'static str,
    pub capabilities: fn() -> BTreeSet<Capability>,
    pub scope: crate::context::ServiceScope,
    pub yield_policy: crate::yielding::YieldPolicy,
    pub build: fn() -> R,
}
