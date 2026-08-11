# 0038 — Type-state request body validation via Unvalidated extractor

Date: 2026-08-06

## Status

Accepted

## Context

Mutating request handlers in `crates/ui/web-api/src/routes/` could extract a request body via
axum's raw `Json<T>`/`Form<T>` extractors and go on to use the deserialized value without ever
calling `Validate::validate()` on it. Nothing at the type level distinguished a body that had
been checked from one that had not — the two shared the same Rust type once deserialization
succeeded. `audit-2026-07-11` (line 1246) flagged this as a live gap: handlers existed that read
a `Validate`-implementing request type and skipped the validation call entirely, so malformed
input reached business logic unfiltered. A coding-standard rule already told authors to call
`.validate()` before use, but the rule had no enforcement mechanism — it depended on every author
remembering it, on every review catching a missed call, forever. `Validated<T>` (a `FromRequest`
wrapper that deserializes and validates in one step, unconditionally erroring on either failure)
already existed as an option, but nothing forced its use, or any equivalent, over the raw
extractors.

## Decision

Request bodies in mutating handlers are extracted through one of two type-state wrappers defined
in `crates/ui/web-api/src/extract.rs`:

- `Unvalidated<T>` / `UnvalidatedForm<T>` (JSON and form-encoded bodies respectively): the inner
  `T` field is private. The struct implements `FromRequest` (deserializing to the unvalidated
  state) and, for `Unvalidated<T>`, `OptionalFromRequest` as well, so a handler taking
  `Option<Unvalidated<T>>` still accepts a missing body as `None` without weakening the
  guarantee once a body _is_ present. The only way out of the wrapper is the consuming method
  `require_valid(self) -> Result<T, ValidationError>`, bounded on `T: Validate`, which runs
  validation and hands back the plain value only on success. There is no field accessor, no
  `Deref`, and no other exit — a handler that holds an `Unvalidated<T>` cannot reach the body
  without going through `require_valid()`, so a missed validation call is a compile error, not a
  reviewer's oversight.
- The pre-existing `Validated<T>`, for handlers that want deserialize-and-validate as a single
  extraction step with a fixed `400 Bad Request` rejection, remains available where a
  per-handler failure mapping isn't needed.

Two `require_valid()` failure mappings are in use across the converted handlers, both producing
`400 Bad Request`: a plain `error_response`/`ApiError` return, and — for handlers whose success
path already emits an audit event — a mirrored audit-log entry carrying
`AuditOutcome::ValidationFailed` before the 400 is returned, so a rejected mutation still leaves
an audit trail matching the entity it targeted.

Twenty-three handlers were converted from raw extractors to `Unvalidated<T>`/`UnvalidatedForm<T>`
across commits 6ed5bf6e5, 7a0f64d92, e5d16cb4e, 249a15a19, and bcc565ccf. A new CI gate,
`ci/verify_no_raw_body_extractors.sh`, now bans raw `Json<T>`/`Form<T>` parameters (and
unreviewed `FromRequest` impls) anywhere in `routes/`, against a frozen, shrink-only allowlist
(`ci/verify_no_raw_body_extractors_allowlist.txt`, frozen against a baseline outside the commit's
control — BASE_REF tip in CI, merge-base locally). The
allowlist rows fall into two classes: 29 legacy handlers that already call `.validate()` manually
on a raw-extracted body (`raw_extractor` rows — deferred to a future Stage 2 conversion, and
checked by the gate's residual pass to ensure the `.validate()` call is still present), and 5
handlers that read the body as raw bytes or an untyped `Request` with no `Validate`-implementing
type in the signature at all (`raw_body` rows: `auth.rs::logout`, `auth.rs::refresh`,
`ocsp.rs::ocsp_post`, `notifications.rs::notification_callback`, and
`oidc_auth.rs::oidc_link` — the last of which reads a raw link-token body ahead of its own
independent parsing, not a `serde`-deserialized `Validate` type).

Adoption is staged: Stage 1 (this change) converts the 23 handlers that had no validation call at
all — the audit-flagged bypass. Stage 2 converts the 29 `raw_extractor` legacy handlers and the 5
`raw_body` reads, each on its own schedule as their request types and rejection handling are
brought in line with `Unvalidated<T>`. Stage 3 would retire `Validated<T>` in favor of
`Unvalidated<T>` + `require_valid()` everywhere, if the two-mapping split above turns out not to
be worth preserving.

## Consequences

For every handler taking `Unvalidated<T>`/`UnvalidatedForm<T>`, unvalidated use of the request
body is now compile-blocked, not just discouraged: the type has no accessor other than
`require_valid()`, so the class of bug that motivated this ADR cannot recur at those call sites
short of deleting the `Validate` bound itself. `ci/verify_no_raw_body_extractors.sh` extends that
guarantee forward — a new mutating handler cannot introduce a fresh raw `Json<T>`/`Form<T>`
parameter without converting it; the gate-script amendment is the only review-gated exception.

The coverage is not total, and this ADR states the boundary precisely rather than rounding up.
The 5 `raw_body` sites are gate-enumerated, not compile-blocked: they read `Bytes`/`Request`
directly, so there is no `Validate`-bound type for the compiler to gate on, and the gate's
protection there is limited to noticing if the raw extractor disappears (a stale-allowlist check)
or if a `raw_extractor` row's `.validate()` call is deleted (the residual check) — it does not,
and cannot, verify that a `raw_body` handler validates its bytes correctly, only that the
allowlist entry still corresponds to a real raw read. Separately, the allowlist row _set_ — not
just its count — is mechanically frozen: every current row must already exist at a baseline
outside the commit's control (a baseline-subset check with bijective rename support, so a
file-move or facade split can carry its row to a new path without being treated as an addition).
CI passes the pull request's base ref, or the push event's prior `before` SHA on `main`, as that
baseline; runs off CI degrade to the merge-base of the default branch, or — if no baseline is
resolvable at all, e.g. offline or a shallow clone — warn and skip the sub-check while the other
checks still run. A commit that deletes one legacy allowlist row while adding a different,
unconverted one is caught: the new row cannot match any row removed at the baseline, so it is
flagged as an addition regardless of the row count staying flat. The remaining, deliberately
review-gated escape hatch is amending the gate script itself.

Finally, `require_valid()` returning `Err` is enforced to produce a rejection, but the gate does
not — and structurally cannot — verify _which_ rejection a handler builds from that error, or
whether it correctly mirrors `AuditOutcome::ValidationFailed` for handlers whose success path is
audited. Getting the failure-mapping choice (plain 400 vs. audit-mirrored 400) right for a given
handler remains a review-time concern, not a gate-enforced one.

`InvokeSurfaceInteractionRequest`'s `Validate` impl is `Ok(())` today — the choke point in
`surfaces.rs` (step 5) runs `require_valid()` unconditionally, but nothing yet exercises the
precedence between an action-gate `403` and a semantic `400` on the same request. The first real
`Validate` rule added to `InvokeSurfaceInteractionRequest` must add the discriminating test proving
the `403` still wins over a `400` when both conditions hold (see the dispatch choke-point comment
in `surfaces.rs`); that obligation is deferred, not forgotten.
