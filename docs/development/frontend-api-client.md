# Frontend API Client

The frontend talks to the controller through a **generated** TypeScript SDK, not a hand-written
client. The SDK is produced by [`@hey-api/openapi-ts`](https://heyapi.dev) from the committed
OpenAPI spec `crates/ui/web-api/openapi.json`, which the Rust backend dumps. Cross-cutting concerns
(bearer auth, 401 refresh-retry, ETag `If-Match`, the 2FA redirect, `ApiError` mapping) live in a
configured client wrapper, so call sites stay declarative.

## Layout (`src/lib/api/`)

| file | role |
| --- | --- |
| `index.ts` | the `$lib/api` barrel — re-exports the generated SDK + the helpers below |
| `generated/` | `@hey-api/openapi-ts` output (committed; never hand-edit) |
| `client.ts` | configured hey-api client + interceptors; the singleton's verbs route through the deduped 401 refresh-retry |
| `errors.ts` | `ApiError` + `Response`→message / `Response`→`ApiError` mappers |
| `raw.ts` | raw-`Response` escape hatch (`authenticatedFetch` / `apiGet` / `loginRaw`) over the configured client |
| `surfaces.ts` | non-spec **surface** endpoints (0 utoipa paths) over the configured client |
| `crypto.ts` | sealed-box encryption (Web Crypto) |
| `batch.ts` | `executeBatchChunked` (client-side 100-id chunking) |
| `oauth.ts` | OAuth client/consent shim for paths outside `/api/v1` |
| `local-types.ts` | frontend-only types with no generated equivalent |

## Calling an endpoint

Import the generated operation from `$lib/api` and pass an option object (snake_case keys),
destructuring `{ data }`:

```ts
import { listServices, updateService } from '$lib/api';

const { data: services } = await listServices({ query: { status: 'pending', page, per_page: 25 } });
const { data: updated } = await updateService({ path: { id }, body: { display_name } });
```

`try/catch` works as usual — failures throw `instanceof ApiError`. Do **not** pass
`{ client: apiClient }`: the default client is already refresh-aware. Do **not** call `fetch`
directly for API endpoints.

## Adding / changing an endpoint

1. Annotate the Rust handler with `#[utoipa::path(...)]` and register it via `.routes(routes!(...))`
   **before** `split_for_parts()` in `crates/ui/web-api/src/router.rs`. Raw `.route()` handlers
   added after the split are absent from the spec (and the generated client).
2. For list endpoints, expose query filters with `params(<IntoParamsStruct>)` rather than a
   hand-maintained `params(("page", Query, …), …)` list — the manual form silently drops any struct
   field it forgets, which removed the software name-filter once.
3. Regenerate the spec and the client in one step, then commit both artifacts:

   ```sh
   ./scripts/regen-api.sh   # dumps openapi.json, then runs `npm run gen:api`
   git add crates/ui/web-api/openapi.json frontend/src/lib/api/generated/
   ```

   CI gates on staleness of both paths.
4. Use the new generated operation from `$lib/api`.

## Non-spec endpoints

**Surfaces** (the dynamic UI-extension API) have zero `#[utoipa::path]` annotations and are not in
the spec, so they have no generated SDK. They are hand-written in `api/surfaces.ts`, routed through
the same configured client. OAuth client/consent management (outside `/api/v1`) lives in
`api/oauth.ts`. SSE streams are hand-written (`sse.ts`) — hey-api does not generate streaming
clients.

## Scope

Today only the **frontend** client is generated from / gated against the spec. The Rust
`uptrakit-openapi-client` crate is not yet generated from this spec (planned follow-up), so the
spec is not yet a workspace-wide source of truth.
