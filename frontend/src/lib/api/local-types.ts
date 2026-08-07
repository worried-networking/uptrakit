// Frontend-only survivor types that have no equivalent in the generated client
// (`./generated`). These were hand-written in the old `src/lib/types.ts` (deleted in
// Task 12b) and are re-exported from `$lib/api` so call sites import everything from one
// place. Types that DO have a generated equivalent (every `*Request` / `*Response`,
// `AuthorityStatus`, `ErrorResponse`, `MfaMethod`, `PluginCapability`,
// `ServiceStatus`, `SystemAlert`, the notification enums, …) are intentionally NOT
// duplicated here — they resolve through `export * from './generated'` in `./index.ts`.

import type { AuthorityStatus } from './generated';

// ── Authenticated user + action helpers ───────────────────────────────────────
// `User` is the shape the frontend renders the session around. The generated client only
// exposes per-endpoint `GetUserResponse`, so the canonical app-facing `User` stays local.

/**
 * Action string in the catalog grammar (`resource:verb`, including dynamic
 * `plugin.*` / `surface.*` and system-plane `system.*` forms). Open set —
 * dynamic actions exist only at runtime, so this is a branded string shape,
 * not a closed union.
 */
export type Action = `${string}:${string}`;

/**
 * Built-in actions the UI gates on. Values are validated against the server
 * catalog (the committed OpenAPI scope dictionary) by `actions.test.ts`.
 */
export const Actions = {
	SERVICES_READ: 'services:read',
	SERVICES_APPROVE: 'services:approve',
	SERVICES_REJECT: 'services:reject',
	SERVICES_DELETE: 'services:delete',
	SERVICES_UPDATE: 'services:update',
	SYSTEM_SERVICES_READ: 'system.services:read',
	SYSTEM_SERVICES_APPROVE: 'system.services:approve',
	SYSTEM_SERVICES_REJECT: 'system.services:reject',
	SYSTEM_SERVICES_DELETE: 'system.services:delete',
	SYSTEM_SERVICES_UPDATE: 'system.services:update',
	HOSTS_READ: 'hosts:read',
	HOSTS_UPDATE: 'hosts:update',
	HOSTS_DELETE: 'hosts:delete',
	SOFTWARE_READ: 'software:read',
	SOFTWARE_CREATE: 'software:create',
	SOFTWARE_UPDATE: 'software:update',
	SOFTWARE_DELETE: 'software:delete',
	CHECKS_TRIGGER: 'checks:trigger',
	UPDATES_TRIGGER: 'updates:trigger',
	SCHEDULER_MANAGE: 'scheduler:manage',
	SETTINGS_READ: 'settings:read',
	SETTINGS_AUTH_MANAGE: 'settings.auth:manage',
	SETTINGS_ENROLLMENT_TOKENS_MANAGE: 'settings.enrollment-tokens:manage',
	SETTINGS_CERTIFICATES_MANAGE: 'settings.certificates:manage',
	SYSTEM_SETTINGS_MANAGE: 'system.settings:manage',
	COMMANDS_MANAGE: 'commands:manage',
	NOTIFICATIONS_READ: 'notifications:read',
	NOTIFICATIONS_MANAGE: 'notifications:manage',
	AUDIT_READ: 'audit:read',
	SYSTEM_AUDIT_READ: 'system.audit:read',
	USERS_MANAGE: 'users:manage',
	DISCOVERY_IGNORES_MANAGE: 'discovery.ignores:manage',
	PLUGIN_CONFIGS_TRIGGER: 'plugin-configs:trigger',
	MCP_USE: 'mcp:use',
	SYSTEM_CONFIG_STATE_READ: 'system.config-state:read',
	SYSTEM_CONFIG_STATE_MANAGE: 'system.config-state:manage'
} as const satisfies Record<string, Action>;

export interface User {
	id: string;
	email: string;
	first_name: string;
	last_name: string;
	actions: readonly string[];
	authority: AuthorityStatus;
	has_pending_email_change: boolean;
}

/**
 * Returns true if the user holds the given action. The field-level `?.`
 * is deliberate: `User` is produced by `as unknown as` casts from wire
 * payloads and by hand-written test fixtures — a missing `actions` key
 * must degrade to "deny", never throw in the layout.
 */
export function hasAction(user: User | null | undefined, action: Action): boolean {
	return user?.actions?.includes(action) ?? false;
}

/** Returns true if the user holds at least one of the given actions. */
export function hasAnyAction(user: User | null | undefined, ...actions: Action[]): boolean {
	return actions.some((a) => hasAction(user, a));
}

