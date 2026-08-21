//! Operation-level deadlines for agent background operations.
//!
//! Shared between the agent (which enforces them around whole operations)
//! and the controller (whose dispatch dedup TTLs must not undercut the
//! agent-side deadline — a dedup window shorter than the op deadline would
//! re-dispatch an operation that is still legitimately running).
//!
//! Version skew: agents predating these deadlines enforce nothing and can
//! still hang silently — the controller-side watchdog (Plan 3, M1.6) stays
//! the authority for those; these constants only guarantee behaviour for
//! upgraded agents.

use std::time::Duration;

/// Deadline for one whole version-check batch on the agent.
pub const VERSION_CHECK_OP_TIMEOUT: Duration = Duration::from_secs(1800);

/// Deadline for one whole discovery run on the agent.
pub const DISCOVERY_OP_TIMEOUT: Duration = Duration::from_secs(1800);

/// Deadline for a single plugin config test on the agent. Config tests back
/// an interactive UI flow, so the bound is tight.
pub const CONFIG_TEST_OP_TIMEOUT: Duration = Duration::from_secs(25);

/// Grace added to the *outer* backstop wraps around whole operations so the
/// inner per-group / per-plugin deadlines (which produce partial results)
/// always fire first. The outer wrap only triggers when the operation
/// wedges outside a bounded section (a bug), falling back to all-error
/// results so a terminal message is still guaranteed.
pub const OP_DEADLINE_GRACE: Duration = Duration::from_secs(60);
