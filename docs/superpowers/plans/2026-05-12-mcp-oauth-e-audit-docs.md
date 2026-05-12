# MCP OAuth — Plan E: Audit Events + Documentation (Phase 5 + Phase 6)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Register every new OAuth audit event type with the `uptrakit-audit-log` action-type catalog, classify them in
`auth_audit_classification.rs`, and ship the full documentation set per spec §21 (one new ADR, four new docs, eight
updates to existing docs). After this plan merges, an Operator may safely flip `oauth.mcp_enabled = true` on a deployed
controller.

**Architecture:** Two clean phases — first the audit-event registrations (the helpers in Plans B and D already wire
structured fields through stub emit calls; this plan plugs the `RegisteredAuditAction` values into those stubs), then
the documentation push. Each documentation file is its own task with explicit acceptance criteria so reviewers can
verify completeness.

**Tech Stack:** `uptrakit-audit-log` action-type registry + markdownlint + prettier + Conventional Commits.

**Spec:** `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` (commit `b7ee4a852`).

**Status:** Draft → Ready for review.

---

## Prerequisites

- **Plan A** (foundation) merged.
- **Plan B** (AS routes) merged — audit helpers stubbed.
- **Plan C** (frontend) merged — user-facing docs reference existing routes.
- **Plan D** (RS + CIMD) merged — `MCP_OAUTH_AUTHENTICATE` + `OAUTH_CIMD_PARSE_FAILED` stubbed.

## Snapshot binding

- "use semantic audit emission (AuditEntry + AuditEmitter) instead of legacy target: 'security_audit'" — every event
  registered with action-type catalog, not free-form strings
- "markdown line length: 150 chars max (enforced; code_blocks and tables ignored)" — every new/modified markdown file
- "Use prettier for markdown formatting" — `npx prettier --write` before commit
- "Conventional Commits: type(scope): description" — `docs(adr)`, `docs(security)`, `docs(admin)`, `feat(audit-log)`
  scopes
- "PR description: what changed and why, how tested, risks/rollout/migrations" — applies if this plan lands as multiple
  PRs

## File Structure

**New files:**

- `docs/adr/0007-mcp-oauth-authorization-server-placement.md` (new ADR)
- `docs/development/oauth-mcp.md` (engineering guide)
- `docs/security/oauth-mcp.md` (security model)
- `docs/end-user/mcp-clients.md` (user guide)
- `docs/admin/oauth-clients.md` (admin runbook + first-run checklist)

**Modified files:**

- `crates/shared/audit-log/src/action_type.rs` — register all OAuth audit event constants
- `crates/ui/web-api/src/auth_audit_classification.rs` — classify the events
- `crates/ui/web-api/src/oauth/audit.rs` — replace stubs with concrete action-type references
- `crates/ui/mcp/src/oauth/audit.rs` (if exists) — same for `MCP_OAUTH_AUTHENTICATE`
- `CONTEXT.md` — add 5 glossary entries + flagged-ambiguity update
- `docs/adr/0001-web-api-decomposition-strategy.md` — append OAuth-AS-deferred row
- `docs/security/auth-and-authorization.md` — new "OAuth 2.1 MCP" section
- `docs/security/audit-logs.md` — document new event types + reason codes
- `docs/end-user/profile-tokens.md` — clarify API tokens stay permanent parallel path
- `docs/end-user/cli-usage.md` — note CLI unchanged in this work
- `docs/superpowers/specs/2026-05-01-extract-mcp-crate-design.md` — replace `// TODO: replace with OAuth 2.1 validation`
  reference with link to the new spec
- `README.md` — one-paragraph mention + link to user guide

---

## Tasks

### Task 1: Register OAuth audit constants in variants() + round-trip test

**Files:**

- Modify: `crates/shared/audit-log/src/action_type.rs`

The 19 OAuth `RegisteredAuditAction` constants are already declared by Plan A Task 17 — Plans B and D emit real audit
events via `AuditEntry::builder(AuditActionType::OAUTH_...)` from day one. This task wires the constants into the
`variants()` registry array and adds the stable-string round-trip test that proves the catalog is complete.

