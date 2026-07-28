# M1.1 — `access` Types Core (`uptrakit-shared-types::access`)

Date: 2026-07-28. Status: approved design, pending plan.

First task of the authn/authz refactoring Milestone 1 (source of truth:
`.superpowers/authn-and-authz-refactoring/`, esp. `05-action-model.md`, `06-grant-model.md`,
`09-resolved-questions.md` §Action model/§Grant model, `11-task-breakdown.md` §M1.1,
`12-test-plan.md` §A). Those documents are settled by owner rounds (2026-07-21) — this spec applies
them; it does not reopen them. Prerequisite verified in-tree: the surfaces ID naming-convention spec
including its plugin-type-id amendment has landed (`crates/shared/types/src/plugin_type_id.rs`
already carries dot-kebab IDs such as `package-manager.apt`).

## Problem / goal

Introduce the typed action vocabulary that replaces the flat `Permission` enum: `Action` =
`resource:verb` over a code-defined built-in catalog plus two runtime namespaces (`plugin.*`,
`surface.*`), grant-pattern types with wildcard matching, `Selector` (all variants typed), the
validity matrix with selector-support levels, and bounded-size validation — all in a new
`uptrakit-shared-types::access` module with **zero consumers**. Later M1 tasks (storage, engine,
extractors) build on these types; `permissions.rs` stays untouched until M1.8.

## Decisions locked during grilling (owner, 2026-07-28)

1. **Matrix rejection at parse time** for built-in resources: `"hosts:approve".parse::<Action>()`
   fails with a typed error — invalid built-in pairs are unrepresentable. Dynamic resources accept
   any closed-set verb at parse; registry narrowing is decision-time (M1.3+), per
   `05-action-model.md` §Dynamic namespaces.
2. **Dynamic `Resource` variants store the full resource string** (`"plugin.package-manager.apt"`),
   so `as_str()` returns `&str` for every variant (`&'static` for built-ins); remainders exposed via
   `strip_prefix`-based accessors (no `string_slice` lint exposure). Mirrors `PluginTypeId`'s
   stored-canonical-string shape.
3. **`TargetRef` is not part of M1.1** — the type lands with the engine's
   `authorize(…, target: Option<&TargetRef>)` signature in M1.3
   (`07-decision-and-enforcement.md` §The PDP interface); M2.1 adds the `HostSoftwareItem`
   variant. Keeps M1.1 exactly at the written task scope ("no consumer changes").

## Scope

In: module `access` (types, catalog macro, parsing, pattern matching, selector, bounds, tests) in
`uptrakit-shared-types`. Out (deferred to the named tasks): DB entities and seed roles (M1.2),
`AccessEngine`/`TargetRef`/wire invalidation (M1.3), extractor macro + OpenAPI security schemes
(M1.4a), grant write-path validation incl. the non-`All` phase gate — test-plan row B9 — and the
`Validate`-trait wiring on request types (M1.2/M1.6a), catalog REST endpoint (M1.6b), canonical doc
rewrites + ADR (M1.9), non-`All` selector acceptance (M2.1).

## Module layout

```text
crates/shared/types/src/access/
├── mod.rs        # module doc (model summary, grammar, pointer to design docs), re-exports
├── verb.rs       # Verb, ParseVerbError
├── catalog.rs    # catalog macro invocation → Resource, CatalogEntry, CATALOG, actions:: consts
├── action.rs     # Action, ParseActionError, serde + utoipa impls
├── pattern.rs    # ActionPattern, ResourcePattern, VerbPattern, ParsePatternError, matching
├── selector.rs   # Selector, SelectorSupport, SelectorValidationError
└── bounds.rs     # the six bound constants + validate_patterns helper
```

`lib.rs` gains `pub mod access;`. The crate is a leaf types crate: errors are plain `thiserror`
enums (the `FromStr`/`Parse{Type}Error` convention of `docs/development/coding-standards.md`), not
`rootcause` Reports — the established crate-wide pattern, not a new carve-out: every existing
`FromStr` in this crate does the same (`ParseAccessPresetError`, `ParseBatchStatusError`,
`ParsePluginRoleError`, …) and the crate has no `rootcause` dependency; the thiserror+rootcause
rule governs fallible operation boundaries (`docs/development/error-handling.md`), not leaf parse
errors. All utoipa impls sit behind the existing `openapi` feature, exactly like
`permissions.rs`. Manifest change: add `uuid = { workspace = true, features = ["serde"] }` to `[dependencies]` — the
root workspace entry is bare (`uuid = { version = "1" }`), and `Selector`'s derived
`Serialize`/`Deserialize` over `Vec<Uuid>` needs the uuid crate's own `serde` feature (utoipa's
`uuid` feature covers only `ToSchema`); every sibling crate putting `Uuid` in serde-derived types
declares it locally the same way (precedent: `crates/shared/web-api-types/Cargo.toml`,
`uuid = { workspace = true, features = ["serde"] }`). The workspace `utoipa` entry already enables
its `uuid` feature (`utoipa = { version = "5.5", features = ["uuid"] }`), so the `ToSchema` derive
needs no utoipa edits.

