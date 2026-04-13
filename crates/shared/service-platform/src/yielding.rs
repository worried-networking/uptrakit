use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YieldPolicy {
    SameServiceSameHost,
    SameServiceAnywhere,
    Never,
}

pub trait YieldHook {
    fn on_yield_start(&mut self);
    fn on_yield_stop(&mut self);
}

#[derive(Debug, Default)]
pub struct RuntimeYieldState {
    yielded: AtomicBool,
}

impl RuntimeYieldState {
    #[must_use]
    pub fn is_yielded(&self) -> bool {
        self.yielded.load(Ordering::Acquire)
    }

    pub fn set_yielded(&self, yielded: bool) {
        self.yielded.store(yielded, Ordering::Release);
    }
}
