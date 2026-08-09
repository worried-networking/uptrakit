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

**Rollback is the mirror-image lockout and is not safe**: redeploying the previous release after the
migration has run leaves `users.email` lowercased while the old binary compares raw — same lockout,
opposite direction — and `down()` deliberately cannot restore casing. Phrase the warning as a
**pre-upgrade instruction** — "take a database backup before upgrading; rollback past this release
is only possible by restoring it" — in the ADR, the release commit body, and the security doc; a
restore-the-backup note read after upgrading arrives when the window has already closed. The
migration's per-row info log (§3 step 2) gives the operator the record of what changed.

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
  rest test fixtures) convert to `"a@b.c".parse::<MaskedEmail>()` + `?`/`expect` — `clippy.toml` sets
  `allow-unwrap-in-tests = true` / `allow-expect-in-tests = true`, so `.expect()` inside `#[cfg(test)]`
  modules needs no lint attribute. Do NOT add `#![expect(clippy::unwrap_used, ...)]` to `#[cfg(test)]`
  modules for this: the lints never fire there, and workspace `unfulfilled_lint_expectations = "deny"`
  makes an unfulfilled expectation a build error. (The boundary is the function, not the file:
  `allow-*-in-tests` suppresses only inside `#[test]`-attributed fns — in `#[cfg(test)]` modules and
  `tests/*.rs` binaries alike. A non-`#[test]` fixture/helper fn using `.expect()` needs an explicit
  `#[expect(clippy::expect_used)]` wherever it lives — the existing attributes in
  `crates/ui/mcp/tests/get_current_user_mcp.rs` are that case, not a file-location rule.)
- **`impl From<&MaskedEmail> for sea_orm::Value`** (alongside the existing by-value impl) so
  `.eq(&req.email)` compiles — without it the path of least resistance at converted sites is
  `.eq(req.email.expose_email())`, re-introducing exactly the raw comparison this fix removes.
- **OpenAPI**: no `ToSchema` derive. Retyped request fields annotate `#[schema(value_type = String)]`
  (established pattern, e.g. `system_services.rs:31`), keeping `openapi.json` and the generated
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

**Claim parse failure is a new, named failure mode**: today the extraction
(`oidc_auth.rs:2164`-area) treats only an _absent_ email as fatal (`oidc_no_email` redirect).
Parsing the claim to `MaskedEmail` adds a rejection for malformed/oversized claims — redirect with
its own error code (`oidc_invalid_email`), and record the behavior change: an IdP emitting a
non-conforming email now fails login instead of proceeding to auto-create.

**Email-change confirm becomes fallible**: `routes/auth.rs:2622/2647` decrypts the stored
`new_email` ciphertext back to `String` and (post-retyping) must parse it to set `users.email`. By
construction the value is canonical — every surviving `email_change_requests` row was created from a
typed `MaskedEmail` after the migration (which deleted all pre-deploy rows) — so a parse failure is
an internal-invariant violation: map it to 500 through the handler's existing error contract (no
silent fallback, row retained for diagnosis). Name this arm so the implementer does not improvise
mid-change.

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
   short-TTL artifact; an unfiltered DELETE emits no column list, so this entity use cannot drift
   against replayed schema, unlike the SELECT/UPDATE in step 2). `email_change_requests.new_email` is `EncryptedString` ciphertext and cannot
   be normalized in place; the two pending tables store plaintext claim emails captured pre-deploy
   that would otherwise (a) be inserted non-canonically by the completion path and (b) defeat the
   `oidc_auth.rs:1611` race guard. All three are re-requestable within minutes; deletion is the
   correct disposition for each.
