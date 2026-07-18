// Barrel re-exporting the configured client, the generated SDK + types, the hand-written
// escape-hatch helpers, and the surviving frontend-only types (api/local-types.ts). As of
// Task 12a this file contains ZERO hand-written API functions — every former local function
// has migrated either to the generated SDK or to api/client.ts (refresh / ETag-cache reset).
// The ETag auto-cache, the deduped `/auth/refresh` logic, and the user-change cache-wipe all
// now live solely in api/client.ts. Task 12b moved this file (formerly the top-level api
// barrel) into the directory as api/index.ts, repointed every `$lib/types` importer here,
// and deleted types.ts.

// Configure the client + interceptors as a side-effect on first import.
import './client';

// Generated SDK + types are reachable via `$lib/api`.
export * from './generated';

// Frontend-only survivor types with no generated equivalent (PaginatedResponse, User,
// FormField + form-schema helpers, the permission helpers, etc.) and the surface contract.
export * from './local-types';

// Error helpers: ApiError plus the Response→message / Response→ApiError mappers.
export { ApiError, extractErrorMessage, extractApiError } from './errors';

// Configured client, the deduped `/auth/refresh` (shared in-flight refresh), and the
// test-only ETag-cache reset — all canonical in api/client.ts.
export { apiClient, dedupedRefresh, _resetSettingsEtagCacheForTests } from './client';

// Raw-Response escape hatch (routes through the configured client).
export { apiGet, authenticatedFetch, loginRaw } from './raw';

// Cross-cutting domain helpers.
export { executeBatchChunked } from './batch';
export { sealedBoxEncrypt } from './crypto';
