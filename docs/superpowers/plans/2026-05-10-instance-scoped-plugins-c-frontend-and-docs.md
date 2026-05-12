# Instance-Scoped Plugins — Plan C: Frontend + Remaining Docs

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the "Instance Plugins" section to the Plugin Configs settings tab (toggle + config edit + Pending restart badge), expose the four
`/api/v1/instance-plugins` endpoints through the API client, and ship every remaining documentation deliverable from the spec.

**Architecture:** New `<SectionCard>` rendered conditionally for `canManageGlobalSettings` users at the top of `PluginConfigsTab.svelte`. Reuses the
existing `flattenConfig`/`unflattenConfig`/`requiredFieldErrors` helpers already present in that file. New types in `frontend/src/lib/types.ts`; new
API functions in `frontend/src/lib/api.ts`. No new shared components — strictly the existing toolkit (`SectionCard`, `DataTable`, `ModalShell`,
`FormFieldRow`, `Input`, `Textarea`, `Checkbox`, `Select`, `StatusBadge`, `Button`, `ConfirmDialog`).

**Tech Stack:** SvelteKit, Svelte 5 runes, TypeScript strict. Source of truth: spec
`docs/superpowers/specs/2026-05-10-instance-scoped-plugins-design.md` §5 + §10. Frontend rules from `frontend/AGENTS.md`. Snapshot:
`.superpowers/standards-snapshot.md`. Depends on Plan B merged (the `/api/v1/instance-plugins` endpoints exist).

**Quality gates (final task):** `cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build` plus the full
`cargo` sweep from Plans A/B (so the `markdownlint` step picks up the new `.md` files).

---

## File structure

