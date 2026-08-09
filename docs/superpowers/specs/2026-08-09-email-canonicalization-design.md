# Email canonicalization — canonical stored form for `users.email`

Date: 2026-08-09
Status: proposed (pending owner review)
Design authority: `.superpowers/authn-and-authz-refactoring/03-principal-auth.md` § "Canonical email form
(independent fix; land before M3 starts)", adopted 2026-08-09 in `09-resolved-questions.md`
§ "Credential-layer review (2026-08-09)". This spec turns that section into an implementation design; it
does not re-litigate decisions recorded there or in 08/09. Mechanism-level deviations from the design
text are listed in § "Deviations from the design-authority mechanism text" with reasoning.

## Problem (tree-verified 2026-08-09)

No email normalization exists anywhere in the workspace, and `users.email` is unique under BINARY
collation (`string_uniq` in `m20260209_000001_initial.rs:81`, no `COLLATE` clause). Reachable today:

- **Case-variant duplicate accounts** — register `User@x.com` alongside `user@x.com`.
- **Case-sensitive password login** — login lookup (`routes/auth.rs:544`) compares the raw request
  string against the column.
- **Duplicate OIDC auto-create** — `resolve_oidc_user`'s email-match step
  (`web-api-auth/src/auth/authentication.rs:171-176`) misses on a case difference and falls through to
  auto-create.
- **Case-variant registration returns 500** — the pre-insert duplicate check misses, the BINARY unique
  constraint fires, and the handler surfaces a raw unique-violation error instead of 409.

Seven production sites compare against `users.email`; five pass raw strings
(`routes/auth.rs:325` register check, `routes/auth.rs:544` login lookup,
`authentication.rs:173` OIDC match, `routes/oidc_auth.rs:1035` registration eligibility,
`routes/oidc_auth.rs:1611` completion race guard). The two typed sites
(`routes/users.rs:912`, `routes/auth.rs:2627`) wrap via `MaskedEmail::new()`, which does not
canonicalize. `MaskedEmail`'s `#[serde(transparent)]` `Deserialize` bypasses both constructors, and the
SeaORM `TryGetable`/`ValueType` impls route through `new()` — so a canonicalizing `new()` would
silently rewrite DB reads.

## Invariant and scope

**One stored form for `users.email`: trimmed, ASCII-lowercased, full address (local part included).**

Explicitly **out of scope** (recorded in the design authority; restated here as an acceptance item):

- Provider-alias normalization (Gmail dot-collapsing, plus-suffixes) — provider-specific and would
  merge distinct humans.
- Unicode case folding — non-ASCII case variants stay distinct (deliberate; locale hazards).
- IdP email drift handling (`user_oidc_links` columns, drift audit event, OIDC-only change path) —
  lands with M3 per the design authority.
- The email-change confirm flow's missing audit emission — folded into the M3 drift work
  (09 § credential-layer review), not this fix.
- Egress representation — `UserResponse.email` stays `String`; response shapes do not change.

## Ordering constraint (load-bearing)

Comparison-site conversion lands **before or with** the data migration. The reverse order lowercases
the stored owner row while login still compares raw, locking out the only administrator (no
password-reset flow exists). This design satisfies the constraint structurally: the ingress retyping,
the comparison-site conversion, and the migration ship in **one atomic change** (single PR/release);
the migration runs at startup of a binary that already contains the converted code. The plan must not
split the migration into a separate earlier-landing commit.

## Design

### 1. `MaskedEmail` v2 (`crates/shared/types/src/masked_email.rs`)

- **Canonical transform** — a pure associated function, the single source of truth reused by the
  migration and the `db-migrate` copy tool:

  ```rust
  impl MaskedEmail {
      /// Canonical form: trim, then ASCII-lowercase. No validation, no Unicode folding.
      pub fn canonical_form(s: &str) -> String {
          s.trim().to_ascii_lowercase()
      }
  }
  ```

- **`FromStr` canonicalizes and validates**: `canonical_form()` first, then validation on the result —
  exactly one `@`, non-empty local and domain parts (existing checks), plus a **new ≤254-byte cap**
  (moved here from the per-request `Validate` impls; applies post-trim). `ParseMaskedEmailError` gains
  typed variants (`MissingAt`/`MultipleAt`, `EmptyLocal`, `EmptyDomain`, `TooLong`) so 400 bodies still
  say which rule broke — today's `Validate` messages did, and a single opaque `"invalid email address"`
  would also collide byte-for-byte with the unrelated SMTP-plugin validator message.