- [ ] **Step 1: Append each OAuth constant to the `variants()` array**

Per the existing pattern at line 242 in `action_type.rs`, append every OAuth constant added in Plan A Task 17 to the
`variants()` list. Order: alphabetical within the OAuth block, after the existing `AUTH_*` block.

- [ ] **Step 2: Add stable-string round-trip test**

```rust
#[test]
fn oauth_actions_have_stable_strings() {
    let expected: &[(RegisteredAuditAction, &str)] = &[
        (AuditActionType::OAUTH_AUTHORIZE_REQUEST, "oauth.authorize_request"),
        (AuditActionType::OAUTH_TOKEN_ISSUED, "oauth.token_issued"),
        (AuditActionType::OAUTH_TOKEN_REJECTED, "oauth.token_rejected"),
        (AuditActionType::OAUTH_REFRESH_ROTATED, "oauth.refresh_rotated"),
        (AuditActionType::OAUTH_REFRESH_REPLAY_DETECTED, "oauth.refresh_replay_detected"),
        (AuditActionType::OAUTH_CLIENT_REGISTERED, "oauth.client_registered"),
        (AuditActionType::OAUTH_CLIENT_FIRST_USE, "oauth.client_first_use"),
        (AuditActionType::OAUTH_CLIENT_METADATA_REFRESHED, "oauth.client_metadata_refreshed"),
        (
            AuditActionType::OAUTH_CLIENT_METADATA_CHANGED_MATERIALLY,
            "oauth.client_metadata_changed_materially",
        ),
        (AuditActionType::OAUTH_CLIENT_TRUSTED, "oauth.client_trusted"),
        (AuditActionType::OAUTH_CLIENT_REVOKED, "oauth.client_revoked"),
        (
            AuditActionType::OAUTH_CLIENT_REGISTRATION_RATE_LIMITED,
            "oauth.client_registration_rate_limited",
        ),
        (
            AuditActionType::OAUTH_CONFIG_AUDIENCE_HOSTS_CHANGED,
            "oauth.config_audience_hosts_changed",
        ),
        (AuditActionType::OAUTH_CIMD_PARSE_FAILED, "oauth.cimd_parse_failed"),
        (AuditActionType::OAUTH_CONSENT_GRANT, "oauth.consent_grant"),
        (AuditActionType::OAUTH_CONSENT_DENY, "oauth.consent_deny"),
        (AuditActionType::OAUTH_CONSENT_REVOKE, "oauth.consent_revoke"),
        (AuditActionType::OAUTH_RATE_LIMITED, "oauth.rate_limited"),
        (AuditActionType::MCP_OAUTH_AUTHENTICATE, "mcp.oauth_authenticate"),
    ];
    for (action, name) in expected {
        assert_eq!(action.as_str(), *name);
        assert!(
            AuditActionType::variants().iter().any(|v| v == action),
            "{name} missing from variants()",
        );
    }
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p uptrakit-audit-log action_type` Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(audit-log): wire OAuth constants into variants() registry

Plan A Task 17 declared the 19 constants; this commit registers them with
the action-type catalog so the audit pipeline lists them in admin views."
```

### Task 2: Classify OAuth events in auth_audit_classification

**Files:**

- Modify: `crates/ui/web-api/src/auth_audit_classification.rs`

- [ ] **Step 1: Classify**

Per existing pattern, attach a classification (e.g., `AuditClass::AuthEvent`, `AuditClass::ClientLifecycle`,
`AuditClass::SecurityCritical`) to each new event. Replay-detection events go to `SecurityCritical`. Configuration
changes (`OAUTH_CONFIG_AUDIENCE_HOSTS_CHANGED`) go to `SecurityCritical`. Routine token issuance / refresh rotation goes
to `AuthEvent`.

- [ ] **Step 2: Add test**

Cover: each event classifies to the expected class; security-critical events appear in the security-relevant filtered
audit stream.

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(web-api): classify OAuth audit events

Replay-detection + audience-hosts config changes flagged SecurityCritical."
```