/**
 * Gate against an optional server-supplied requirement (e.g. a surface's
 * `required_action`): a null/undefined requirement gates nothing.
 */
export function hasActionValue(user: User | null | undefined, action?: string | null): boolean {
	// `!action` (not just null/undefined): an empty-string requirement
	// gates nothing.
	if (!action) return true;
	return user?.actions?.includes(action) ?? false;
}

/**
 * Returns true when the backend could not resolve the user's authority
 * (e.g. a transient access-engine failure). In that state `me` returns
 * HTTP 200 with an empty `actions` array on purpose, so callers must not
 * read the resulting empty-`actions` gate as a genuine denial.
 */
export function isAuthorityUnavailable(user: User | null | undefined): boolean {
	return user?.authority === 'unavailable';
}

// ── Generic pagination envelope ────────────────────────────────────────────────
// The generated client monomorphizes pagination per resource (e.g.
// `PaginatedResponseHostResponse`); this generic stays for code that is itself generic
// over the row type.

export interface PaginatedResponse<T> {
	items: T[];
	total: number;
	page: number;
	per_page: number;
	total_pages: number;
}

// ── Update history ──────────────────────────────────────────────────────────────

export type UpdateHistoryStatus =
	| 'queued'
	| 'pending'
	| 'in_progress'
	| 'awaiting_restart'
	| 'completed'
	| 'failed'
	| 'interrupted';

// ── Audit log (richer than the generated `AuditLogResponse`) ─────────────────────
// The generated row types `action_kind`/`details_json`/snapshots as `string` / `unknown`;
// the detail view indexes `details_json[...]` and discriminates on `action_kind`, so the
// narrower hand-written shape is preserved and rows are bridged at the call site.

export interface AuditLogEntry {
	id: string;
	actor_type: string;
	actor_id: string | null;
	actor_display: string | null;
	action_type: string;
	target_type: string | null;
	target_id: string | null;
	target_display: string | null;
	outcome: string;
	details_json: Record<string, unknown> | null;
	request_id: string | null;
	action_kind: 'stateful' | 'event';
	before_snapshot: Record<string, unknown> | null;
	after_snapshot: Record<string, unknown> | null;
	correlation_id: string | null;
	occurred_at: string;
}

// ── Software attestation status ──────────────────────────────────────────────────

export type AttestationStatus = 'Verified' | 'NotFound' | 'Unverified';

// ── Dynamic form schema (surfaces + plugin config forms) ─────────────────────────

export type FieldType =
	| 'text'
	| 'password'
	| 'number'
	| 'select'
	| 'multi_select'
	| 'textarea'
	| 'toggle'
	| 'hidden'
	| 'ssh_private_key'
	| string;

export interface SelectOption {
	value: string;
	label: string;
}

/** Dynamic data source for a `Select` field, loaded at form-open time. */
export type SelectSource =
	| {
			type: 'rest_api';
			/** API path relative to the controller base URL (e.g., `"/api/v1/hosts"`). */
			path: string;
			/** Field in each response item to use as the submitted option value. */
			value_field: string;
			/** Field in each response item to use as the human-readable label. */
			label_field: string;
	  }
	| {
			type: 'action';
			/** Extension action ID to invoke. Must return `{ options: [{ value, label }] }`. */
			action_id: string;
	  };

/** Condition for conditional field visibility. */
export interface VisibleWhen {
	/** Key of the controlling field. */
	field: string;
	/** Field is visible when the controlling field's value is in this list. */
	values: string[];
}

export interface FormField {
	key: string;
	label: string;
	field_type: FieldType;
	required: boolean;
	placeholder?: string;
	help_text?: string;
	default_value?: string;
	options?: SelectOption[];
	/** When set, options are loaded dynamically from the given source. Takes precedence over `options`. */
	select_source?: SelectSource;
	/** When true, the field value is encrypted client-side before being sent to the service. */
	sensitive?: boolean;
	/** When true, the textarea value is a newline-separated list serialized as a JSON string array. */
	list?: boolean;
	/** When set, the field is only visible when the controlling field's value matches. */
	visible_when?: VisibleWhen;
}

// ── Surface contract (frontend-only extension-point descriptors) ──────────────────

export type {
	ControllerQueryId,
	DataSourceDescriptor,
	DataSourceId,
	InteractionDescriptor,
	InteractionId,
	ProviderEncryptionMetadata,
	RegisteredSurface,
	SchemaContract,
	SurfaceCapability,
	SurfaceDescriptor,
	SurfaceId,
	SurfaceNode,
	SurfaceScope,
	SurfaceTab,
	SurfaceTabId,
	SurfaceTargeting
} from '../surfaces/contract';
