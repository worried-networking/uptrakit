pub(crate) mod builtins;
pub(crate) mod embedded_host;
pub(crate) mod yielding;

#[cfg(any(
    feature = "embedded-scheduler",
    feature = "embedded-agent",
    feature = "embedded-ssh-agent",
    feature = "embedded-mqtt"
))]
pub(crate) use embedded_host::BuiltinServiceHost;