### Task 3: Verify OAuth audit emissions end-to-end

**Files:**

- Test only — no production code changes (Plans B and D already emit via
  `AuditEntry::builder(AuditActionType::OAUTH_...)` from day one because Plan A Task 17 declared the constants).

- [ ] **Step 1: Run all OAuth integration tests**

Run: `cargo test -p uptrakit-web-api oauth && cargo test -p uptrakit-mcp oauth` Expected: PASS — every test that asserts
an audit event was emitted now finds the corresponding row in the test audit collector (Task 2's classification already
in place).

- [ ] **Step 2: Run Docker integration test for OAuth round-trip**

Run: `cargo test -p uptrakit-integration-tests -- --ignored oauth_end_to_end` Expected: PASS — Plan D Task 14's
end-to-end test asserts the full audit chain (authorize → consent grant → token issued → MCP request →
MCP_OAUTH_AUTHENTICATE).

- [ ] **Step 3: Commit (only if any fix was needed)**

```bash
git commit -m "feat(oauth): wire audit helpers to registered action types

Stubs replaced; integration tests now assert structured audit events."
```

### Task 4: New ADR 0007 — AS placement decision

**Files:**

- Create: `docs/adr/0007-mcp-oauth-authorization-server-placement.md`

- [ ] **Step 1: Write the ADR**

Use the existing ADR format (see `docs/adr/0001-...md`, `0006-...md`). Section headings:

- **Status**: Accepted
- **Context**: state the problem (MCP needs OAuth 2.1; AS placement is non-trivial)
- **Decision**: embed AS in `uptrakit-web-api`; defer extraction; HS256 v1 with kid; CIMD over DCR priority; reject
  Model B v1 with seams preserved
- **Consequences**: list operational and architectural consequences, both positive and negative
- **Alternatives considered**: extract to `uptrakit-oauth-as`; delegate fully to external IdP (Model B); use
  `oxide-auth` crate; asymmetric JWT v1; promote scopes to granular v1

Cross-link to the design spec at `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` for full
rationale; ADR carries only the decision summary.

Run prettier:

```bash
npx prettier --write docs/adr/0007-mcp-oauth-authorization-server-placement.md
markdownlint --config .markdownlint.json docs/adr/0007-mcp-oauth-authorization-server-placement.md
```

- [ ] **Step 2: Commit**

```bash
git add docs/adr/0007-mcp-oauth-authorization-server-placement.md
git commit -m "docs(adr): 0007 MCP OAuth Authorization Server placement

Records the AS-in-web-api decision, HS256 v1, CIMD > DCR priority,
Phase 2 deferral of Dashboard-API-as-RS / Model B / asymmetric signing."
```

### Task 5: New engineering doc — `docs/development/oauth-mcp.md`

**Files:**

- Create: `docs/development/oauth-mcp.md`

- [ ] **Step 1: Write the doc**

Audience: engineers adding new MCP tools or modifying the OAuth surface. Sections:

- Adding an MCP tool that needs OAuth: declare a `ToolAuth` constant; pick scope per the read-vs-write rule; reference
  the `crates/ui/mcp/src/tools/update.rs` `TRIGGER_UPDATE_AUTH` pattern.
- Scope migration policy: future granular scopes are additive only; `mcp:read` / `mcp:write` retain v1 semantics.
- Token validation invariants: HS256 pinned; `aud` exact-match against canonical resource URL; `iss` exact-match against
  canonical issuer; required spec claims enumerated.
- Audit emission: how to use the `oauth::audit` helpers + new action-type constants.
- Test patterns: clock injection via `Arc<dyn Fn() -> OffsetDateTime + Send + Sync>`; in-memory SQLite TestApp; FK
  constraints enforced.
- How to disable OAuth in tests: flip `oauth.mcp_enabled = false`; assert 404 on every surface.

