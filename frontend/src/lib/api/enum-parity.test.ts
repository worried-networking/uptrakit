import { describe, it, expect } from 'vitest';
import { Permission, PluginCapability } from '../types';
import { PluginCapability as GenPluginCapability } from './generated';

// Generated Permission is a type-only union (no runtime const).
// The Rust enum uses the wire-safe Other(String) catch-all pattern, so utoipa emits
// a oneOf of single-value string enums → hey-api generates a TypeScript type union,
// not a runtime const object. String literal arms extracted manually from
// src/lib/api/generated/types.gen.ts (lines 1957–2002) — excluding the { Other: string } arm.
const GENERATED_PERMISSION_VALUES: string[] = [
	'ViewServices',
	'ApproveServices',
	'RejectServices',
	'RemoveServices',
	'UpdateServices',
	'ViewSystemServices',
	'ApproveSystemServices',
	'RejectSystemServices',
	'RemoveSystemServices',
	'UpdateSystemServices',
	'ViewSoftware',
	'CreateSoftware',
	'UpdateSoftware',
	'DeleteSoftware',
	'TriggerChecks',
	'TriggerUpdates',
	'ManageScheduler',
	'ViewHosts',
	'UpdateHosts',
	'DeactivateHosts',
	'ViewSettings',
	'ManageAuthSettings',
	'ManageEnrollmentTokens',
	'ManageAgentCerts',
	'ManageGlobalSettings',
	'ManageCommands',
	'ViewNotifications',
	'ManageNotifications',
	'ViewAuditLogs',
	'ViewSystemAuditLogs',
	'ManageUsers',
	'ManageIgnores',
	'TestPluginConfigs',
	'AccessMcp',
	'ViewInstanceConfigState',
	'ManageInstanceConfigState'
];

describe('R5: enum value parity (current types.ts vs generated)', () => {
	// R5 KNOWN DRIFT: spec declares PascalCase Permission values, app uses snake_case.
	// Deltas:
	//   - ALL values differ in case: generated is PascalCase (e.g. 'ViewServices'),
	//     current is snake_case (e.g. 'view_services').
	//   - 'AccessMcp' present in generated, absent from types.ts (no snake_case equivalent).
	// Backend OpenAPI (utoipa) schema is inaccurate vs the live wire format; reconcile in Plan C.
	// See docs/superpowers/notes/2026-06-28-openapi-codegen-audit.md (Task 7).
	// Expected to FAIL today; it.fails() keeps the suite green. When parity is achieved,
	// it.fails() reports RED — remove the it.fails wrapper then.
	it.fails('R5: Permission string values are byte-equal (KNOWN DRIFT)', () => {
		const current = Object.values(Permission).sort();
		const generated = GENERATED_PERMISSION_VALUES.slice().sort();
		expect(generated).toEqual(current);
	});

	// R5 KNOWN DRIFT: generated PluginCapability has 2 extra values absent from types.ts.
	// Deltas:
	//   - String format matches (both are snake_case).
	//   - Extra in generated (not in types.ts): 'software_item_lifecycle', 'enrich_installed_version'.
	//   These are new capabilities added to the Rust enum after types.ts was last updated.
	// See docs/superpowers/notes/2026-06-28-openapi-codegen-audit.md (Task 7).
	// Expected to FAIL today; it.fails() keeps the suite green. When parity is achieved,
	// it.fails() reports RED — remove the it.fails wrapper then.
	it.fails('R5: PluginCapability string values are byte-equal (KNOWN DRIFT)', () => {
		const current = Object.values(PluginCapability).sort();
		const generated = Object.values(GenPluginCapability).sort();
		expect(generated).toEqual(current);
	});
});
