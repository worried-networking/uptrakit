//! Agent operations shared between `routes::agents` and `routes::service_ws`.
//!
//! This module houses the core functions for host management, certificate
//! signing, and certificate revocation that are used by both the REST agent
//! enrollment routes and the WebSocket connection/handler modules. By placing
//! them in a neutral module, the circular dependency between `service_ws` →
//! `agents` ← `hosts` → `service_ws` is broken.

// Re-export the items that service_ws and hosts depend on from agents.
pub(crate) use super::agents::{
    EnrollParams, ServiceStatus, SystemServiceEnrollParams, do_enroll, do_enroll_system_service,
    do_sign_csr, do_sign_csr_for_system_service, find_or_create_host_and_link, revoke_certificate,
    revoke_system_certificate,
};