## Identifier grammar (one grammar, everywhere)

```text
action    = resource ":" verb
resource  = segment *( "." segment )
segment   = kebab identifier: ^[a-z][a-z0-9]*(-[a-z0-9]+)*$
verb      = "read" | "create" | "update" | "delete" | "trigger"
          | "approve" | "reject" | "manage" | "use"
```

Implemented as one hand-rolled char-level helper (`is_valid_segment` / `is_valid_segment_path`) —
no regex dependency. Built-in resources parse by exact catalog lookup (their strings satisfy the
grammar by construction; a test asserts it). Dynamic remainders (`plugin.` / `surface.` suffixes)
and pattern subtree stems run through the same helper. `"plugin"`/`"surface"` without a dot are not
catalog entries and not dynamic — unknown-resource error; `"plugin."` has an empty remainder —
structure error.

## Components

### `Verb` (verb.rs)

Closed nine-variant enum: `Read, Create, Update, Delete, Trigger, Approve, Reject, Manage, Use`.
`Copy, Clone, PartialEq, Eq, Hash, Debug`, `#[cfg_attr(test, derive(strum::EnumIter))]` (the
coding-standards required pattern — iteration is test-only). `as_str() -> &'static str`,
`Display`, `FromStr` with `ParseVerbError` (thiserror). Production code needing the closed set
(dynamic resources' `allowed_verbs`) uses a `pub const ALL: &[Verb]` slice, guarded by an
EnumIter test asserting `Verb::iter()` equals `ALL` (a hand-maintained array alone would silently
skip a new variant — coding-standards §Exhaustive Enum Test Coverage). No serde of its own —
verbs cross the wire only inside action/pattern strings. Adding a verb is an architecture decision
(`05-action-model.md`); the enum carries a doc comment saying so, and is **deliberately
exhaustive** (no `#[non_exhaustive]`): a new verb must break every match site, per the
coding-standards "explicitly guaranteed to be closed" carve-out.

### Catalog macro → `Resource`, matrix, metadata (catalog.rs)

One **private** `macro_rules!` invocation is the single source of truth. One row per built-in
resource: variant name, wire string, allowed verbs each with its per-action description,
`selector_support` level. From that single invocation the macro emits:

- `Resource` enum: one unit variant per row plus `Plugin(String)` and `Surface(String)` (full
  resource string stored, decision 2). Derives `Clone, PartialEq, Eq, Hash, Debug`;
  `#[non_exhaustive]` (evolving cross-crate enum — every new domain adds a variant; external
  matches carry a wildcard arm, and consumers mostly go through the catalog/`as_str` anyway).
  The `Plugin`/`Surface` **variants** additionally carry variant-level `#[non_exhaustive]`,
  sealing external construction: a consumer-built `Resource::Plugin("hosts".into())` would bypass
  the prefix + grammar validation and reopen at the `Resource` layer the invariant hole `Action`'s
  `pub(crate)` fields close. Validated construction goes through `FromStr` or the checked helpers
  `Resource::plugin(plugin_type: &str)` / `Resource::surface(surface_id: &str)` (grammar-check the
  remainder, compose the prefixed full string, return `Result<Self, ParseResourceError>`); reads
  go through `as_str()`/`plugin_type()`/`surface_id()` (external `match` uses `Plugin(..)` rest
  patterns — field binding is variant-sealed too).
- `Resource::as_str(&self) -> &str` — `&'static` literals for built-ins, stored string for dynamic.
- `Resource::allowed_verbs(&self) -> &'static [Verb]` — matrix row for built-ins; for dynamic
  variants the full closed set (registry narrowing is decision-time).
- `Action::selector_support(&self) -> SelectorSupport` — **per action, not per resource**:
  `05-action-model.md` ("every action's entry carries `selector_support`") and `06-grant-model.md`
  ("per-action `selector_support` rides the catalog API") both specify per-(resource, verb)
  granularity, so the level lives on `VerbEntry`, not `CatalogEntry` — today every
  selector-capable resource happens to be uniform across its verbs, but a per-resource field could
  never express a mixed row and would force an M1.6b surface change. Dynamic actions are
  `SelectorSupport::None` (only the five catalog actions are selector-capable).
- `Resource::is_system(&self) -> bool` — derived from the wire string having the `system.` prefix
  (dynamic variants are never system).
- `Resource::plugin_type(&self) -> Option<&str>` / `surface_id(&self) -> Option<&str>` — remainder
  accessors via `strip_prefix("plugin.")` / `strip_prefix("surface.")`.
- `FromStr for Resource` with its own `ParseResourceError` (the `Parse{TypeName}Error` naming
  convention, coding-standards §Error type naming), embedded into `ParseActionError` via
  `#[from]`: exact catalog lookup; else `plugin.`/`surface.` prefix → dynamic variant with
  remainder validated against the segment grammar; else error.
- `pub const CATALOG: &[CatalogEntry]` where
  `CatalogEntry { resource: Resource, resource_str: &'static str, verbs: &'static [VerbEntry] }`
  and `VerbEntry { verb: Verb, action_str: &'static str, description: &'static str, selector_support: SelectorSupport }`
  (support is per-action metadata — see `Action::selector_support` above) —
  `action_str` built with `concat!` at macro-expansion time. Both structs are `#[non_exhaustive]`
  with pub fields (coding-standards §`#[non_exhaustive]` on Public Structs — catalog rows will
  gain metadata fields; in-crate macro construction is unaffected). **Documented exception** to
  that section's "must expose a constructor" requirement: catalog rows are static data the macro
  alone emits — external construction is deliberately impossible (a foreign `CatalogEntry` would
  be a fake catalog row), so no constructor/`Default` is provided; consumers only read fields. Drives the exhaustive tests here, the
  M1.4a scope dictionary + extractor-consistency CI script, and the M1.6b catalog endpoint.
- `pub mod actions`: per valid built-in (resource, verb) pair, **both** a `pub const` typed
  `Action` (`HOSTS_READ`, `UPDATES_TRIGGER`, `ACCESS_MANAGE`, …; `Action` is const-constructible
  for unit resource variants) **and** the paired `pub const … : &'static str` action string
  (`HOSTS_READ_STR`, …; same `concat!` output as the `CATALOG` row) — both from the same macro
  row. The string consts exist because M1.4a's OpenAPI security declarations and CI scope
  dictionary need compile-time `&'static str` values (attribute positions cannot format a runtime
  `Display`); hand-written scope literals at route sites would be exactly the drift this catalog
  exists to prevent. M1.4a verify note: a `const &'static str` path satisfies macro-expansion
  consumption (the action-extractor macro emitting the security declaration), **not** a
  literal-only attribute slot — M1.4a must route scope emission through its macro, or the consts
  don't discharge the drift risk. Seed grants (M1.2) and extractors (M1.4a) reference these,
  compile-checked
  (`09-resolved-questions.md` §Action model, catalog-generation resolution).

Macro internals (exact matcher syntax, helper arms mapping verb tokens to literals) are
implementer's discretion; the emitted surface above is binding.

**Built-in catalog rows** — the normative source is the mapping table in
`05-action-model.md` §Built-in catalog (re-check it at plan time; the current `Permission` enum was
verified 2026-07-28 to contain no variants beyond that table). Restated for the implementer,
`resource → verbs [selector_support if not none]` (the support column applies to each listed verb
— uniform per resource today, but stored per action on `VerbEntry`):

| Resource | Verbs | Selector support |
| --- | --- | --- |
| `services` | read, approve, reject, delete, update | — |
| `system.services` | read, approve, reject, delete, update | — |
| `software` | read, create, update, delete | — |
| `checks` | trigger | host_and_software |
| `updates` | trigger | host_and_software |
| `scheduler` | manage | — |
| `hosts` | read, update, delete | host |
| `hosts.tags` | manage | — (never selector-capable) |
| `settings` | read | — |
| `settings.auth` | manage | — |
| `settings.enrollment-tokens` | manage | — |
| `settings.certificates` | manage | — |
| `system.settings` | manage | — |
| `commands` | manage | — |
| `plugin-configs` | trigger | — |
| `notifications` | read, manage | — |
| `audit` | read | — |
| `system.audit` | read | — |
| `users` | manage | — |
| `access` | manage | — |
| `discovery.ignores` | manage | — |
| `mcp` | use | — |
| `system.config-state` | read, manage | — |

Descriptions carry over from `Permission::description()` where a 1:1 predecessor exists; the new
actions take theirs from `05-action-model.md` (`hosts.tags:manage` — tag CRUD **and**
assignment, security-relevant; `access:manage` — grant/role CRUD, role assignment;
`commands:manage` keeps its code-execution-authority warning text verbatim).

### `Action` (action.rs)

```rust
pub struct Action { pub(crate) resource: Resource, pub(crate) verb: Verb }
```

`Clone, PartialEq, Eq, Hash, Debug`; `Display` emitting `resource:verb`. **Fields are
`pub(crate)`, not `pub`** (deviation from the `05-action-model.md` sketch's pub fields — the
sketch is explicitly non-binding while its invariants are binding): a public-field struct would
let any consumer build `Action { resource: Hosts, verb: Approve }` by literal, bypassing the
parse-time matrix rejection (decision 1) — the "invalid pairs cannot exist even transiently"
invariant must hold for construction, not just parsing. `pub(crate)` (not fully private) because
the sibling `catalog.rs` module's macro emits the `actions::` consts as struct literals
(matrix-valid by construction from catalog rows). External surface: `resource()`/`verb()`
accessors plus fallible `Action::new(resource: Resource, verb: Verb) -> Result<Self,
ParseActionError>` (matrix-checked, same rules as parse step 5). Crate-restricted fields also make
`#[non_exhaustive]` unnecessary here. **No `Other` catch-all anywhere in the module** — unknown strings are parse
errors, and a parse error is a deny (`05-action-model.md` §Rust representation).

`FromStr` with `ParseActionError` (thiserror), checks in order:

1. Length: `len() > bounds::MAX_PATTERN_LEN` → `TooLong { max }` (before any parsing — no
   truncation; test A9).
2. Structure: exactly one `:` with non-empty sides → `Structure` (bare resource, bare verb,
   multiple colons; tests A8).
3. Verb: closed-set lookup → `UnknownVerb` (A5).
4. Resource: catalog lookup; else dynamic-prefix path with remainder grammar check →
   `UnknownResource` (A4), `InvalidSegment` (charset/empty-segment violations; A7, A8's `a..b`),
   `EmptyDynamicRemainder` (A8).
5. Matrix (built-ins only, decision 1): verb ∉ `allowed_verbs` → `InvalidPair` (A6).

Error variants may be merged/renamed at implementation as long as each numbered check has a
distinguishable variant (the table tests assert variants, not messages).

Serde: hand-written, adapting the `permissions.rs` idiom (`serialize_str(self.as_str())` there) —
`Action` has no stored canonical string, so `Serialize` uses `serializer.collect_str(self)` (the
`Display` form) instead; `Deserialize` via `String` → `parse()` → `serde::de::Error::custom` on
failure. Fail-closed: deserialization of an invalid action string is an error, never a fallback.

OpenAPI (behind `openapi`): manual `PartialSchema`/`ToSchema` (named `Action`) as an **open string
schema** — `Type::String` with the grammar and built-in catalog documented in the schema
`description` (sourced from `CATALOG`, never a hand-maintained list). Deliberately **not**
`enum_values` (divergence from the `Permission` schema treatment, mandated by `05-action-model.md`:
a closed enum would reject every dynamic action; the generated frontend type becomes a branded
string in M1.7).

### Grant patterns (pattern.rs)

```rust
pub struct ActionPattern { pub(crate) resource: ResourcePattern, pub(crate) verb: VerbPattern }
pub enum ResourcePattern {
    Any,             // "*"        — every tenant-plane resource, incl. dynamic; never system.*
    Exact(String),   // "hosts", "settings.auth", "plugin.package-manager.apt"
    Subtree(String), // stem of "settings.*", "plugin.package-manager.*", "system.*"
}
pub enum VerbPattern { Any, Exact(Verb) }
```

`FromStr` + `ParsePatternError`; `Display`; hand-written string serde (patterns are stored as JSON
string arrays in `access_grants.patterns`, M1.2). Same ≤ `MAX_PATTERN_LEN` bound, checked first.
**Pattern grammar is strict** — `*` is legal in exactly two resource positions and one verb
position, nothing else:

```text
resource-pattern = "*"                      ; whole-resource wildcard
                 | segment-path             ; exact (grammar only; catalog membership NOT required)
                 | segment-path ".*"        ; subtree (stem satisfies segment-path grammar)
verb-pattern     = "*" | verb
```

Mid-path stars (`a.*.b`, `plugin.*.apt`), glued stars (`*x`, `x*`, `**`), a bare `.*` (empty
stem), and any charset violation in a stem segment are `ParsePatternError` — the pattern grammar
is the authority-granting surface, so its rejection set gets the same table-test rigor as the
action grammar (negative rows mirroring A7/A8: `*.foo`, `a.*.b`, `*x`, `**`, `.*`,
`settings.*.auth`). Both `Exact` strings and `Subtree` stems are validated against the
segment-path **grammar only** — `Exact` does **not** require catalog membership (it stores the
validated string, and matching is string equality against `action.resource.as_str()`).

Parse is deliberately **grammar-only on the resource side** — catalog membership and matchability
are a single write-time check (`can_match_any`), never folded into `FromStr`: stored grant
patterns outlive catalog changes (M1.3 re-parses them at every resolution), and a `FromStr` that
consulted the catalog — for the matrix *or* for resource existence — would turn a catalog
evolution (resource rename/removal) into a parse failure that poisons whole stored grant rows.
The verb side *is* membership-checked at parse: the verb set is compiled-in closed vocabulary,
not an evolving catalog, so an unknown verb is a grammar error, never a data-rot hazard. Matching
against concrete actions (below) is the decision-time truth; `can_match_any` is write-time advice
only — the engine never calls it during resolution. `ActionPattern` fields are `pub(crate)` with `resource()`/`verb()` accessors and an
infallible `new(ResourcePattern, VerbPattern)` (per-pattern validity is the separate
`can_match_any` check below, so construction stays infallible); `ResourcePattern`/`VerbPattern`
are `#[non_exhaustive]` (evolving cross-crate enums — the coding-standards default; variant
construction from consumer crates still works, matches gain wildcard arms).

**Write-time validity** (validation rule 1 of `06-grant-model.md` §Validation summary; tests A14):
a pattern is rejected iff it is **provably unmatchable** against the static matrix *and* cannot
reach a dynamic namespace. The traversal primitive is
`pattern.matched_catalog_actions() -> impl Iterator<Item = (&CatalogEntry, &VerbEntry)>` — every
catalog `(resource, allowed verb)` pair run through the real `matches` predicate —
and `can_match_any()` is defined over it: non-empty iterator, **or** the resource side can reach
`plugin.*`/`surface.*` (i.e. `Any`, a dynamic `Exact`, or a `Subtree` whose stem is
`plugin`/`surface` or extends one at a segment boundary) — dynamic verbs are registry-declared, so
any closed-set verb is accepted there and the reality check stays decision-time. Exposing the
iterator (not just the boolean) is deliberate: M1.2's validation rule 3 ("non-`All` selector ⇒
every possible match is selector-capable") needs the matched **set** to test each entry's
`selector_support` — without it M1.2 would re-implement this exact traversal and the two copies
could drift. **Rule-3 reuse contract** (binding on M1.2/M2.1, stated now so the vacuous branch
never ships): the iterator ranges over the built-in catalog only, so rule 3 is *two* conditions —
every yielded `VerbEntry.selector_support` admits the grant's selector, **and** the pattern does
not reach a dynamic namespace (dynamic actions are `SelectorSupport::None`; a pattern like
`plugin.*:trigger` with a `Hosts{…}` selector must reject even though the iterator is empty —
an empty iterator is not a pass). Examples: `hosts:approve` and `settings.*:trigger` reject;
`plugin.foo:approve` and `*:approve` accept.

**Matching** — `ActionPattern::matches(&self, action: &Action) -> bool`:

- `Any` resource: matches every resource with `is_system() == false` — the `system.` namespace is
  reachable only by patterns whose resource side literally starts with `system` (Exact or Subtree;
  tests A12) — and **includes** the dynamic namespaces (A13).
- `Exact`: string equality on `as_str()`.
- `Subtree(stem)`: matches resources whose string equals `stem + "." + <non-empty suffix>` —
  segment-boundary prefix matching (`stem.len()`-then-`.` check via iterator/`strip_prefix`, never
  bare `starts_with` — `plugin.package-manager.*` must not match a hypothetical
  `plugin.package-managerx.foo`). A subtree never matches its own root (`settings.*` ∌ `settings`;
  A11). `Subtree("system")` matches the `system.` plane — that is the sanctioned explicit form.
- Verb side: `Any` or equality.

Scope-string note (for M1.4a/M3): pattern syntax is a superset of action syntax and both are valid
RFC 6749 scope tokens; test A15 asserts the charset property over every `CATALOG` action string.

### `Selector` + support levels (selector.rs)

```rust
#[derive(..., Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Selector {
    All,
    Tags { ids: Vec<Uuid> },
    Hosts { ids: Vec<Uuid> },
    Software { ids: Vec<Uuid> },
    Items { ids: Vec<Uuid> },
}
```

**Derived** tagged serde — it emits exactly the storage JSON of `06-grant-model.md` §Storage schema
(`{"type":"all"}`, `{"type":"tags","ids":[…]}`, uniform `ids` field); hand-writing serde here would
reinvent the derive. Golden-shape tests pin the JSON (both directions) so a rename/attr regression
fails loud. Serde's internally-tagged representation cannot combine with `deny_unknown_fields`
(documented serde limitation), so unknown keys in a selector object are ignored on deserialize —
accepted explicitly: selectors do transit the user boundary (M1.6a grant-creation bodies), but an
ignored extra key can never *broaden* authority (selectors only narrow, and the known fields are
what grants match on), while a missing or mistyped required field (`idz` for `ids`) still fails
with a missing-field error. `ToSchema` derive behind `openapi`. Both `Selector` and `SelectorSupport` are
`#[non_exhaustive]` (coding-standards default for new public cross-crate enums; consumer variant
construction unaffected). The same default applies to every error enum in the module,
individually: `ParseVerbError`, `ParseResourceError`, `ParseActionError`, `ParsePatternError`,
`SelectorValidationError`, `PatternSetError` — all `#[non_exhaustive]` (they gain variants as
checks are added; older in-crate parse errors like `ParsePluginRoleError` predate the
non-exhaustive-by-default rule and are not the precedent to copy).

```rust
pub enum SelectorSupport { None, Host, HostAndSoftware }
```

Serde `snake_case` (`"none" | "host" | "host_and_software"`), `Copy`, ordered semantics documented
(each level admits the previous levels' selectors). `SelectorSupport::admits(&self, &Selector) ->
bool`: `All` always; `Tags`/`Hosts` require ≥ `Host`; `Software`/`Items` require
`HostAndSoftware`. This is the reusable predicate for validation rule 3 (M1.2 write path, M2.1) and
the published catalog metadata (M1.6b). The M1-phase "reject any non-`All` selector" gate (test row
B9) is write-path policy and lives in M1.2's grant validation, **not** in these types.

`Selector::validate(&self) -> Result<(), SelectorValidationError>` — inherent, enforcing the ID
count bounds below (typed error naming the exceeded bound). Note the crate-layering constraint: the
`Validate` **trait** lives in `uptrakit-web-api-types::validation` (which depends on this crate, not
vice versa), so M1.1 ships inherent fns + constants; M1.6a's request types implement the trait by
delegating. This is the scoped reading of the task bullet "bounded-size `Validate` impls".

### Bounds (bounds.rs)

The resolved constants (`09-resolved-questions.md` §Grant model, bounded-size resolution), named
approximately:

```rust
pub const MAX_PATTERNS_PER_GRANT: usize = 16;
pub const MAX_PATTERN_LEN: usize = 64;          // also the Action parse bound (test A9)
pub const MAX_GRANT_DESCRIPTION_LEN: usize = 500;
pub const MAX_SELECTOR_TAG_IDS: usize = 32;
pub const MAX_SELECTOR_HOST_IDS: usize = 100;
pub const MAX_SELECTOR_SOFTWARE_IDS: usize = 100;
pub const MAX_SELECTOR_ITEM_IDS: usize = 100;
pub const MAX_GRANTS_PER_SUBJECT: usize = 200;
```

Plus `pub fn validate_patterns(&[ActionPattern]) -> Result<(), PatternSetError>` (count bound +
per-pattern `can_match_any`) for M1.2/M1.6a to call. `MAX_GRANT_DESCRIPTION_LEN` and
`MAX_GRANTS_PER_SUBJECT` are consumed by later tasks; they live here so every access bound has one
home.

**Known cross-subsystem coupling** (flagged, not resolved here): `MAX_PATTERN_LEN = 64` also
bounds action strings, while surface IDs allow up to 128 bytes
(`uptrakit-surfaces::ids::MAX_IDENTIFIER_LEN`) — a permission-gated surface whose ID exceeds
64 − len("surface.") − len(":" + verb) — verb-dependent, worst case 48 chars for
`:trigger`/`:approve` — produces a `required_action` that cannot parse. The bound is owner-resolved (`09-resolved-questions.md` §Grant model) and stays;
M1.5's admission parse must reject such registrations with an error naming the length budget so
the failure is diagnosable, and real first-party surface IDs today are far below the budget.
Carried as a residual note for M1.5.

## Test plan (crate-local, table tests)

Rows A1–A15 of `12-test-plan.md` §A verbatim, plus module-local additions. Anti-vacuity rules
applied (project mistakes ledger): the exhaustive tests iterate the **production** `CATALOG` /
`strum` iterators and first assert the collection is non-empty and contains known members
(`actions::HOSTS_READ`, `actions::UPDATES_TRIGGER`), so a stripped/empty catalog fails loud;
serde-shape tests assert positive content (exact JSON), not just round-trip.

- A1: every `CATALOG` action string parses and round-trips `FromStr` → `Display`/`as_str`; also
  every `actions::` const equals its parsed string form.
- A2/A3: dynamic + grammar-edge positives (`plugin.package-manager.apt:manage`,
  `surface.ssh-agent.hosts:use`, single segment, multi-dot, digits, single-char segments,
  hyphenated + dotted surface IDs).
- A4–A9: negatives per the parse-check ordering above (unknown resource, unknown verb, matrix
  pair, charset incl. underscore/uppercase/leading digit/hyphen edges, structure incl. `a..b` and
  empty dynamic remainder, over-length exactly at `MAX_PATTERN_LEN + 1` with at-bound acceptance).
- A10–A14: pattern parse positives; subtree-∌-root; `system.` exclusion for `*`/`*:*` and
  `Subtree("system")` inclusion; `*` matches dynamic; unmatchable-pattern rejection
  (`hosts:approve`, `settings.*:trigger`) vs dynamic-reachable acceptance (`plugin.foo:approve`).
- Pattern-grammar negatives (mirroring A7/A8 on the pattern side): `*.foo`, `a.*.b`, `*x`, `x*`,
  `**`, bare `.*`, `settings.*.auth`, charset-violating stems — all `ParsePatternError`.
- Parse/validate split pin: a grammar-valid non-catalog `Exact` pattern (`unknownthing:read`)
  **parses** successfully, then `can_match_any` rejects it at write time — locks in that pattern
  `FromStr` never consults the catalog on the resource side.
- Dynamic-resource construction seam: `Resource::plugin`/`Resource::surface` reject
  grammar-violating remainders and empty strings; accepted values round-trip through
  `as_str()`/`plugin_type()`/`surface_id()`, asserting the exact composed string
  (`Resource::plugin("package-manager.apt").as_str() == "plugin.package-manager.apt"`) — the
  constructors take the **remainder**, never a pre-prefixed string (doc-comment states it; a
  double-prefixed input composes to a garbage-but-grammatical dynamic id that write-time
  `can_match_any` still passes — dynamic-reachable — and only the decision-time registry check
  renders inert: fail-safe, matches nothing, never broadens).
- A15: RFC 6749 scope-token charset property over all `CATALOG` action strings.
- Selector: golden JSON per variant (serialize + deserialize), unknown `type` tag rejected,
  `admits` full 3×5 matrix, `validate` at-bound pass / over-bound fail per variant.
- Verb: strum-exhaustive `as_str` ↔ `FromStr` round-trip (per
  `docs/development/coding-standards.md` exhaustive-enum coverage); `Verb::ALL` equals
  `Verb::iter()` (guards the hand-maintained closed-set slice against a silently-added variant).
- Constructor path: `Action::new` rejects a matrix-invalid built-in pair (A6's condition through
  the construction seam, not just `FromStr`) and accepts dynamic resources with any closed-set
  verb.
- Catalog invariants: every built-in resource string satisfies the segment-path grammar; the five
  selector-capable actions are exactly `hosts:read|update|delete` (Host) +
  `checks:trigger`/`updates:trigger` (HostAndSoftware); `hosts.tags` support is `None`; `system.`
  rows report `is_system()`.
- Segment-boundary matching: `Subtree("plugin.package-manager")` matches
  `plugin.package-manager.apt:manage` but not a `plugin.package-managerx…` action.

## Verification gates

Feature worlds get **clippy and test paired** (a world covered only by clippy never runs its
tests). Crate-scoped lanes (both verified green at baseline 2026-07-28), then the canonical
workspace lanes from `docs/development/quality-gates.md`:

```sh
cargo fmt --all
# crate-scoped: default world + openapi world
cargo clippy --all-targets -p uptrakit-shared-types
cargo test  -p uptrakit-shared-types
cargo clippy --all-targets -p uptrakit-shared-types --features openapi
cargo test  -p uptrakit-shared-types --features openapi
# canonical workspace lanes (whole-workspace, not crate checks)
cargo check --no-default-features --features db-sqlite
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo check --all-features            # needs frontend/build (embed-frontend)
cargo clippy --all-targets --all-features
cargo test --all-features
```

The `--all-features` check/clippy lanes must be run explicitly: `.husky/pre-push` runs only the
db-sqlite lane plus `--all-features` **doctests**, so the full feature union (`openapi`,
`schema`, `sea-orm`, `http-ssrf`, `test-support` interacting) is exercised nowhere else. The `access`
module itself is feature-gated only on `openapi`; it deliberately adds **no** `schema`/`JsonSchema`
derives — its types cross no wire in M1 (the M1.3 `AccessInvalidated` payload carries UUID vectors,
not these types) — so the `schema` world compiles it unchanged. Also run `cargo deny check` (the
manifest gains the uuid `serde` feature — trivially clean, but the gate is mandated for manifest
edits) and expect `bash ci/verify_no_new_cfg_not_feature.sh` to pass untouched (the module adds
only positive `#[cfg(feature = "openapi")]` gates; the script targets negated-feature cfg). No
OpenAPI regen (no endpoint changes), no asyncapi regen, no audit-catalog entries (no
state-changing sites), no migration.
Commit scope: copy the crate's existing Conventional-Commit scope from
`git log --oneline -- crates/shared/types` at plan time (do not guess from the directory name).

## Documentation deliverables

- Rustdoc on every public item; `access/mod.rs` module doc summarizes the model (grammar, closed
  verb set, no-`Other` rationale, `system.` rule, selector-support levels) and points at
  `.superpowers/authn-and-authz-refactoring/05-action-model.md`.
- **No canonical doc/ADR/CONTEXT.md updates in M1.1 — deliberate deferral, not an oversight**: the
  module has zero consumers and no observable behavior until M1.2+; the milestone plan assigns the
  `docs/security/auth-and-authorization.md` + `docs/api/*` + `AGENTS.md` rewrites and the
  replacement ADR to M1.9, and the vocabulary enters CONTEXT.md there. This spec file is the M1.1
  design record.

## Alternatives considered

- **Separate `validate()` instead of parse-time matrix rejection** — rejected (grilling decision
  1): transiently-valid invalid actions + a second call every consumer must remember; A6 expects a
  parse error.
- **Dynamic variants storing remainder only** — rejected (decision 2): `as_str()` could not return
  `&str` for dynamic resources; every serialize would compose/allocate.
- **`wire_safe_enum!` for `Verb`/`Action`** — rejected: that macro exists to provide an
  `Other(String)` wire catch-all, which this model deliberately bans (parse error = deny).
- **`#[serde(try_from = "String", into = "String")]` derives for `Action`** — viable, but the
  hand-written `collect_str`/parse pair matches the `permissions.rs` sibling idiom and avoids the
  `into` clone; kept hand-written.
- **Regex for the grammar** — rejected: new dependency for a 20-line char scan.
- **Build-script codegen for the catalog** — already rejected upstream
  (`09-resolved-questions.md` §Action model: declarative macro + CI consistency check).

## Deferred / out of scope (verbatim carriers)

- `TargetRef` (type lands with M1.3's `authorize()`; `HostSoftwareItem` variant M2.1),
  `AccessEngine` + Permission→Action shim (M1.3), `access_grants`
  entity/migration/seed roles (M1.2), extractor macro + scope dictionary + `ci/verify_*` script
  (M1.4a), grant write-path validation rules 2–6 incl. the M1 non-`All` phase gate B9
  (M1.2/M1.6a), catalog endpoint (M1.6b), `Validate`-trait impls on request types (M1.6a),
  canonical docs + ADR + CONTEXT.md vocabulary (M1.9), non-`All` selector acceptance + matcher
  (M2.1), `plugin.` verb declarations in `declare_plugin!` (post-v1 trigger: first plugin needing
  one, per `09-resolved-questions.md` §Action model).
