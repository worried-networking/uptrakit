use uuid::Uuid;

use crate::yielding::RuntimeYieldState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceScope {
    System,
    Tenant,
}

#[derive(Debug)]
pub struct ServiceContext {
    pub instance_id: Uuid,
    pub yield_state: RuntimeYieldState,
}

impl ServiceContext {
    #[must_use]
    pub fn new(instance_id: Uuid) -> Self {
        Self {
            instance_id,
            yield_state: RuntimeYieldState::default(),
        }
    }
}

impl Default for ServiceContext {
    fn default() -> Self {
        Self::new(Uuid::now_v7())
    }
}