2. **Collision pre-check and normalization in Rust, not SQL — and entity-free.** The migration must
   NOT use `user::Entity`/`user::Model`: a migration referencing the live entity is replayed on fresh
   installs against the schema _as of this migration_, and breaks the moment a later migration adds a
   `users` column the entity now carries (SELECT of a not-yet-existing column → every new install
   fails). Use migration-local `Iden`s + `Query::select()` on exactly `(id, email)` reading raw
   `QueryResult` rows, and `Query::update()` per changed row (the table is small — single-digit rows
   in the only live deployment). Group loaded rows by `MaskedEmail::canonical_form(email)`. If any
   group has more than one row: **abort loudly** with a typed error naming each colliding group as
   user IDs plus **masked** emails; the runner's transaction rolls back the whole run. Never merge.
   Otherwise, update each row whose stored value differs from its canonical form (a per-row loop,
   justified inline: each row receives a distinct computed value, this runs once at migration time;
   the N+1 rule targets steady-state query paths), logging each row at info level as user ID +
   masked before/after in **intent tense** ("normalizing …") plus one post-success summary line —
   the per-row lines are emitted inside the transaction, and past-tense wording would assert a fact
   the rollback can undo while journald keeps the line.
   Normalization applies `canonical_form` only — it never validates or rejects an existing row.
   Additionally, **warn (masked, non-fatal)** for any row whose canonical form fails the new
   `FromStr` validation (e.g. >254 bytes, malformed): such a row survives the migration but no
   ingress path can ever produce a matching parse again, making the account unreachable by
   email-keyed login until an operator fixes the row — silence here would hide a lockout.
   **On Postgres, run a second grouping pass** over the same loaded rows keyed by Unicode
   `str::to_lowercase` and abort (same IDs + masked-emails error) on any group >1: the backstop
   index's Postgres `lower()` is Unicode-aware, so a pair that is distinct under the ASCII invariant
   (`Ä@x` / `ä@x`) passes the canonical pre-check yet kills `CREATE UNIQUE INDEX` in step 3 with a
   raw duplicate-key error naming no rows — the friendly abort must catch it first. Zero extra
   queries; SQLite skips the pass (its `lower()` is ASCII-only and cannot diverge).
   The shared helper splits into two functions: `check_collisions(&C)` (read-only, returns the
   grouped report) and `normalize(&C)` (writes; calls the checker first) — §5's source-side use
   requires the read-only half alone.
3. **Backstop index**: `CREATE UNIQUE INDEX uix_users_lower_email ON users (lower(email))` via
   `execute_unprepared`, single statement on both backends (same shape as the precedent
   `m20260322_000001_hosts_lower_name_index.rs`). The inline comment required by the raw-SQL policy
   must name the limitation **accurately**: sea_query 1.0's `SqliteQueryBuilder` panics on expression
   index columns (`IndexColumn::Expr` hits the trait-default `prepare_index_columns`), while
   `PostgresQueryBuilder` supports them — so the typed builder cannot serve SQLite, and one raw
   statement keeps both backends on a single code path. (The precedent's comment blames sea_query
   generally; do not copy that wording.) The existing BINARY unique constraint on `email` stays.
   Created **after** normalization so it cannot fire on pre-existing case variants.

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

**Why Rust-computed values, not SQL `lower()`/`trim()`** (deviation from the design text, which the
invariant itself forces — the `Query::update()` builder itself is kept; only the SQL-side transform
is rejected): SQL `lower()`/`trim()` are not the invariant's transform. On Postgres,
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

Shape: mirror `DbMigrate`'s dispatch — extract a standalone handler (e.g.
`async fn migrate_and_exit::run(&Args) -> ...` mapped to an `ExitCode` at the call site) that
`async_main` calls, rather than inlining the logic. `async_main` parses `Args` from the real process
environment and is not test-constructible; the extracted fn is what the AC6 test drives.

### 5. `db-migrate` copy tool: close the normalization bypass

`db_migrate::run` migrates the empty target schema first, then `copy_one` streams `E::Model` rows
verbatim — emails pass through the non-canonicalizing DB-read path. Copying a pre-normalization
source (old backup, SQLite→Postgres recovery) would land non-canonical rows in a target whose
migration is already recorded as applied and will never re-run — an unrecoverable lockout for the
affected user. Fix — two parts, because a _post_-copy pass is structurally wrong here:

- **Source-side collision pre-check before any write**: the target schema (migrated first) already
  carries `uix_users_lower_email`, so a colliding source would die inside `insert_many` with a raw
  unique-violation mid-copy — the designed loud abort would never execute. Run the shared
  collision-check against the **source** connection before copying `users`, aborting with the same
  IDs + masked-emails error.
- **Canonicalize during the copy, not after**: `copy_one` batches with no wrapping transaction —
  each `insert_many` commits. A crash between a verbatim `users` copy and a post-copy normalize
  would persist exactly the lockout state this section exists to prevent. Instead, canonicalize each
  user row's email as it is copied, so every committed batch already satisfies the invariant. Two
  mechanics this requires (an implementer dead-ends without them): (a) `MaskedEmail` gains
  `pub fn canonicalized(&self) -> MaskedEmail` — infallible re-canonicalization of an
  already-loaded value via the private constructor; the only public `String → MaskedEmail` route is
  the validating `FromStr`, which would hard-abort a recovery copy on exactly the non-conforming
  rows §3 step 2 deliberately tolerates. (b) `copy_one` is generic over `E: EntityTrait` with no
  per-field hook — the `users` table gets a dedicated copy path (or a per-descriptor row-mapper),
  not a patch inside the generic loop.