| File                                                      | Status             | Responsibility                                                                      |
| --------------------------------------------------------- | ------------------ | ----------------------------------------------------------------------------------- |
| `frontend/src/lib/types.ts`                               | modify             | Add `InstancePluginSummary` interface                                               |
| `frontend/src/lib/api.ts`                                 | modify             | Add `listInstancePlugins`, `setInstancePluginEnabled`, `upsertInstancePluginConfig` |
| `frontend/src/routes/settings/PluginConfigsTab.svelte`    | modify             | New "Instance Plugins" section above existing Configurations section                |
| `frontend/src/routes/settings/PluginConfigsTab.test.ts`   | modify (or create) | Tests for new section render + toggle interaction                                   |
| `docs/development/plugin-guidelines.md`                   | modify             | New section "Plugin scopes (Tenant vs Instance)"                                    |
| `ARCHITECTURE.md`                                         | modify             | Brief mention in Plugin section                                                     |
| `website/public/docs/end-user/dashboard-icons/index.html` | modify             | Note disabled-by-default + instance-owner managed + CDN-URL persistence note        |
| `docs/admin/instance-plugins.md`                          | create             | Short admin guide page                                                              |
| `docs/admin/README.md`                                    | create             | Index for the new admin docs section (only if `docs/admin/` doesn't already exist)  |

---

## Task 1: Branch confirmation

**Files:** none

- [ ] **Step 1: Confirm Plan B is committed**

```bash
git log --oneline -10
```

Expected: ADR + dashboard-icons descriptor flip + integration tests visible in history.

- [ ] **Step 2: Branch (or stay on Plan B's branch)**

```bash
git checkout -b feat/instance-scoped-plugins-frontend-docs
```

No commit.

---

## Task 2: Add `InstancePluginSummary` type

**Files:**

- Modify: `frontend/src/lib/types.ts`

Snapshot rules (frontend AGENTS): TS strict, no `any`.

- [ ] **Step 1: Add interface alongside existing `PluginTypeInfo` definition**

Locate `PluginTypeInfo` in `types.ts` and append:

```ts
export interface InstancePluginSummary {
  plugin_type: string;
  display_name: string;
  /** Stored desired state from instance_plugin_setting. */
  enabled: boolean;
  /**
   * Catalog snapshot from controller boot. When `enabled !== running_enabled`,
   * the UI shows a "Pending restart" badge.
   */
  running_enabled: boolean;
  has_instance_config: boolean;
  instance_config_form_fields: FormField[];
  type_settings_form_fields: FormField[];
  current_config: Record<string, unknown>;
  updated_at: string | null;
}
```

- [ ] **Step 2: Build types**

```bash
cd frontend && npm run check 2>&1 | tail -10
```

Expected: zero errors.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/types.ts
git commit -m "feat(frontend): InstancePluginSummary type

Mirrors crates/shared/web-api-types InstancePluginSummary shape, includes
running_enabled for restart-pending badge."
```

---

## Task 3: Add API client functions

**Files:**

- Modify: `frontend/src/lib/api.ts`

Snapshot rules (frontend AGENTS): all API calls go through `src/lib/api.ts`; no direct `fetch` in components.

- [ ] **Step 1: Locate the import block at the top and add the new type**

```ts
import type {
  // ... existing imports ...
  InstancePluginSummary,
} from "./types";
```

- [ ] **Step 2: Add the three functions next to `listPluginTypes` / `upsertPluginTypeSettings`**

```ts
export async function listInstancePlugins(): Promise<InstancePluginSummary[]> {
  return apiGet<InstancePluginSummary[]>("/api/v1/instance-plugins");
}

export async function setInstancePluginEnabled(
  pluginType: string,
  enabled: boolean,
): Promise<InstancePluginSummary> {
  return apiPut<InstancePluginSummary>(
    `/api/v1/instance-plugins/${encodeURIComponent(pluginType)}/enabled`,
    { enabled },
  );
}

export async function upsertInstancePluginConfig(
  pluginType: string,
  config: Record<string, unknown>,
): Promise<InstancePluginSummary> {
  return apiPut<InstancePluginSummary>(
    `/api/v1/instance-plugins/${encodeURIComponent(pluginType)}/config`,
    { config },
  );
}
```

(Match the actual function names exposed by the file's existing `apiGet`/`apiPut` helpers — read the top of the file first.)

- [ ] **Step 3: Build types**

```bash
cd frontend && npm run check 2>&1 | tail -10
```

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/api.ts
git commit -m "feat(frontend): API client for /api/v1/instance-plugins

list, set-enabled, upsert-config wrappers. Per AGENTS rule, all instance-
plugin API calls flow through src/lib/api.ts (no direct fetch in components)."
```

---

## Task 4: Add "Instance Plugins" section to `PluginConfigsTab.svelte`

**Files:**

- Modify: `frontend/src/routes/settings/PluginConfigsTab.svelte`

Snapshot rules (frontend AGENTS): reuse existing components; Svelte 5 runes; no `any`; keep section conventions consistent with built-in pages.

- [ ] **Step 1: Add imports at the top of the `<script>` block**

```ts
import {
  listInstancePlugins,
  setInstancePluginEnabled,
  upsertInstancePluginConfig,
} from "$lib/api";
import type { InstancePluginSummary } from "$lib/types";
```

- [ ] **Step 2: Add reactive state next to existing state declarations**

```ts
const canManageGlobalSettings = $derived(
  getUser()?.permissions.includes(Permission.ManageGlobalSettings) ?? false,
);

let instancePlugins: InstancePluginSummary[] = $state([]);
let instancePluginsLoading: boolean = $state(true);
let instancePluginsError: string | null = $state(null);
let instancePluginToggleConfirm: {
  plugin_type: string;
  display_name: string;
  next_enabled: boolean;
} | null = $state(null);
let editingInstancePluginType: string | null = $state(null);
let showInstancePluginConfigModal: boolean = $state(false);
let instancePluginFormValues: Record<string, string> = $state({});
let instancePluginFieldErrors: Record<string, string> = $state({});
```

- [ ] **Step 3: Wire load into `onMount`**

Find the existing `onMount(() => { ... })` block. Add a branch:

```ts
onMount(() => {
  // ... existing branches ...
  if (canManageGlobalSettings) {
    void loadInstancePlugins();
  }
});

async function loadInstancePlugins() {
  instancePluginsLoading = true;
  instancePluginsError = null;
  try {
    instancePlugins = await listInstancePlugins();
  } catch (e) {
    instancePluginsError =
      e instanceof Error ? e.message : "Failed to load instance plugins";
    showError(instancePluginsError);
  } finally {
    instancePluginsLoading = false;
  }
}
```

- [ ] **Step 4: Add the toggle handler**

```ts
async function executeInstancePluginToggle() {
  if (!instancePluginToggleConfirm) return;
  const { plugin_type, display_name, next_enabled } =
    instancePluginToggleConfirm;
  instancePluginToggleConfirm = null;
  try {
    const updated = await setInstancePluginEnabled(plugin_type, next_enabled);
    instancePlugins = instancePlugins.map((p) =>
      p.plugin_type === plugin_type ? updated : p,
    );
    showSuccess(
      `${display_name} ${next_enabled ? "enabled" : "disabled"}. Restart the controller to apply.`,
    );
  } catch (e) {
    showError(
      e instanceof Error ? e.message : "Failed to toggle instance plugin",
    );
  }
}
```

- [ ] **Step 5: Add the config edit handlers**

```ts
function openEditInstancePluginConfig(pluginType: string) {
  editingInstancePluginType = pluginType;
  const summary = instancePlugins.find((p) => p.plugin_type === pluginType);
  if (!summary) return;
  instancePluginFormValues = flattenConfig(
    summary.current_config,
    summary.instance_config_form_fields,
  );
  instancePluginFieldErrors = {};
  showInstancePluginConfigModal = true;
}

function closeInstancePluginConfigModal() {
  showInstancePluginConfigModal = false;
  editingInstancePluginType = null;
  instancePluginFieldErrors = {};
}

async function saveInstancePluginConfig() {
  if (!editingInstancePluginType) return;
  const summary = instancePlugins.find(
    (p) => p.plugin_type === editingInstancePluginType,
  );
  if (!summary) return;
  const fields = summary.instance_config_form_fields;
  instancePluginFieldErrors = requiredFieldErrors(
    fields,
    instancePluginFormValues,
  );
  if (Object.keys(instancePluginFieldErrors).length > 0) return;
  const config = unflattenConfig(instancePluginFormValues, fields);
  try {
    const updated = await upsertInstancePluginConfig(
      editingInstancePluginType,
      config,
    );
    instancePlugins = instancePlugins.map((p) =>
      p.plugin_type === editingInstancePluginType ? updated : p,
    );
    showSuccess("Instance plugin configuration saved.");
    closeInstancePluginConfigModal();
  } catch (e) {
    showError(
      e instanceof Error ? e.message : "Failed to save instance plugin config",
    );
  }
}
```

- [ ] **Step 6: Add the section markup at the very top of the template (above `Configurations`)**

The markup uses `as unknown as Record<string, unknown>[]` and `as unknown as InstancePluginSummary` to bridge the `DataTable` component's untyped row
contract. Before adding the casts, verify whether `DataTable` accepts a generic row type —
`grep -n "rows: " frontend/src/lib/components/ui/DataTable.svelte`.

- If `DataTable` is already generic (`<T>`-parameterized rows): pass `instancePlugins` directly without casts and parameterize the `{#snippet row}`
  callback with the concrete type. Preferred — matches snapshot rule "Avoid `any` casts in TypeScript" (frontend AGENTS).
- If `DataTable` is not generic: the `as unknown as` casts mirror the **existing** pattern already in this same file (search for
  `as unknown as Record<string, unknown>[]` in `PluginConfigsTab.svelte` — it is used by the Configurations and Type Defaults sections). Match the
  existing pattern. A separate refactor making `DataTable` generic is out of scope for this plan.

```svelte
{#if canManageGlobalSettings}
  <SectionCard
    title="Instance Plugins"
    description="Plugins managed at the instance level. Disabled plugins are invisible to tenant Operators. Changes take effect after the controller restarts."
  >
    {#if instancePluginsLoading}
      <p class="text-sm text-[var(--text-muted)]">Loading instance plugins...</p>
    {:else if instancePluginsError}
      <Callout tone="danger" title="Failed to load">{instancePluginsError}</Callout>
    {:else if instancePlugins.length === 0}
      <p class="text-sm text-[var(--text-muted)]">No instance-scoped plugins available.</p>
    {:else}
      <DataTable
        columns={[
          { key: 'plugin', label: 'Plugin' },
          { key: 'state', label: 'State' },
          { key: 'actions', label: 'Actions' }
        ]}
        rows={instancePlugins as unknown as Record<string, unknown>[]}
        loading={false}
        error={null}
        emptyTitle="No instance plugins"
        emptyDescription="No instance-scoped plugins available."
        rowKey={(row) => (row as unknown as InstancePluginSummary).plugin_type}
      >
        {#snippet header()}
          <tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
            <th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
              >Plugin</th
            >
            <th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
              >State</th
            >
            <th class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
              >Actions</th
            >
          </tr>
        {/snippet}
        {#snippet row(rowValue, _index)}
          {@const plugin = rowValue as unknown as InstancePluginSummary}
          {@const restartPending = plugin.enabled !== plugin.running_enabled}
          <tr class="border-b border-[var(--border-subtle)] last:border-b-0">
            <td class="table-cell-pad">
              <div class="flex flex-col">
                <span>{plugin.display_name}</span>
                <span class="text-xs text-[var(--text-muted)]">{plugin.plugin_type}</span>
              </div>
            </td>
            <td class="table-cell-pad">
              <div class="flex items-center gap-2">
                {#if plugin.enabled}
                  <StatusBadge tone="success" label="Enabled" />
                {:else}
                  <StatusBadge tone="neutral" label="Disabled" />
                {/if}
                {#if restartPending}
                  <StatusBadge tone="warning" label="Pending restart" />
                {/if}
              </div>
            </td>
            <td class="table-cell-pad">
              <div class="flex flex-wrap gap-1">
                <Button
                  variant="secondary"
                  size="sm"
                  onclick={() =>
                    (instancePluginToggleConfirm = {
                      plugin_type: plugin.plugin_type,
                      display_name: plugin.display_name,
                      next_enabled: !plugin.enabled
                    })}
                >
                  {plugin.enabled ? 'Disable' : 'Enable'}
                </Button>
                {#if plugin.has_instance_config}
                  <Button
                    variant="secondary"
                    size="sm"
                    onclick={() => openEditInstancePluginConfig(plugin.plugin_type)}
                    >Edit Settings</Button
                  >
                {/if}
              </div>
            </td>
          </tr>
        {/snippet}
      </DataTable>
    {/if}
  </SectionCard>
{/if}

{#if instancePluginToggleConfirm}
  <ConfirmDialog
    title="{instancePluginToggleConfirm.next_enabled ? 'Enable' : 'Disable'} {instancePluginToggleConfirm.display_name}"
    messagePrefix="Are you sure you want to {instancePluginToggleConfirm.next_enabled ? 'enable' : 'disable'}"
    entityName={instancePluginToggleConfirm.display_name}
    confirmLabel={instancePluginToggleConfirm.next_enabled ? 'Enable' : 'Disable'}
    onconfirm={executeInstancePluginToggle}
    oncancel={() => (instancePluginToggleConfirm = null)}
  >
    {#snippet body()}
      <p class="text-sm text-[var(--text-muted)]">
        Restart the controller to apply this change.
      </p>
    {/snippet}
  </ConfirmDialog>
{/if}

{#if showInstancePluginConfigModal && editingInstancePluginType}
  {@const summary = instancePlugins.find((p) => p.plugin_type === editingInstancePluginType)}
  {#if summary}
    <ModalShell
      title="Edit Instance Configuration — {summary.display_name}"
      onclose={closeInstancePluginConfigModal}
      maxWidth="max-w-2xl max-h-[90vh] overflow-y-auto"
    >
      <div class="space-y-4">
        {#each summary.instance_config_form_fields as field (field.key)}
          {#if isFieldVisible(field, instancePluginFormValues)}
            <FormFieldRow
              label={field.label}
              inputId={'ip-' + field.key}
              required={field.required}
              hint={field.field_type === 'toggle' ? undefined : field.help_text}
              error={instancePluginFieldErrors[field.key] || undefined}
            >
              {#if field.field_type === 'textarea'}
                <Textarea
                  id="ip-{field.key}"
                  bind:value={instancePluginFormValues[field.key]}
                  placeholder={field.placeholder}
                  required={field.required}
                  variant="mono"
                  rows={3}
                />
              {:else if field.field_type === 'select'}
                <Select
                  id="ip-{field.key}"
                  bind:value={instancePluginFormValues[field.key]}
                  options={resolvedOptions(field)}
                  placeholder="— select —"
                  required={field.required}
                  error={instancePluginFieldErrors[field.key] || undefined}
                />
              {:else if field.field_type === 'toggle'}
                <label class="flex items-center gap-2">
                  <Checkbox
                    id="ip-{field.key}"
                    checked={instancePluginFormValues[field.key] === 'true'}
                    onchange={(e) => {
                      instancePluginFormValues[field.key] = String(
                        (e.target as HTMLInputElement).checked
                      );
                    }}
                  />
                  <span class="text-sm">{field.help_text ?? ''}</span>
                </label>
              {:else}
                <Input
                  id="ip-{field.key}"
                  type={field.field_type === 'password' ? 'password' : 'text'}
                  bind:value={instancePluginFormValues[field.key]}
                  placeholder={field.placeholder}
                  required={field.required}
                  error={instancePluginFieldErrors[field.key] || undefined}
                />
              {/if}
            </FormFieldRow>
          {/if}
        {/each}
      </div>
      {#snippet footer()}
        <Button variant="secondary" onclick={closeInstancePluginConfigModal}>Cancel</Button>
        <Button variant="primary" onclick={saveInstancePluginConfig}>Save</Button>
      {/snippet}
    </ModalShell>
  {/if}
{/if}
```

(Reuses `isFieldVisible`, `resolvedOptions`, `flattenConfig`, `unflattenConfig`, `requiredFieldErrors`, `Callout` already in the file.)

- [ ] **Step 7: Run lint, format check, and type check**

```bash
cd frontend && npm run lint && npm run format:check && npm run check 2>&1 | tail -20
```

If `format:check` fails: `cd frontend && npm run format`.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/routes/settings/PluginConfigsTab.svelte
git commit -m "feat(frontend): Instance Plugins section in Plugin Configs tab

New SectionCard rendered for canManageGlobalSettings users at top of the
tab. Toggle (with restart-required confirm copy) + Edit Settings (only
when has_instance_config). Pending restart badge when stored != running.
Reuses every existing shared component per frontend AGENTS rules."
```

---

## Task 5: Frontend tests for the new section

**Files:**

- Modify (or create): `frontend/src/routes/settings/PluginConfigsTab.test.ts`

- [ ] **Step 1: Locate existing test pattern in the file (or sibling Svelte tests)**

```bash
ls frontend/src/routes/settings/*.test.ts 2>/dev/null
grep -rn "vitest\|@testing-library/svelte" frontend/package.json frontend/vite.config.ts 2>/dev/null | head
```

Match the existing harness — likely Vitest + `@testing-library/svelte`.

- [ ] **Step 2: Write four tests**

```ts
import { render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";
import PluginConfigsTab from "./PluginConfigsTab.svelte";
import * as api from "$lib/api";
import { Permission } from "$lib/types";

vi.mock("$lib/api");
vi.mock("$lib/auth.svelte", () => ({
  getUser: () => ({
    permissions: [Permission.ManageGlobalSettings, Permission.ViewSoftware],
  }),
}));

describe("PluginConfigsTab — Instance Plugins section", () => {
  it("renders the section when user has ManageGlobalSettings", async () => {
    vi.mocked(api.listInstancePlugins).mockResolvedValue([
      {
        plugin_type: "enhancement_dashboard_icons",
        display_name: "Dashboard Icons",
        enabled: false,
        running_enabled: false,
        has_instance_config: false,
        instance_config_form_fields: [],
        type_settings_form_fields: [],
        current_config: {},
        updated_at: null,
      },
    ]);
    render(PluginConfigsTab);
    expect(await screen.findByText("Instance Plugins")).toBeTruthy();
    expect(await screen.findByText("Dashboard Icons")).toBeTruthy();
  });

  it("does not render the section when user lacks ManageGlobalSettings", async () => {
    vi.doMock("$lib/auth.svelte", () => ({
      getUser: () => ({ permissions: [Permission.ViewSoftware] }),
    }));
    render(PluginConfigsTab);
    expect(screen.queryByText("Instance Plugins")).toBeNull();
  });

  it("shows Pending restart badge when stored != running", async () => {
    vi.mocked(api.listInstancePlugins).mockResolvedValue([
      {
        plugin_type: "enhancement_dashboard_icons",
        display_name: "Dashboard Icons",
        enabled: true, // stored
        running_enabled: false, // catalog snapshot from boot
        has_instance_config: false,
        instance_config_form_fields: [],
        type_settings_form_fields: [],
        current_config: {},
        updated_at: "2026-05-10T00:00:00Z",
      },
    ]);
    render(PluginConfigsTab);
    expect(await screen.findByText("Pending restart")).toBeTruthy();
  });

  it("does not show Edit Settings button when has_instance_config is false", async () => {
    vi.mocked(api.listInstancePlugins).mockResolvedValue([
      {
        plugin_type: "enhancement_dashboard_icons",
        display_name: "Dashboard Icons",
        enabled: false,
        running_enabled: false,
        has_instance_config: false,
        instance_config_form_fields: [],
        type_settings_form_fields: [],
        current_config: {},
        updated_at: null,
      },
    ]);
    render(PluginConfigsTab);
    await screen.findByText("Dashboard Icons");
    expect(screen.queryByText("Edit Settings")).toBeNull();
  });
});
```

- [ ] **Step 3: Run tests**

```bash
cd frontend && npm run test PluginConfigsTab.test.ts -- --run
```

Expected: 4 passed.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/settings/PluginConfigsTab.test.ts
git commit -m "test(frontend): Instance Plugins section render + badge

Four tests: renders for ManageGlobalSettings, hidden otherwise, Pending
restart badge appears on enabled!=running_enabled drift, Edit Settings
button hidden when has_instance_config=false."
```

---

## Task 6: Update `docs/development/plugin-guidelines.md`

**Files:**

- Modify: `docs/development/plugin-guidelines.md`

- [ ] **Step 1: Inspect existing structure**

```bash
grep -n "^## " docs/development/plugin-guidelines.md
```

Find a logical insertion point — likely after "Plugin Capabilities" or "Plugin Families".

- [ ] **Step 2: Insert new section "Plugin Scopes (Tenant vs Instance)"**

```markdown
## Plugin Scopes (Tenant vs Instance)

Every plugin declares a `scope: PluginScope` (defaults to `Tenant` if omitted in `declare_plugin!`). The scope determines who manages the plugin and
where its configuration lives.

| Aspect                   | `Tenant` (default)                               | `Instance`                                                     |
| ------------------------ | ------------------------------------------------ | -------------------------------------------------------------- |
| Who configures it        | Tenant Operators                                 | Instance owners (`ManageGlobalSettings` only)                  |
| Per-tenant override      | Via `plugin_type_settings` (existing)            | Via `plugin_type_settings` (still allowed when enabled)        |
| Instance-wide knobs      | None                                             | Optional via `instance_config: Some(&MY_INSTANCE_CONFIG_OPS)`  |
| Storage when disabled    | N/A — tenant disables via `plugin_type_settings` | Row absent (or `enabled = false`) in `instance_plugin_setting` |
| Visibility when disabled | Always visible                                   | Hidden from tenants — predicate in `crate::visibility`         |
| Toggling                 | Per-tenant, no restart                           | Instance-wide; **controller restart required** in v1           |

### When to choose `Instance`

Promote a plugin to `Instance` only when **all** are true:

1. The kill switch is meaningful at instance level — flipping it should affect every tenant simultaneously, not be a per-tenant choice.
2. Tenants seeing the plugin's existence (in pickers, surfaces, audit log targets) is undesirable when the instance owner has not opted in.
3. The plugin emits no `AdminEvent` directly, has no agent role, does not publish to MQTT, and does not appear in OpenAPI schema enums. (See the
   leakage vectors checklist in the spec.)

If only #1 is true, prefer extending the per-tenant `type_settings` schema with a kill-switch field; tenant Operators retain control without the
visibility gate.

### Two config surfaces

An Instance-Scoped Plugin can declare both `instance_config` (instance-owner-managed shared knobs) and `type_settings` (per-tenant behavior). They are
independent storage and independent UI rows. The instance owner sees both in the Settings tab — Instance Plugins section for instance config, Type
Defaults section for type_settings. Tenant Operators only see the Type Defaults row, and only when the instance is enabled.

### Restart-required

Toggling `enabled` writes the row but **does not** affect the running catalog. The controller reads `instance_plugin_setting` only at boot. The
`InstancePluginSummary` API response carries `running_enabled` (catalog snapshot) alongside `enabled` (DB state); the UI shows a "Pending restart"
badge when the two differ.

### Leakage vectors checklist

Spec §6 lists every channel an instance-scoped plugin's existence could leak through. **Run that checklist for every new instance-scoped plugin** and
document the result in the plugin's README. The visibility predicate covers HTTP and surfaces; everything else is the plugin author's responsibility.

See also:

- ADR `docs/adr/0006-instance-scoped-plugins.md`
- Spec `docs/superpowers/specs/2026-05-10-instance-scoped-plugins-design.md`
```

- [ ] **Step 3: Verify markdownlint**

```bash
markdownlint --config .markdownlint.json docs/development/plugin-guidelines.md
```

If lint errors:

```bash
npx prettier --write --prose-wrap always --print-width 150 docs/development/plugin-guidelines.md
```

- [ ] **Step 4: Commit**

```bash
git add docs/development/plugin-guidelines.md
git commit -m "docs(plugin-guidelines): Plugin Scopes section

When to choose Instance vs Tenant; two-surface model (instance_config vs
type_settings); restart-required + Pending restart badge; leakage vectors
checklist reference."
```

---

## Task 7: Update `ARCHITECTURE.md`

**Files:**

- Modify: `ARCHITECTURE.md`

- [ ] **Step 1: Locate the Plugin section**

```bash
grep -n "^## \|^### " ARCHITECTURE.md | head -30
```

- [ ] **Step 2: Add a paragraph about Plugin Scopes**

Insert under the existing Plugin section (or wherever plugin lifecycle is described):

```markdown
### Plugin Scopes

Every Plugin has a scope (`Tenant` — default — or `Instance`). Tenant-scoped plugins are configured per-tenant via `plugin_configs` and
`plugin_type_settings`. Instance-scoped plugins (e.g. `enhancement_dashboard_icons`) are configured exclusively by Operators with
`ManageGlobalSettings`; their state lives in `instance_plugin_setting` and is loaded once at controller boot. When an instance-scoped plugin is
disabled, its singleton is never constructed and tenant Operators see no evidence of its existence (route handlers, surfaces, and SSE filtered via a
single visibility predicate).

See ADR `docs/adr/0006-instance-scoped-plugins.md` and the development guide `docs/development/plugin-guidelines.md`.
```

- [ ] **Step 3: Verify markdownlint**

```bash
markdownlint --config .markdownlint.json ARCHITECTURE.md
npx prettier --write --prose-wrap always --print-width 150 ARCHITECTURE.md
```

- [ ] **Step 4: Commit**

```bash
git add ARCHITECTURE.md
git commit -m "docs(architecture): brief mention of Plugin Scopes"
```

---

## Task 8: Update end-user dashboard-icons doc

**Files:**

- Modify: `website/public/docs/end-user/dashboard-icons/index.html`

- [ ] **Step 1: Open the file and find the body content**

```bash
grep -n "<h1>\|<h2>\|<p>" website/public/docs/end-user/dashboard-icons/index.html | head
```

- [ ] **Step 2: Update the page**

Add or replace the relevant prose to convey:

```html
<p>
  <strong>Disabled by default.</strong> Dashboard Icons is now an
  instance-scoped enhancement: it must be enabled by an instance owner from
  <em>Settings → Plugin Configs → Instance Plugins</em>. After enabling, the
  controller must be restarted for the change to take effect; the Settings UI
  shows a <em>Pending restart</em> badge until then.
</p>
<p>
  <strong>Tenant-side opt-out is preserved.</strong> Once an instance owner
  enables Dashboard Icons, individual tenants can still disable enrichment for
  their own software items via
  <em>Settings → Plugin Configs → Type Defaults</em>.
</p>
<p>
  <strong>Existing icons remain.</strong> If your installation previously used
  Dashboard Icons, software items already enriched with icons of the form
  <code>https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/...</code> will
  retain those URLs after the conversion. Disabling the plugin stops new
  enrichments but does not retroactively wipe historical ones.
</p>
```

- [ ] **Step 3: Verify the HTML lints / opens correctly**

```bash
xmllint --html --noout website/public/docs/end-user/dashboard-icons/index.html 2>&1 | head
```

(If `xmllint` not available, just open in a browser to eyeball.)

- [ ] **Step 4: Commit**

```bash
git add website/public/docs/end-user/dashboard-icons/index.html
git commit -m "docs(end-user): dashboard-icons disabled-by-default + restart note

Documents the instance-scoped behavior, the Pending restart badge,
tenant-side opt-out preservation, and the persistence of historical
CDN URLs on icon_url after disable."
```

---

## Task 9: Create admin guide page for Instance Plugins

**Files:**

- Create: `docs/admin/instance-plugins.md`
- Create (if absent): `docs/admin/README.md`

- [ ] **Step 1: Confirm whether `docs/admin/` already exists**

```bash
ls docs/admin/ 2>&1 | head
```

If absent: create directory + an index `README.md`:

```markdown
# Administrator Documentation

Guides for instance owners (Operators with `ManageGlobalSettings`).

- [Instance Plugins](instance-plugins.md) — enabling, configuring, and troubleshooting instance-scoped plugins (e.g. Dashboard Icons).
```

- [ ] **Step 2: Write the admin guide**

```markdown
# Instance Plugins

This page is for instance owners — Operators holding the `ManageGlobalSettings` permission. Tenant Operators do not see Instance Plugins in the
Settings tab.

## What is an Instance-Scoped Plugin?

Most plugins are configured per tenant. **Instance-Scoped Plugins** are configured at the instance level: you enable them once for the entire
instance, and tenant Operators see no evidence of the plugin's existence until you do.

The first instance-scoped plugin is **Dashboard Icons**, which fetches icon URLs from the public Dashboard Icons CDN to enrich software items.

## Enabling an instance plugin

1. Open **Settings → Plugin Configs → Instance Plugins** (the section is visible only to instance owners).
2. Locate the plugin row.
3. Click **Enable**.
4. Read the confirmation dialog — it reminds you that a controller restart is required.
5. Confirm.
6. **Restart the controller.** The action does not take effect until then.

While the change is pending, the row shows a **Pending restart** badge.

## Editing instance configuration

Plugins that expose instance-wide configuration knobs (currently none — Dashboard Icons is kill-switch only) show an **Edit Settings** button next to
the toggle. The form schema is supplied by the plugin descriptor and rendered using the same field types as the rest of Plugin Configs.

## Auditing toggles

Two new audit actions are emitted when you change instance plugin state:

- `instance_plugin.toggled` — whenever you enable or disable a plugin. Details include `previous_enabled` and `new_enabled`.
- `instance_plugin.config_upserted` — whenever you save a configuration. Raw config fields are not included in the audit details (only the field
  count).

Both events are visible in the system-level audit log to anyone with `ViewSystemAuditLogs`.

## Tenant-side opt-out

Once an instance plugin is enabled, individual tenants may still disable it for their own scope via **Settings → Plugin Configs → Type Defaults**. The
two settings are independent — an instance-disabled plugin is invisible to tenants entirely; an instance-enabled plugin is visible and tenant
Operators choose whether to opt in or out per their own tenant.

## Disabling an instance plugin

Disabling stops new operations from the plugin (no new lifecycle hooks fire, no new background tasks run after the next restart). It does **not**
retroactively undo persisted side effects.

For Dashboard Icons specifically, software items previously enriched with URLs of the form
`https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/...` keep those URLs. There is no provenance column to safely identify which icons came from
this plugin, so a wipe is not offered.

## Troubleshooting

**Q: I enabled the plugin but nothing happens.** A: Did you restart the controller? Check the **Pending restart** badge — if it's still showing, the
controller is still running the old (disabled) catalog.

**Q: A tenant says they don't see the plugin in their Type Defaults section.** A: Confirm the plugin is enabled at the instance level. If it is, check
the controller logs for any error during plugin construction. Disabled instance plugins are intentionally invisible to tenants.

**Q: I want to wipe Dashboard Icons enrichments from existing software items.** A: Not supported in v1 — there is no plugin-origin tracking on
`software_item.icon_url`. Manual cleanup via the Software Items UI is the only option today.

## See also

- ADR `docs/adr/0006-instance-scoped-plugins.md` — architectural context.
- Spec `docs/superpowers/specs/2026-05-10-instance-scoped-plugins-design.md` — the original design document.
- Plugin developer guide `docs/development/plugin-guidelines.md` — for writing new instance-scoped plugins.
```

- [ ] **Step 3: Lint the markdown**

```bash
markdownlint --config .markdownlint.json docs/admin/instance-plugins.md docs/admin/README.md 2>&1 | head
npx prettier --write --prose-wrap always --print-width 150 docs/admin/instance-plugins.md docs/admin/README.md
```

- [ ] **Step 4: Commit**

```bash
git add docs/admin/instance-plugins.md docs/admin/README.md
git commit -m "docs(admin): instance plugins guide for instance owners

How to enable, edit, audit, and disable instance-scoped plugins. Q&A for
common confusion: Pending restart badge, tenant invisibility, lack of
icon_url provenance wipe."
```

---

## Task 10: Quality gates checkpoint

**Files:** none (verification only)

- [ ] **Step 1: Frontend gates**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: all green, build artifact written.

- [ ] **Step 2: Markdown lint sweep**

```bash
markdownlint --config .markdownlint.json '**/*.md'
```

If any lint errors, fix with `npx prettier --write --prose-wrap always --print-width 150 <file>` and re-run.

- [ ] **Step 3: Cargo full sweep (re-run because frontend build embedded into controller via `rust-embed` per frontend AGENTS — controller binary must
      rebuild against the new bundle)**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo deny check
```

- [ ] **Step 4: End-to-end smoke**

```bash
RUST_LOG=info cargo run -p uptrakit-controller --all-features -- \
    --master-key-file <test-key>
```

(Open the dev server, log in as admin, navigate to Settings → Plugin Configs, confirm "Instance Plugins" section renders, toggle Dashboard Icons → see
"Pending restart" badge appear, restart controller, see badge clear and the catalog construct dashboard-icons.)

- [ ] **Step 5: No commit needed.**

If everything is green, all three plans (A + B + C) are done. Open a PR with the conventional title
`feat(plugins): instance-scoped plugins (dashboard-icons opt-in)` and the spec + ADR linked in the body.

---

## Self-review

Plan C vs spec:

- **Spec §5 (frontend section, Pending restart badge, modal reuse):** Tasks 2, 3, 4, 5.
- **Spec §10 (documentation deliverables):**
  - CONTEXT.md — ✅ done in grilling session, listed in spec §10 as completed.
  - ADR `docs/adr/0006-instance-scoped-plugins.md` — ✅ Plan B Task 13.
  - `docs/development/plugin-guidelines.md` — ✅ Task 6.
  - `ARCHITECTURE.md` — ✅ Task 7.
  - `website/public/docs/end-user/dashboard-icons` — ✅ Task 8.
  - New admin guide page — ✅ Task 9 (`docs/admin/instance-plugins.md`).
  - OpenAPI auto-generated — ✅ Plan B Task 5 (utoipa annotations on handlers).
- **Quality gates:** Task 10.

Snapshot conformance per task:

- Frontend tasks reference `frontend/AGENTS.md` rules: API via `$lib/api.ts` (Task 3), reuse shared components (Task 4), TS strict + no `any` (Task
  2), Svelte 5 runes (Task 4 state + derived).
- Doc tasks use prettier `--prose-wrap always --print-width 150` to satisfy `MD013` per CLAUDE memory rule ("Use prettier for markdown formatting").
  `MD049` emphasis style — use asterisks, not underscores.
- No `#[allow(...)]`. No "silence the lint" tasks. No fights with framework.

Doc tasks are first-class — not bundled with frontend or "polish."

Across all three plans, every spec deliverable from §10 has at least one task; every behavior described in §1–§9 has at least one task.
