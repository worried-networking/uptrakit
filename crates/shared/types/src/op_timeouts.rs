//! Operation-level deadlines for agent background operations.
//!
//! Shared between the in-operation deadlines enforced here and the agent's
//! background-op guard (`uptrakit_agent_core::client::BackgroundOps`,
//! 2026-08-22 spec amendment), whose dedup window must not undercut the
//! in-op deadline: a window shorter than `budget + OP_DEADLINE_GRACE` would
//! re-dispatch an operation that is still legitimately running.
//!
//! Version skew: agents predating these deadlines enforce nothing and can
//! still hang silently. There is NO controller-side watchdog — a hung
//! legacy agent surfaces only via the missed-pong disconnect (M1.11) and,
//! later, the `uptrakit-async-op-failure-surface` follow-up epic.

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
