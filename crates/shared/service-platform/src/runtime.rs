#[async_trait::async_trait]
pub trait ServiceRuntime: Send {
    async fn activate(
        &mut self,
        session: &mut dyn crate::session::ServiceSession,
        ctx: &mut crate::context::ServiceContext,
    ) -> rootcause::Result<()>;

    async fn run_until_stopped(
        &mut self,
        session: &mut dyn crate::session::ServiceSession,
        ctx: &mut crate::context::ServiceContext,
        control: &mut dyn RuntimeControl,
    ) -> rootcause::Result<()>;

    async fn drain(
        &mut self,
        session: &mut dyn crate::session::ServiceSession,
        ctx: &mut crate::context::ServiceContext,
    );

    async fn abort(&mut self, ctx: &mut crate::context::ServiceContext);
}

pub trait RuntimeControl: Send {
    fn stop_requested(&self) -> bool;
}