Run prettier + markdownlint.

- [ ] **Step 2: Commit**

```bash
git commit -m "docs(development): oauth-mcp engineering guide

Adding tools, scope assignment rule, token invariants, audit emission,
test patterns."
```

### Task 6: New security doc — `docs/security/oauth-mcp.md`

**Files:**

- Create: `docs/security/oauth-mcp.md`

- [ ] **Step 1: Write the doc**

Audience: security reviewers + auditors. Sections:

- Threat model: phishing via DCR; CIMD silent re-keying; token theft; multi-controller secret drift; audience confusion.
- Mitigations: opt-in DCR/CIMD; consent screen typed-confirmation against redirect URI hostname; CIMD content-hash
  material-change re-consent; SsrfSafeResolver; multi-controller boot guard; algorithm pinning; refresh-token family
  replay detection.
- Deviation from RFC 9068: HMAC HS256 v1, not asymmetric. Justification per spec §24. Migration path documented.
- Key rotation: boot-time `kid` from secret fingerprint; rotation requires a hard cut v1; planned `oauth_jwt_keys`
  overlap window post-v1.

Run prettier + markdownlint.

- [ ] **Step 2: Commit**

```bash
git commit -m "docs(security): oauth-mcp threat model + mitigations + key rotation"
```

### Task 7: New end-user doc — `docs/end-user/mcp-clients.md`

**Files:**

- Create: `docs/end-user/mcp-clients.md`

- [ ] **Step 1: Write the doc**

Audience: end-users connecting Claude Desktop / Cursor / Inspector to a controller. Sections:

- Prerequisites (controller running, OAuth enabled, account exists)
- Connect Claude Desktop: paste controller URL, sign in, grant consent
- Connect Cursor: same flow
- What the consent screen shows: client name, redirect URI hostname, scopes
- Reviewing your authorized apps: `/settings/account/authorized-apps`
- Revoking access
- Troubleshooting `WWW-Authenticate` errors: how to recognise an `insufficient_scope` and what to do
- Reporting suspicious consent prompts

Run prettier + markdownlint.

- [ ] **Step 2: Commit**

```bash
git commit -m "docs(end-user): mcp-clients connection guide

Covers Claude Desktop, Cursor, the consent screen, Authorized Apps, and
common WWW-Authenticate errors."
```

### Task 8: New admin doc — `docs/admin/oauth-clients.md`

**Files:**

- Create: `docs/admin/oauth-clients.md`

- [ ] **Step 1: Write the doc**

Audience: Operators. Sections:

- **First-run checklist** (numbered, per spec §20 Phase 1.5):
  1. Set `oauth.canonical_host`.
  2. Set `oauth.accepted_audience_hosts` if behind a reverse proxy or split DNS.
  3. (Optional) Set `oauth.jwt_signing_secret`; otherwise a per-boot secret is generated with a WARN.
  4. Flip `oauth.mcp_enabled = true`.
  5. Optionally enable DCR / CIMD after reading the threat model.
- Rate-limit knobs: every `oauth.rate.*` setting with default value and tuning guidance.
- Reviewing OAuth Clients in the Dashboard: list view, status badges, trust promotion, revocation.
- Monitoring: which audit events to alert on; how to spot DCR-driven phishing attempts via
  `OAUTH_CLIENT_REGISTRATION_RATE_LIMITED` and `OAUTH_CLIENT_FIRST_USE` clusters.
- Multi-controller deployments: not supported v1; `oauth.allow_multi_controller_unsafe` is intentionally a footgun.
- Rotating `oauth.jwt_signing_secret`: hard cut; clients receive 401, refresh dance succeeds within ~15 min.
- Upgrading from a deployment that never enabled OAuth: this doc's first-run checklist is the canonical path.

Run prettier + markdownlint.

- [ ] **Step 2: Commit**

```bash
git commit -m "docs(admin): oauth-clients runbook + first-run checklist

Per spec §20 Phase 1.5. Captures the ordered toggle sequence operators
must follow before flipping oauth.mcp_enabled."
```

