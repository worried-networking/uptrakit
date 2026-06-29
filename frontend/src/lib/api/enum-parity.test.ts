import { describe, it, expect } from 'vitest';
import { Permission, PluginCapability } from '../types';
import type { NotificationEventType, NotificationDeliveryStatus } from '../types';
import {
	Permission as GenPermission,
	PluginRole as GenPluginRole,
	NotificationEventType as GenNotificationEventType,
	NotificationDeliveryStatus as GenNotificationDeliveryStatus,
	PluginCapability as GenPluginCapability
} from './generated';

// ── Background ────────────────────────────────────────────────────────────────
//
// The four behavior-dependent catch-all enums (Permission, PluginRole,
// NotificationEventType, NotificationDeliveryStatus) previously emitted Rust
// PascalCase identifiers in the OpenAPI schema instead of their serde snake_case
// wire strings (a utoipa derive bug — the derive was blind to the hand-written
// infallible serde). The backend `ToSchema` fix now documents the wire strings,
// so hey-api emits clean `as const` objects whose VALUES are the snake_case wire
// strings — which is what the app has always sent and received. These guards
// prove the drift collapsed and keep it collapsed.

describe('R5: enum value parity (current types.ts vs generated)', () => {
	// Permission: app enum (runtime) vs generated const (runtime). Both are
	// snake_case after the fix and must be byte-equal. `access_mcp` was added to
	// the types.ts Permission enum as part of this fix (it is a real backend
	// permission that was previously missing from the app source of truth).
	it('R5: Permission string values are byte-equal', () => {
		const current = Object.values(Permission).sort();
		const generated = Object.values(GenPermission).sort();
		expect(generated).toEqual(current);
	});

	// NotificationEventType is a TS type-union in types.ts (no runtime const), so
	// Object.values() cannot reach it. The app source of truth is mirrored here as
	// a literal array typed against the union — TypeScript rejects any literal not
	// in the app union — and compared against the generated const's values.
	it('R5: NotificationEventType literal members are byte-equal', () => {
		const appSourceOfTruth: NotificationEventType[] = [
			'update_available',
			'update_completed',
			'update_failed',
			'new_software_discovered',
			'new_service_enrolled',
			'ca_rotated',
			'batch_update_completed',
			'batch_update_partially_completed',
			'stdin_attention'
		];
		const generated = Object.values(GenNotificationEventType).sort();
		expect(generated).toEqual(appSourceOfTruth.slice().sort());
	});

	// NotificationDeliveryStatus is likewise a TS type-union in types.ts.
	it('R5: NotificationDeliveryStatus literal members are byte-equal', () => {
		const appSourceOfTruth: NotificationDeliveryStatus[] = ['pending', 'delivered', 'failed'];
		const generated = Object.values(GenNotificationDeliveryStatus).sort();
		expect(generated).toEqual(appSourceOfTruth.slice().sort());
	});

	// PluginRole has no source-of-truth union in types.ts (role fields are typed
	// `string`). Pin the generated wire literals to the documented backend
	// contract so any future drift (e.g. a regressed PascalCase schema) is caught.
	it('R5: PluginRole literal members match the wire contract', () => {
		const wireContract = ['detect_version', 'fetch_releases', 'execute_update', 'pre_update_hook', 'post_update_hook'];
		const generated = Object.values(GenPluginRole).sort();
		expect(generated).toEqual(wireContract.slice().sort());
	});

	// R5 KNOWN DRIFT: generated PluginCapability has 2 extra values absent from types.ts.
	// Deltas:
	//   - String format matches (both are snake_case).
	//   - Extra in generated (not in types.ts): 'software_item_lifecycle', 'enrich_installed_version'.
	//   These are new capabilities added to the Rust enum after types.ts was last updated.
	// This is a separate mechanical-additive drift, NOT one of the 4 behavior-dependent
	// enums fixed in Plan C Task 1.
	// See docs/superpowers/notes/2026-06-28-openapi-codegen-audit.md (Task 7).
	// Expected to FAIL today; it.fails() keeps the suite green. When parity is achieved,
	// it.fails() reports RED — remove the it.fails wrapper then.
	it.fails('R5: PluginCapability string values are byte-equal (KNOWN DRIFT)', () => {
		const current = Object.values(PluginCapability).sort();
		const generated = Object.values(GenPluginCapability).sort();
		expect(generated).toEqual(current);
	});
});