- **`Deserialize` canonicalizes**: drop `#[serde(transparent)]`; add `#[serde(try_from = "String")]`
  with `TryFrom<String> for MaskedEmail` delegating to `FromStr`. `serde_json::from_str` therefore
  canonicalizes and validates — the actual front door closes.
- **`Serialize` stays plaintext** (derived; a newtype serializes as its inner value in JSON). It must
  NOT emit the masked form: the retyped request structs live in `uptrakit-shared-web-api-types` and are
  serialized by `uptrakit-openapi-client` when the CLI sends requests — a masking `Serialize` would
  corrupt outbound request bodies. Collateral: `docs/development/audit-logs.md:121-123` falsely claims
  `MaskedEmail` has a masked-form `Serialize`; correct the doc to reality (masked `Debug`/`Display`
  only) in this change.
- **The raw constructor goes private.** `pub fn new()` is deleted. A private non-canonicalizing
  constructor (e.g. `fn from_stored(String) -> Self`) serves only the `TryGetable`/`ValueType` impls —
  which live in a child module of the same file, so no wider visibility is needed. Loaded rows must not
  silently diverge from persisted bytes; everything else goes through `FromStr`. The compiler, not a
  grep, enforces canonical ingress. All ~49 external `MaskedEmail::new(...)` call sites (5 production,
  rest test fixtures) convert to `"a@b.c".parse::<MaskedEmail>()` + `?`/`expect` (test modules already
  carry `#![expect(clippy::unwrap_used, clippy::expect_used, ...)]` — established pattern in this file's
  own test module).
- **`impl From<&MaskedEmail> for sea_orm::Value`** (alongside the existing by-value impl) so
  `.eq(&req.email)` compiles — without it the path of least resistance at converted sites is
  `.eq(req.email.expose_email())`, re-introducing exactly the raw comparison this fix removes.
- **OpenAPI**: no `ToSchema` derive. Retyped request fields annotate `#[schema(value_type = String)]`
  (established pattern, e.g. `system_services.rs:25`), keeping `openapi.json` and the generated
  frontend types unchanged in shape.

Unit tests (success + failure paths): `serde_json::from_str` canonicalization
(`" User@Example.COM "` → `user@example.com`), idempotence (`canonical_form(canonical_form(x)) ==
canonical_form(x)`), non-ASCII passthrough (`ÜSER@Example.com` → `ÜSER@example.com` — ASCII folded,
`Ü` preserved), trim behavior, each `ParseMaskedEmailError` variant, length cap measured post-trim,
DB-read fidelity (a `TryGetable`/`ValueType` round-trip of a mixed-case value stays byte-identical).

### 2. Ingress retyping and comparison-site conversion

**Retyped request fields** (`crates/shared/web-api-types`), each `String` → `MaskedEmail` with
`#[schema(value_type = String)]`:

- `LoginRequest.email` (`auth.rs:35`)
- `RegisterRequest.email` (`auth.rs:16`)
- `InitiateEmailChangeRequest.new_email` (`profile.rs:45`)

Their `Validate` impls drop the now-redundant email checks (`contains('@')`, `len() > 254` — both
enforced at parse time). The three existing `Validate` tests asserting `field == "email"`/`"new_email"`
are rewritten as `FromStr` parse tests, not deleted.

**MFA email**: verified vacuous — `MfaEmailRequest` carries only `mfa_token`; the email-MFA flow loads
the delivery address from the user row by `user_id` (`routes/mfa.rs:592`) and performs no email
comparison anywhere. No retyping exists to do; the flow inherits canonical delivery addresses from the
migrated column. The spec records this so the design-authority phrase "MFA email" is discharged
explicitly.

