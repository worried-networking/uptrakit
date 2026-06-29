// Transitional barrel re-exporting the configured client, the generated SDK, and the
// hand-written escape-hatch helpers. As of Task 12a this file contains ZERO hand-written
// API functions — every former local function has migrated either to the generated SDK
// or to api/client.ts (refresh / ETag-cache reset). The ETag auto-cache, the deduped
// `/auth/refresh` logic, and the user-change cache-wipe all now live solely in api/client.ts.
// (Task 12b will `git mv` this file to api/index.ts and repoint `$lib/types`.)

// Configure the client + interceptors as a side-effect on first import.
import './api/client';

// Generated SDK + types are reachable via `$lib/api`.
export * from './api/generated';

// Error helpers: ApiError plus the Response→message / Response→ApiError mappers.
export { ApiError, extractErrorMessage, extractApiError } from './api/errors';

// Configured client, the deduped `/auth/refresh` (shared in-flight refresh), and the
// test-only ETag-cache reset — all canonical in api/client.ts.
export { apiClient, dedupedRefresh, _resetSettingsEtagCacheForTests } from './api/client';

// Raw-Response escape hatch (routes through the configured client).
export { apiGet, authenticatedFetch, loginRaw } from './api/raw';

// Cross-cutting domain helpers.
export { executeBatchChunked } from './api/batch';
export { sealedBoxEncrypt } from './api/crypto';
export { listSurfaces, listSurfaceProviders, getSurfaceRead, invokeSurfaceInteraction } from './api/surfaces';
