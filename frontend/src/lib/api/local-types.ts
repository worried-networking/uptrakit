// Frontend-only survivor types that have no equivalent in the generated client
// (`./generated`). These were hand-written in the old `src/lib/types.ts` (deleted in
// Task 12b) and are re-exported from `$lib/api` so call sites import everything from one
// place. Types that DO have a generated equivalent (every `*Request` / `*Response`,
// `ErrorResponse`, `MfaMethod`, `Permission`, `PluginCapability`, `ServiceStatus`,
// `SystemAlert`, the notification enums, …) are intentionally NOT duplicated here — they
// resolve through `export * from './generated'` in `./index.ts`.

import type { Permission } from './generated';

// ── Authenticated user + permission helpers ───────────────────────────────────
// `User` is the shape the frontend renders the session around. The generated client only
// exposes per-endpoint `GetUserResponse`, so the canonical app-facing `User` stays local.

export interface User {
	id: string;
	email: string;
	first_name: string;
	last_name: string;
	permissions: Permission[];
	has_pending_email_change: boolean;
}

/** Returns true if the user holds at least one of the given permissions. */
export function hasAnyPermission(user: User | null | undefined, ...perms: Permission[]): boolean {
	if (!user) return false;
	return perms.some((p) => user.permissions.includes(p));
}

export function hasPermissionValue(user: User | null | undefined, permission?: string | null): boolean {
	if (!permission) return true;
	if (!user) return false;
	return user.permissions.includes(permission as Permission);
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