**Handler collateral**: `initiate_email_change` (`routes/users.rs:856`) currently uses a grandfathered
raw `Json<T>` extractor (allowlist row 15 of `ci/verify_no_raw_body_extractors_allowlist.txt`) whose
manual validation returns 422 JSON; with validation moved into `Deserialize`, a raw `Json` rejection
would degrade to axum's `text/plain`. Convert the handler to `Validated<T>` in this change: allowlist
shrinks (ratchet `MAX_ALLOWLIST_ENTRIES` 33 → 32), the failure becomes 400 with the standard
`ErrorResponse` shape, and the `#[utoipa::path]` status at `users.rs:845` updates accordingly.
Login/register already use `Validated<T>`, which collapses serde and `Validate` failures to the same
400 + `ErrorResponse` shape — only message strings change there.

**Query chokepoint**: add `find_by_canonical_email` beside the user entity in `uptrakit-shared-db`
(the `users` table is global, not tenant-scoped; the helper is generic over
`sea_orm::ConnectionTrait` so `resolve_oidc_user` — itself generic over `ConnectionTrait` — can call
it):

```rust
pub async fn find_by_canonical_email<C: ConnectionTrait>(
    db: &C,
    email: &MaskedEmail,
) -> Result<Option<user::Model>, DbErr>
```

All seven production comparison sites convert to it. `OidcUserParams.email` retypes `&'a str` →
`&'a MaskedEmail`; `check_registration_eligibility`'s param retypes likewise; the OIDC claim email is
parsed to `MaskedEmail` once at claim extraction in `routes/oidc_auth.rs` (single choke point), so
`pending_oidc_registrations` rows created after the deploy store canonical values and the completion
path (`oidc_auth.rs:1655`) inserts typed values. Test-only comparison sites convert to the chokepoint
too (they are `.eq("owner@test.local")`-style fixtures; conversion keeps the CI gate allowlist at
zero).

**Frontend** (trim fixes; backend canonicalizes regardless — this is UX consistency):

- `frontend/src/routes/login/+page.svelte` and `register/+page.svelte`: both validate `email.trim()`
  truthiness but submit the untrimmed bound value — trim before send.
- `frontend/src/routes/profile/+page.svelte`: submits `newEmail` raw with no client-side validation —
  trim before send.
- Component tests updated; frontend coverage minimums apply (lines 70%, branches 65%, functions 70%).

### 3. Migration (`crates/shared/db/src/migration/mYYYYMMDD_000001_canonicalize_user_emails.rs`)

`up()`, in order, all inside the runner's wrapping transaction (file-backed SQLite: single-connection
run where an abort rolls back the entire migration run; Postgres: plain transaction):

1. **Delete all rows** from `email_change_requests`, `pending_oidc_registrations`, and
   `pending_account_links` (typed `delete_many`, no filter — every row in these tables is a pending
   short-TTL artifact). `email_change_requests.new_email` is `EncryptedString` ciphertext and cannot
   be normalized in place; the two pending tables store plaintext claim emails captured pre-deploy
   that would otherwise (a) be inserted non-canonically by the completion path and (b) defeat the
   `oidc_auth.rs:1611` race guard. All three are re-requestable within minutes; deletion is the
   correct disposition for each.
2. **Collision pre-check and normalization in Rust, not SQL.** Load `(id, email)` for all users
   (`SELECT` via SeaORM; the table is small — single-digit rows in the only live deployment). Group by
   `MaskedEmail::canonical_form(email)`. If any group has more than one row: **abort loudly** with a
   typed error naming each colliding group as user IDs plus **masked** emails; the runner's transaction
   rolls back the whole run. Never merge. Otherwise, update each row whose stored value differs from
   its canonical form (`update` per changed row — a per-row loop, justified inline: each row receives
   a distinct computed value, the table is tiny, and this runs once at migration time; the N+1 rule
   targets steady-state query paths).
   Normalization applies `canonical_form` only — it never validates or rejects an existing row.
3. **Backstop index**: `CREATE UNIQUE INDEX uix_users_lower_email ON users (lower(email))` via
   `execute_unprepared`, with the inline comment required by the raw-SQL policy (sea_query's
   `Index::create()` cannot express functional indexes — same limitation and comment as the precedent
   `m20260322_000001_hosts_lower_name_index.rs`). The existing BINARY unique constraint on `email`
   stays. Created **after** normalization so it cannot fire on pre-existing case variants.

