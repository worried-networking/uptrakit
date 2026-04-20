# Manual Software Item Merge Design

## Goal

Allow users to manually consolidate duplicate software items that represent the same underlying software across different plugins and hosts, without
schema changes. The user chooses one survivor to keep, all other selected items are soft-deleted, and eligible host links are transferred to the
survivor with careful deduplication.

## Scope

This feature includes:

- frontend entry points from the software dashboard and software detail page
- a two-step merge wizard
- dedicated backend preview and execute APIs
- backend-enforced transactional merge behavior

This feature explicitly does not include:

- automatic duplicate detection or automatic merging
- schema changes
- survivor metadata mutation during merge
- custom audit-comment persistence

## User Experience

### Entry points

Users can start merge from:

- the software dashboard row context menu via `Merge...`
- the software detail page via `Merge...`
- the software dashboard batch action bar via `Merge`

### Wizard flow

The merge flow is a two-step wizard.

#### Step 1: candidate selection and survivor choice

- Batch entry preloads the currently selected software items.
- Single-item entry preloads the clicked item and opens tenant-wide candidate search immediately.
- Single-item entry may also surface same-host suggestions first for convenience, but those suggestions are not a restriction.
- The user can add and remove candidates before continuing.
- The user must designate exactly one survivor from within the candidate set.

#### Step 2: confirmation

The confirmation step shows:

- the survivor that will be kept
- the loser items that will be soft-deleted
- affected host links split into:
  - links that will move to the survivor
  - links already represented on the survivor and therefore skipped

On success, the UI always stays on the dashboard and shows a toast.

## Backend API

Use a dedicated merge API rather than overloading generic batch actions.

### Preview endpoint

`POST /api/v1/software-items/merge/preview`

Request:

- `candidate_ids: Uuid[]`
- `survivor_id: Uuid`
- optional `seed_item_id: Uuid` for single-item flows that want server-side suggestions

Response:

- normalized candidate list actually considered
- survivor summary
- loser summaries
- host-link transfer plan with `move` and `skip_duplicate` groups
- aggregate counts for the confirm screen

The preview endpoint is responsible for validating merge feasibility using the same rules as execution.

### Execute endpoint

`POST /api/v1/software-items/merge/execute`

Request:

- `candidate_ids: Uuid[]`
- `survivor_id: Uuid`

Response:

- survivor id
- loser ids soft-deleted
- moved host-link ids
- skipped-duplicate host-link ids

Execution revalidates everything and performs the merge in one transaction.

## Merge Rules

### Tenant and lifecycle checks

The backend must reject merge requests when:

- fewer than two distinct active items are present
- the survivor is not included in the candidate set
- any candidate does not exist
- any candidate belongs to another tenant
- any candidate is already deactivated

### Survivor behavior

The survivor keeps its existing:

- software item row
- metadata
- current host links

The merge does not mutate survivor metadata to match any loser.

### Loser behavior

Each loser is soft-deleted by setting the existing deactivation fields. No hard delete is introduced.

### Host-link transfer behavior

Host-link transfer must preserve legitimate duplicates while avoiding accidental collapse.

Deduplication must be based on logical host-link identity, not `host_id` alone.

Rules:

- two unqualified links on the same host are duplicates
- two qualified links are duplicates only when their qualifiers match
- a qualified link and an unqualified link are not duplicates of each other
- multiple valid sibling links on the same host, such as multiple Docker containers, must remain distinct

For each loser link:

- if the survivor already has an equivalent link, mark it as `skip_duplicate`
- otherwise move the loser link to the survivor

Associated `host_software_item_plugins` rows must move with the transferred host link so that the per-link plugin behavior is preserved.

## Suggested Candidate Discovery

The backend should not define duplicate candidates by software-item name alone, because cross-plugin duplicates may legitimately differ in name.

Single-item flows should support:

- tenant-wide candidate search
- optional same-host suggestions for convenience

Batch flows use the explicit user selection as the starting candidate set.

## Architecture Placement

### Shared API types

Add merge request and response types to:

- `crates/shared/web-api-types/src/software_items.rs`

### OpenAPI client

Add client methods to:

- `crates/shared/openapi-client/src/software_items.rs`

### Web API routes

Add thin handlers under:

- `crates/ui/web-api/src/routes/software_items/mod.rs`

### Query-layer logic

Add focused merge query logic under:

- `crates/ui/web-api-queries/src/queries/software_items/merge.rs`

Re-export it from the existing software-items query module.

The query layer owns:

- preview computation
- validation
- transactional execute logic
- transfer and dedupe behavior

## Frontend Shape

The frontend should use a reusable merge wizard component rather than expanding the already large software route files.

Recommended placement:

- API helpers in `frontend/src/lib/api.ts`
- merge wizard component in `frontend/src/lib/components/`
- integration wiring in:
  - `frontend/src/routes/software/+page.svelte`
  - `frontend/src/routes/software/[id]/+page.svelte`

The wizard should treat the backend preview as authoritative and render server-returned outcomes rather than computing merge effects locally.

## Error Handling

The backend should return clear validation failures for:

- invalid survivor selection
- missing or deactivated candidates
- cross-tenant references
- merge requests that collapse incompatible link identities

The frontend should surface preview and execute errors inline in the wizard and avoid silently dropping candidates.

## Testing

### Backend query tests

Add focused tests for:

- preview validation failures
- transfer of non-duplicate loser links
- skip behavior when survivor already has an equivalent link
- preservation of qualified sibling links
- survivor metadata remaining unchanged
- losers becoming soft-deleted after execute

### Route and client tests

Add coverage for:

- preview request and response handling
- execute request and response handling

### Frontend tests

Add coverage for:

- wizard state transitions
- survivor selection
- candidate add/remove behavior
- rendering of moved vs skipped links in confirmation
- success behavior staying on the dashboard with a toast

### Verification commands

At minimum run:

- `cargo fmt --all`
- targeted Rust tests for the web API and web-api-queries crates
- relevant frontend tests and lint checks for the new wizard flow

## Tradeoff Decision

The design uses a dedicated merge API rather than generic batch actions because merge requires:

- structured preview data
- survivor-specific validation
- transactional transfer semantics
- future room for richer candidate discovery

That extra surface area is justified by the feature complexity and keeps merge logic out of generic batch plumbing.
