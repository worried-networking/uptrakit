// Shared helpers include members not used by system tests
// (raw_get, raw_post, start_with_trust_domain added for the harness).
#![expect(
    dead_code,
    reason = "shared test helpers include members not exercised by the system test binary"
)]

mod helpers;

mod system {
    mod agent_enrollment;
    mod agent_ssh_enrollment;
    mod controller_startup;
    mod full_system;
    mod mqtt_enrollment;
    mod scheduler_enrollment;
}