`down()`: drop the index (typed `Index::drop`, as in the precedent); the data normalization is
documented as an irreversible no-op (lossy by design — original casing is gone). Cover `down()` on
both backends (precedent: `c2910869b`).

**Transient-collision safety of step 2** (documented in the migration): a mid-update collision against
the existing BINARY unique constraint would require some row's new canonical value to equal another
row's current value; if that other row is already canonical the pre-check caught the pair, and if it
is not, a canonical value cannot equal a non-canonical one. The proof holds only because the pre-check
and the update use the **same Rust function** — do not split them across SQL and Rust.

**Index-vs-Rust semantics note** (documented in the migration and the security doc): Postgres
`lower()` is Unicode-aware while the Rust invariant is ASCII-only, so the backstop index is strictly
_stricter_ than the application invariant on Postgres — two stored emails differing only by non-ASCII
case (distinct under the invariant) would collide in the index at insert time and surface as a raw
unique-violation 500. This is a deliberate, documented property of a defense-in-depth backstop, not
the enforcement path; the application-level check (canonical comparison via the chokepoint) is the
enforcement path and returns 409. SQLite `lower()` is ASCII-only and matches the invariant exactly.

**Why Rust-side, not `Query::update()` + `Func::lower`** (deviation from the design text, which the
invariant itself forces): SQL `lower()`/`trim()` are not the invariant's transform. On Postgres,
`lower('Ä@x.com')` = `ä@x.com` — a value Rust ingress will never produce, so the migration would
rewrite a login key into a form the converted comparison sites can never match: precisely the lockout
the ordering constraint exists to prevent, plus false-positive collision aborts on legal
non-ASCII-variant pairs. SQL `trim()` also strips only spaces where Rust `str::trim()` strips all
Unicode whitespace. Bit-identical semantics with ingress are achievable only by running the ingress
function itself. (Additionally, sea_query 1.0 has no `Func::trim`, so the SQL form would have needed a
`Func::cust` escape hatch anyway.)

### 4. `--migrate-and-exit` (pre-flight escape hatch)