Part (a) uses §3's read-only `check_collisions` alone — never `normalize`, which writes and must not
touch a source that may be a read-only backup.

**Feature-gating constraint**: the shared helper is entity-free (§3's requirement carries over — it
operates on `(id, email)` via migration-style `Iden`s/raw rows, never `user::Model`). The two
consumers live behind _independent_ features —
`#[cfg(feature = "migration")] pub mod migration` vs `#[cfg(feature = "db-migrate")] pub mod
migrate_core_tables` — and most workspace crates enable `migration` without `db-migrate` (only
`controller-runtime` enables both). The shared helper must therefore live in a module available under
`#[cfg(any(feature = "migration", feature = "db-migrate"))]` (or unconditionally), never inside
`migrate_core_tables.rs`, or isolated `-p` builds with `migration` alone fail to compile (the
bare-crate clippy sweep in CI exists precisely for this failure class).

### 6. CI gate (`ci/verify_email_canonical_ingress.sh`)

Shape (house style of `verify_engine_owned_entities.sh` + the self-check hardening of
`verify_no_raw_body_extractors.sh`; NOT the canary-less audit-gate twins):

- **Rule**: `user::Column::Email` used in a comparison/filter may appear only in the chokepoint file
  (`find_by_canonical_email`'s module). Matching is slurp-mode (`perl -0777` or `rg -U`) so a
  rustfmt-wrapped `.eq(` on its own line (the exact shape at `routes/auth.rs:2627` today) cannot slip
  through a line-anchored pattern. Anchor the pattern to the `user::` path: bare `Column::Email` also
  names `pending_oidc_registration::Column::Email` and `pending_account_link::Column::Email` (real
  `String` columns, zero comparison sites today). Those two stay ungated — note the hole explicitly
  in the gate's header comment so a future comparison site there is a conscious decision, not an
  accident. Second pattern for the import bypass: a file containing a `use` of `entity::user`
  (bringing `Column` in unqualified) plus a bare `Column::Email` comparison evades the path-anchored
  pattern — match that combination too, or the gate is one `use` statement away from silence.
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
    `run_migrations_debug`, which lacks the outer transaction on Postgres. On SQLite the fixture
    must be a **file-backed** temp DB: `run_migrations` branches on `sqlite_main_db_file()`, and an
    in-memory DB takes the caller-pool FK-ON branch — a different code path from production
    (precedent: `run_migrations_file_sqlite_pool` test, `migration/mod.rs`).
  - `down()` on both backends (index dropped cleanly).
- **`db-migrate` copy tool**: copy of a non-canonical source lands canonical rows in the target
  (canonicalize-during-copy — every committed batch canonical); collision source aborts via the
  source-side pre-check **before any write** reaches the target.
- **`--migrate-and-exit`** (AC6): a test exercising the flag path itself — clean DB → exit 0;
  collision-fixture DB → non-zero exit with the collision message on stderr. No test currently covers
  any controller-runtime CLI dispatch path, so this is new scaffolding; drive the extracted handler
  fn (§4's mandated shape) directly with constructed `Args` — asserting the outcome mapping is the
  point, spawning the full binary is not required.
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
   false-positive aborts (§3). The "no raw SQL" intent is honored — the data pass is typed sea_query
   builders over raw `QueryResult` rows (entity-free by §3's replay rule);
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
  mechanism, the backstop-index semantics note, the **pre-upgrade backup instruction** ("take a
  database backup before upgrading; rollback past this release is only possible by restoring it" —
  also carried in the security-doc subsection), and the out-of-scope list.
  Sections must satisfy `adrs doctor` hard-fail (no placeholder tokens). The release commit body
  carries the same rollback note (release-plz `git_only` surfaces commit bodies in the changelog).
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
6. `--migrate-and-exit` reuses the boot phases (config/directories/persistence) so it cannot resolve
   a different database than the server, and exits with a diagnosable status (0 clean / non-zero +
   collision message). "Same database" is guaranteed by construction (shared code path), asserted in
   the test by driving the extracted handler — not re-proven end-to-end.
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
