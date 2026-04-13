use uuid::Uuid;

pub trait ServiceSession: Send {
    fn id(&self) -> Uuid;
}