Declared in `controller-runtime/src/cli.rs:87-89` and documented in README/CHANGELOG/ADR-0008, but
read by no code path today. Implement it in `async_main` (`lib.rs`) after the `--check-config` block,
mirroring the `DbMigrate` subcommand's tracing init, by **reusing the boot phases verbatim**:
`config::load` → `directories::resolve` → `persistence::open` (which connects and runs migrations,
including the dedicated single-connection SQLite migration pool) → exit 0/1. Reuse, don't reimplement:
a hand-rolled config/DB resolution that diverges from boot (e.g. a different derived SQLite path when
`runtime.db.url` is empty) would migrate a _different_ database, report success, and leave the server
to hit the collision abort anyway — a false all-clear is worse than no escape hatch. Scope is DB
migration only (no master-key validation): boot runs `crypto::verify_and_migrate` _after_
`persistence::open`, so migrations need no key ring, and the documented flag description ("Run
database migrations and exit") stays accurate. A collision abort thereby becomes a diagnosable
pre-flight failure instead of a restart crash-loop. The existing `db-migrate` subcommand
(cross-database copy) is unrelated and untouched by this item.

### 5. `db-migrate` copy tool: close the normalization bypass

`db_migrate::run` migrates the empty target schema first, then `copy_one` streams `E::Model` rows
verbatim — emails pass through the non-canonicalizing DB-read path. Copying a pre-normalization
source (old backup, SQLite→Postgres recovery) would land non-canonical rows in a target whose
migration is already recorded as applied and will never re-run — an unrecoverable lockout for the
affected user. Fix: after the `users` table copy, run the same Rust normalize-with-collision-check
routine as the migration (extract it into a shared helper in `uptrakit-shared-db` called by both);
a collision aborts the copy run loudly before completion is reported.

### 6. CI gate (`ci/verify_email_canonical_ingress.sh`)

Shape (house style of `verify_engine_owned_entities.sh` + the self-check hardening of
`verify_no_raw_body_extractors.sh`; NOT the canary-less audit-gate twins):

- **Rule**: `user::Column::Email` used in a comparison/filter may appear only in the chokepoint file
  (`find_by_canonical_email`'s module). Matching is slurp-mode (`perl -0777` or `rg -U`) so a
  rustfmt-wrapped `.eq(` on its own line (the exact shape at `routes/auth.rs:2627` today) cannot slip
  through a line-anchored pattern.
- **Non-vacuity canary**: assert the pattern still matches the chokepoint file itself; zero matches ⇒
  "gate is stale/broken", exit 1.
- **Allowlist**: target is zero rows (test sites converted to the chokepoint); if any row proves
  necessary, the file carries stale-entry detection and a shrink-only ratchet.
- **Dry-run proof** (ledger requirement): before trusting a clean run, the implementation plan must
  demonstrate the gate failing against a deliberately added raw `Column::Email.eq("x")` line and
  against a multi-line-wrapped variant.
- **Wiring, same commit**: `.github/workflows/ci.yml` `semantic-boundary` job step, `.husky/pre-push`
  echo+run pair, `docs/development/quality-gates.md` (canonical command list), and the `AGENTS.md`
  quick-start block.

The constructor-discipline half of the original gate idea (banning raw construction outside the type)
is deliberately absent: making the raw constructor private (§1) turns that entire violation class into
a compile error, which a grep could only approximate.

## Error handling

- `ParseMaskedEmailError`: typed variants via `thiserror` (existing enum extended); no `unwrap`/
  `panic`; `FromStr` is the only fallible path.
- Migration: typed error via the migration crate's existing error contract; the collision abort
  message lists user IDs + masked emails (see Deviations) and instructs the operator to resolve
  duplicates (delete/merge manually) and re-run via `--migrate-and-exit`.
- HTTP: converted handlers keep the standard `ErrorResponse` shape through `Validated<T>` (400).
  Case-variant registration now hits the canonical pre-insert check → 409; a lost race lands on the
  backstop index → 500, equivalent to today's exact-case race behavior.

## Testing

- **Unit** (`masked_email.rs`): as listed in §1.
- **REST integration** (shared `TestApp` harness, per testing standards):
  - Mixed-case login succeeds against a canonically-stored user.
  - Case-variant registration returns 409 (not 500).
  - **OIDC duplicate auto-create regression test**: mixed-case IdP claim resolves to the existing
    user — no second account (the headline defect).
  - Email-change to a case-variant of an existing address is refused.
- **Migration tests** (`crates/shared/db`, both backends; Postgres via the Docker suite — export
  `DOCKER_HOST` from the active context per the testcontainers pitfall):
  - Mixed-case fixture DB → normalized, index present, pending tables empty.
  - Collision fixture (`A@x.com` + `a@x.com`) → loud abort naming both rows, **whole run rolled
    back, `users` byte-unchanged** — asserted against `run_migrations` (the production runner), not
    `run_migrations_debug`, which lacks the outer transaction on Postgres.
  - `down()` on both backends (index dropped cleanly).
- **`db-migrate` copy tool**: copy of a non-canonical source normalizes the target; collision source
  aborts.
- **OIDC redirect tests** (`oidc_auth.rs:2392`, `:2402`): existing literals use lowercase inputs and
  stay valid; add a mixed-case-input case asserting the redirect carries the canonical
  (percent-encoded) form.
- **Gate**: dry-run failure demonstrations as in §6.
- **Frontend**: trim-before-send covered in component tests.

## Known collateral (accepted, recorded)

- **Audit `actor_display`** keeps historical casing in existing rows — acceptable; audit is a
  historical record, and new events use the canonical form.
- **TOTP `otpauth://` labels** change casing for new enrolments only; the email is not HMAC input —
  existing enrolments keep working.
- **Error-message strings** for invalid emails change (typed parse errors replace `Validate`
  messages); shape and status stay 400 via `Validated<T>` everywhere after the §2 conversion.
- **`initiate_email_change`** responds 400 (was 422) for invalid `new_email`; OpenAPI + regenerated
  client reflect this.
- **`docs/development/audit-logs.md`** masked-`Serialize` claim corrected (§1).

## Deviations from the design-authority mechanism text

The design's _decisions_ (invariant, scope, ordering, deliverable list) are untouched. Four
_mechanism_ details are adjusted, each because following the letter would break the design's own
invariant or a binding repo rule:

1. **Normalization + pre-check in Rust instead of `Query::update()` + `Func::lower`** — Postgres
   `lower()` is Unicode-aware and diverges from the ASCII-only invariant: silent lockout +
   false-positive aborts (§3). The "no raw SQL" intent is honored — the data pass is typed SeaORM;
   the only raw SQL is the functional index, per the design's own cited precedent.
2. **Two additional tables cleared** (`pending_oidc_registrations`, `pending_account_links`) — same
   rationale the design applies to `email_change_requests`; leaving them creates a post-deploy
   non-canonical insert path and defeats a race guard (§3 step 1).
3. **Collision abort names rows as user IDs + masked emails**, not plaintext — the workspace treats
   emails as maskable PII (`MaskedEmail`'s masked `Debug`/`Display` exist precisely to keep them out
   of logs); IDs are sufficient for the operator runbook. "Naming rows" is satisfied; plaintext is
   not required for it.
4. **No public `from_stored`** — the design's non-canonicalizing DB-read constructor exists but is
   private to `masked_email.rs`; the DB-read path uses it, nothing else can. A public `from_stored`
   plus a grep gate is strictly weaker than a compile error, and this repo's own gate history shows
   `#[cfg(test)]`-aware grep gating is unreliable.

## Documentation deliverables

- **New ADR** (via `adrs new "Canonical email form for user email addresses"` — never hand-numbered):
  records the invariant, the ASCII-only decision, the ingress-choke-point + private-constructor
  mechanism, the backstop-index semantics note, and the out-of-scope list. Sections must satisfy
  `adrs doctor` hard-fail (no placeholder tokens).
- `docs/security/auth-and-authorization.md`: short "Canonical email form" subsection (invariant,
  enforcement point, backstop semantics, link to ADR).
- `docs/development/quality-gates.md` + `AGENTS.md` quick-start block: new gate line (same commit as
  the gate).
- `docs/development/audit-logs.md`: correct the `MaskedEmail` masked-`Serialize` claim.
- `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/`: regenerate via
  `./scripts/regen-api.sh` and commit both (schema shapes unchanged thanks to `value_type = String`,
  but the 422→400 status change and description text will drift the spec; CI gates on staleness).
- README `--migrate-and-exit` row and ADR-0008/CHANGELOG mentions: already accurate once the flag
  works — verified, no edits needed. Release CHANGELOG entries come from conventional commits
  (release-plz `git_only`); no hand edits.
- No wire-protocol change → no `asyncapi.yaml` regen. No new dependencies → no `Cargo.toml`
  dependency work beyond possibly a dev-dependency already present for migration tests.

## Acceptance criteria

1. Mixed-case login succeeds (TestApp test).
2. Case-variant registration → 409, not a unique-violation 500.
3. OIDC email match hits across case difference — no duplicate auto-create (regression test).
4. Email-change to a case-variant of an existing address is refused.
5. Migration collision abort is loud, names rows (IDs + masked emails), and is transactional — proven
   against a mixed-case fixture DB on both backends with the production runner.
6. `--migrate-and-exit` performs the same migration the server would, against the same database, and
   exits with a diagnosable status.
7. CI gate rejects raw `Column::Email` comparisons (dry-run-proven) and passes on the converted tree.
8. Explicit statement (this spec + the ADR): provider-alias normalization (gmail dots/plus) is out of
   scope.

## Alternatives considered

- **SQL-side normalization (`Func::lower`, per the design text's letter)** — rejected: Postgres
  Unicode `lower()` breaks the ASCII-only invariant (lockout vector); no `Func::trim` in sea_query 1.0.
- **Public `from_stored` + grep gate on construction** — rejected: preserves the bypass at five
  production sites under a mechanical rename; `#[cfg(test)]`-aware grep gating already failed in this
  repo (`verify_no_raw_body_extractors.sh` documents the abandonment).
- **`CITEXT`/`COLLATE NOCASE` column** — rejected: backend-specific semantics (Postgres `citext` is
  Unicode-folding, SQLite `NOCASE` is ASCII), and the design chose an application-level canonical form.
- **`ToSchema` on `MaskedEmail`** — rejected: mints a named OpenAPI component, churns generated
  frontend types and the golden spec for zero information gain; `value_type = String` is the
  established pattern.