### Task 9: Update `CONTEXT.md` with five glossary entries

**Files:**

- Modify: `CONTEXT.md`

- [ ] **Step 1: Add entries**

Per spec §18.3 verbatim:

- **OAuth Client**
- **MCP Resource Server**
- **MCP Authorization Server**
- **Consent Grant**
- **Scope (OAuth)**

Insert in alphabetical order within the existing glossary. Use the existing entry format (term name in bold; one
paragraph; `_Avoid_:` line for term collisions).

- [ ] **Step 2: Update the "Flagged ambiguities" section**

Add the spec §18.3 entry about `scope` (OAuth) vs `Permission` collision.

- [ ] **Step 3: Run prettier + markdownlint**

```bash
npx prettier --write CONTEXT.md
markdownlint --config .markdownlint.json CONTEXT.md
```

- [ ] **Step 4: Commit**

```bash
git commit -m "docs(context): add OAuth domain terms + scope-vs-permission ambiguity"
```

### Task 10: Update ADR 0001 with deferred OAuth-AS row

**Files:**

- Modify: `docs/adr/0001-web-api-decomposition-strategy.md`

- [ ] **Step 1: Append the row**

Add a row to the candidates table per spec §18.3 / §21:

```markdown
| OAuth Authorization Server | Deferred (Phase 2) | Seam not yet clean — depends on JwtManager, AuthState, session
middleware, frontend. See ADR 0007. |
```

- [ ] **Step 2: Run gates**

- [ ] **Step 3: Commit**

```bash
git commit -m "docs(adr): note OAuth AS deferral in 0001 decomposition table"
```

### Task 11: Update `docs/security/auth-and-authorization.md` with OAuth 2.1 section

**Files:**

- Modify: `docs/security/auth-and-authorization.md`

- [ ] **Step 1: Add a new section**

Title: "OAuth 2.1 for MCP". Content: pointer to the design spec and ADR 0007; brief description of the dual-auth model
(API tokens + OAuth, prefix-dispatched); list the cross-rejection guarantees between Dashboard JWT and OAuth JWT.

Cross-link to:

- `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md`
- `docs/adr/0007-mcp-oauth-authorization-server-placement.md`
- `docs/security/oauth-mcp.md`
- `docs/development/oauth-mcp.md`

- [ ] **Step 2: Run gates**

- [ ] **Step 3: Commit**

```bash
git commit -m "docs(security): add OAuth 2.1 MCP section + cross-links"
```

### Task 12: Update `docs/security/audit-logs.md` with new event types

**Files:**

- Modify: `docs/security/audit-logs.md`

- [ ] **Step 1: Document every new event type**

For each of the 19 new events: action name, the actor type, the target type, the reason codes the event carries, and the
classification (`AuthEvent` / `ClientLifecycle` / `SecurityCritical`).

Group into sections matching spec §14.1: AS endpoint events; client lifecycle; consent; RS-side; rate-limit; CIMD.

- [ ] **Step 2: Run gates**

- [ ] **Step 3: Commit**

```bash
git commit -m "docs(security): document OAuth audit event types + reason codes"
```

### Task 13: Update `docs/end-user/profile-tokens.md`

**Files:**

- Modify: `docs/end-user/profile-tokens.md`

- [ ] **Step 1: Add clarifying paragraph**

State that opaque `upk_*` API tokens remain the canonical credential for non-interactive callers (CLI, CI, ops scripts).
OAuth 2.1 is layered alongside, not on top of, API tokens. Phase 2 may revisit deprecation; v1 ships no sunset
commitment. Cross-link to `docs/end-user/mcp-clients.md`.

- [ ] **Step 2: Run gates**

- [ ] **Step 3: Commit**

```bash
git commit -m "docs(end-user): clarify API tokens + OAuth coexistence in profile-tokens"
```

### Task 14: Update `docs/end-user/cli-usage.md`

**Files:**

- Modify: `docs/end-user/cli-usage.md`

- [ ] **Step 1: Add note**

Note that `uptrakit-cli` continues to use API tokens v1 — no behavioural change. Phase 2 may revisit. Cross-link to the
device-flow login page.

- [ ] **Step 2: Run gates**

- [ ] **Step 3: Commit**

```bash
git commit -m "docs(end-user): note CLI continues using API tokens v1"
```

### Task 15: Update the extract-mcp spec with link to this spec

**Files:**

- Modify: `docs/superpowers/specs/2026-05-01-extract-mcp-crate-design.md`

- [ ] **Step 1: Find the `// TODO: replace with OAuth 2.1 validation` reference**

Search for the OAuth 2.1 Forward Compatibility section. Replace the TODO scaffolding note with a link to the new spec at
`docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` plus the commit hash `b7ee4a852` (the
spec-creation commit) — actually use the current HEAD of the OAuth spec by the time this plan lands, not the draft
commit.

- [ ] **Step 2: Commit**

```bash
git commit -m "docs(spec): cross-link extract-mcp design to 2026-05-12 OAuth spec"
```

### Task 16: Update README.md with OAuth 2.1 paragraph

**Files:**

- Modify: `README.md`

- [ ] **Step 1: Add a short paragraph**

Under the existing "Features" or "MCP" section: one sentence on OAuth 2.1 MCP support; cross-link to
`docs/end-user/mcp-clients.md` for users and `docs/admin/oauth-clients.md` for operators.

- [ ] **Step 2: Run gates**

- [ ] **Step 3: Commit**

```bash
git commit -m "docs(readme): mention OAuth 2.1 MCP support"
```

### Task 17: Run full quality gates

- [ ] **Step 1: Run gates**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
npx prettier --check 'docs/**/*.md' 'README.md' 'CONTEXT.md'
cd frontend && npm run check && npm run build
```

- [ ] **Step 2: Fix any failures inline (no warnings suppressed)**

- [ ] **Step 3: Commit cleanups if any**

### Task 18: Verify the full doc deliverables list matches spec §21

- [ ] **Step 1: Walk spec §21 row by row**

Open `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` §21. For every row in the doc-deliverable
table, confirm the corresponding file was created or modified by Tasks 4–16 above.

- [ ] **Step 2: Open each new doc and verify**

Each new doc satisfies the "purpose" column in §21 verbatim (audience, scope of content).

- [ ] **Step 3: No commit required** — verification only.

### Task 19: Final smoke test — flip oauth.mcp_enabled in a staging deployment

- [ ] **Step 1: Outside this plan's scope**

Deployment validation belongs to the merge-orchestration / release workflow, not this implementation plan. Note here
that the v1 release notes should explicitly call out: "Operators must follow the first-run checklist in
`docs/admin/oauth-clients.md` before flipping `oauth.mcp_enabled = true`."

- [ ] **Step 2: Add release-notes blurb to wherever the project tracks release notes** (if such a file exists — check
      `CHANGELOG.md` or equivalent).

If no changelog convention exists, skip this step and let the merge-orchestration workflow handle release-note
authoring.

---

## Self-review checklist

- [ ] **Snapshot conformance**: every event uses `RegisteredAuditAction::new(...)` const + `AuditEntry::builder(...)` —
      never `target: "security_audit"` strings; every markdown file passes 150-char line cap (code/tables exempt) and
      prettier; every new doc lives at the path required by spec §21.
- [ ] **Idiomatic pattern check**: event-name strings follow the existing dotted convention (`oauth.token_issued`); ADR
      0007 follows the existing ADR format precisely; no doc duplicates content covered by the design spec — docs
      cross-link instead.
- [ ] **Documentation completeness**: every row in spec §21 has a corresponding task above. Verified in Task 18.
- [ ] **Task atomicity**: each task is a single coherent change with its own commit.
- [ ] **Phase ordering**: requires Plans A + B + C + D merged. Plan E is the last plan to land before the
      operator-driven `oauth.mcp_enabled = true` flip.
