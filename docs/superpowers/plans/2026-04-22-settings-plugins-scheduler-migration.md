# Settings Plugins/Scheduler/System-Services Button Migration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate every interactive button in five settings sub-components to render through the `<Button>` primitive.

**Architecture:** Template-level attribute migration only — no runtime behavior changes. Each file is a standalone commit. Unit tests extended per file.

**Tech Stack:** Svelte 5, TypeScript, `<Button>` from `$lib/components/Button.svelte`, Vitest + @testing-library/svelte

---

## File Map

| File | Unmigrated buttons | Import needed |
| --- | --- | --- |
| `frontend/src/routes/settings/PluginConfigsTab.svelte` | 17 raw `<button>` elements across 3 modals and 3 table action columns | Add `import Button from '$lib/components/Button.svelte'` |
| `frontend/src/routes/settings/SchedulerTab.svelte` | 4 raw `<button>` elements: Retry, Edit, Run, Cancel, Save | Add import |
| `frontend/src/routes/settings/SystemServicesSettings.svelte` | 6 raw `<button>` elements: Load Tokens, Refresh, Create Token launcher, Copy, modal Cancel, modal Create; Revoke | Add import |
| `frontend/src/routes/settings/EnrollmentTokenSettings.svelte` | 5 raw `<button>` elements; Revoke already migrated | Import already present |
| `frontend/src/routes/settings/AgentCertificateSettings.svelte` | 1 raw `<button>`; introduce `saving` state | Add import |

Test files to create:

- `frontend/src/routes/settings/PluginConfigsTab.test.ts`
- `frontend/src/routes/settings/SchedulerTab.test.ts`
- `frontend/src/routes/settings/SystemServicesSettings.test.ts`
- `frontend/src/routes/settings/EnrollmentTokenSettings.test.ts`
- `frontend/src/routes/settings/AgentCertificateSettings.test.ts`

---

## Variant reference

| Old Skeleton class | Button prop |
| --- | --- |
| `preset-filled-primary-500` | `variant="primary"` |
| `preset-filled-error-500` | `variant="danger"` |
| `preset-tonal-error` | `variant="danger"` |
| `preset-filled-error-500` | `variant="danger"` |
| `preset-tonal` / `preset-tonal-surface` | `variant="secondary"` |
| `btn-sm` size modifier | `size="sm"` |

Remove `class="btn ..."` entirely. Keep all other props (`onclick`, `disabled`, `aria-*`). When source had a
text-swap expression (`{saving ? 'Saving...' : 'Save'}`), remove the swap and pass `loading={saving}` — Button
renders its own spinner. When `disabled` was set to `disabled={saving || someOtherCondition}`, the `saving` part
moves into `loading={saving}`; keep only the non-saving gate in `disabled`.

---

## Task 1: Migrate `PluginConfigsTab.svelte`

**Files:**

- Modify: `frontend/src/routes/settings/PluginConfigsTab.svelte`
- Create: `frontend/src/routes/settings/PluginConfigsTab.test.ts`

### Context

Full button inventory (verified with Grep):

| Line (approx) | Label | Old class | Target |
| --- | --- | --- | --- |
| 672 | Add Config | `btn preset-filled-primary-500` | `variant="primary"` |
| 809 | Edit (config row) | `btn btn-sm preset-tonal` | `variant="secondary" size="sm"` |
| 812–818 | Discover (config row) | `btn btn-sm preset-tonal` + text-swap `'...' : 'Discover'` | `variant="secondary" size="sm" loading={discoveringId === config.id}`, static text `Discover` |
| 821–826 | Delete (config row) | `btn btn-sm preset-tonal-error` | `variant="danger" size="sm"` |
| 885 | Add Plugin Type | `btn preset-filled-primary-500` | `variant="primary"` |
| 962–965 | Remove (allowlist row) | `btn btn-sm preset-tonal-error` | `variant="danger" size="sm"` |
| 1067–1068 | Edit (type settings row) | `btn btn-sm preset-tonal` | `variant="secondary" size="sm"` |
| 1071–1075 | Reset (type settings row) | `btn btn-sm preset-tonal-error` | `variant="danger" size="sm"` |
| 1202–1212 | Advanced: Edit as JSON | `btn btn-sm preset-tonal` | `variant="secondary" size="sm"` |
| 1234–1248 | Return to form editor | `btn btn-sm preset-tonal` | `variant="secondary" size="sm"` |
| 1304 | Cancel (config modal footer) | `btn preset-tonal-surface` | `variant="secondary"` |
| 1306–1312 | Test (config modal footer) | `btn preset-tonal` + text-swap `'Testing...' : 'Test'` | `variant="secondary" loading={configTesting} disabled={!configForm.plugin_type}`, static text `Test` |
| 1314–1316 | Create/Update (config modal footer) | `btn preset-filled-primary-500` | `variant="primary"` |
| 1356 | Cancel (allowlist modal footer) | `btn preset-tonal-surface` | `variant="secondary"` |
| 1357 | Add (allowlist modal footer) | `btn preset-filled-primary-500` | `variant="primary"` |
| 1448 | Cancel (type settings modal footer) | `btn preset-tonal-surface` | `variant="secondary"` |
| 1449 | Save (type settings modal footer) | `btn preset-filled-primary-500` | `variant="primary"` |

No `saving` state introduction needed — no async button in this file lacks a loading path that is currently missed;
Discover and Test already have their flags.

- [ ] **Step 1.1: Write failing test**

Create `frontend/src/routes/settings/PluginConfigsTab.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
  getPluginConfigs: vi.fn().mockResolvedValue({ items: [], total: 0, page: 1, per_page: 20, pages: 0 }),
  createPluginConfig: vi.fn(),
  updatePluginConfig: vi.fn(),
  deletePluginConfig: vi.fn(),
  triggerPluginConfigDiscovery: vi.fn(),
  listDiscoveryAllowlist: vi.fn().mockResolvedValue({ items: [], total: 0, page: 1, per_page: 20, pages: 0 }),
  addDiscoveryAllowlistEntry: vi.fn(),
  deleteDiscoveryAllowlistEntry: vi.fn(),
  listPluginTypes: vi.fn().mockResolvedValue([]),
  batchPluginConfigs: vi.fn(),
  listPluginTypeSettings: vi.fn().mockResolvedValue([]),
  upsertPluginTypeSettings: vi.fn(),
  deletePluginTypeSettings: vi.fn(),
  testPluginConfig: vi.fn()
}));
vi.mock('$lib/auth.svelte', () => ({
  getUser: vi.fn(() => ({
    id: 'u1',
    email: 'a@b.com',
    first_name: 'A',
    last_name: 'B',
    permissions: ['manage_plugin_configs', 'trigger_discovery', 'manage_allowlist', 'manage_plugin_type_settings']
  }))
}));
vi.mock('$lib/notifications.svelte', () => ({ showSuccess: vi.fn(), showError: vi.fn() }));
vi.mock('$lib/stores/events.svelte', () => ({ subscribe: vi.fn(() => () => {}), getLastEvent: vi.fn(() => null) }));

import { Permission } from '$lib/types';
import * as auth from '$lib/auth.svelte';
import PluginConfigsTab from './PluginConfigsTab.svelte';

describe('PluginConfigsTab button variants', () => {
  it('has no raw preset-filled-primary-500 buttons', async () => {
    vi.mocked(auth.getUser).mockReturnValue({
      id: 'u1', email: 'a@b.com', first_name: 'A', last_name: 'B',
      permissions: [Permission.ManagePluginConfigs, Permission.TriggerDiscovery,
                    Permission.ManageDiscoveryAllowlist, Permission.ManagePluginTypeSettings]
    } as ReturnType<typeof auth.getUser>);
    const { container } = render(PluginConfigsTab);
    await waitFor(() => expect(container.querySelector('button.preset-filled-primary-500')).toBeNull());
  });

  it('has no raw preset-tonal-error buttons', async () => {
    const { container } = render(PluginConfigsTab);
    await waitFor(() => expect(container.querySelector('button.preset-tonal-error')).toBeNull());
  });

  it('has no raw preset-tonal-surface buttons', async () => {
    const { container } = render(PluginConfigsTab);
    await waitFor(() => expect(container.querySelector('button.preset-tonal-surface')).toBeNull());
  });

  it('Add Config button has primary gradient class', async () => {
    const { container } = render(PluginConfigsTab);
    await waitFor(() => {
      const btn = Array.from(container.querySelectorAll('button')).find(
        (b) => b.textContent?.trim() === 'Add Config'
      );
      expect(btn?.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
    });
  });
});
```

- [ ] **Step 1.2: Run test — expect FAIL**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/PluginConfigsTab.test.ts 2>&1 | tail -20
```

Expected: failures — raw preset classes found, Add Config does not have gradient class.

- [ ] **Step 1.3: Add Button import to `PluginConfigsTab.svelte`**

After line 36 (`} from '$lib/components/ui';`), add:

```svelte
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 1.4: Replace Add Config button (line ~672)**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={openCreateConfig}>Add Config</button>
```

New:

```svelte
<Button variant="primary" onclick={openCreateConfig}>Add Config</Button>
```

- [ ] **Step 1.5: Replace config row action buttons (lines ~809–826)**

Old Edit:

```svelte
<button class="btn btn-sm preset-tonal" onclick={() => openEditConfig(config)}>Edit</button>
```

New:

```svelte
<Button variant="secondary" size="sm" onclick={() => openEditConfig(config)}>Edit</Button>
```

Old Discover:

```svelte
<button
  class="btn btn-sm preset-tonal"
  disabled={discoveringId === config.id}
  onclick={() => triggerDiscover(config)}
>
  {discoveringId === config.id ? '...' : 'Discover'}
</button>
```

New:

```svelte
<Button
  variant="secondary"
  size="sm"
  loading={discoveringId === config.id}
  onclick={() => triggerDiscover(config)}
>Discover</Button>
```

Old Delete:

```svelte
<button
  class="btn btn-sm preset-tonal-error"
  onclick={() => (configDeleteConfirm = { id: config.id, name: config.name })}
>
  Delete
</button>
```

New:

```svelte
<Button
  variant="danger"
  size="sm"
  onclick={() => (configDeleteConfirm = { id: config.id, name: config.name })}
>Delete</Button>
```

- [ ] **Step 1.6: Replace Add Plugin Type button (line ~885)**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={openAddAllowlistEntry}>Add Plugin Type</button>
```

New:

```svelte
<Button variant="primary" onclick={openAddAllowlistEntry}>Add Plugin Type</Button>
```

- [ ] **Step 1.7: Replace allowlist row Remove button (lines ~962–965)**

Old:

```svelte
<button
  class="btn btn-sm preset-tonal-error"
  onclick={() => (allowlistDeleteConfirm = { id: entry.id, plugin_type: entry.plugin_type })}
>
  Remove
</button>
```

New:

```svelte
<Button
  variant="danger"
  size="sm"
  onclick={() => (allowlistDeleteConfirm = { id: entry.id, plugin_type: entry.plugin_type })}
>Remove</Button>
```

- [ ] **Step 1.8: Replace type settings row buttons (lines ~1067–1075)**

Old Edit:

```svelte
<button class="btn btn-sm preset-tonal" onclick={() => openEditTypeSettings(t.plugin_type)}
  >Edit</button
>
```

New:

```svelte
<Button variant="secondary" size="sm" onclick={() => openEditTypeSettings(t.plugin_type)}>Edit</Button>
```

Old Reset:

```svelte
<button
  class="btn btn-sm preset-tonal-error"
  onclick={() => (typeSettingsResetConfirm = t.plugin_type)}
>
  Reset
</button>
```

New:

```svelte
<Button
  variant="danger"
  size="sm"
  onclick={() => (typeSettingsResetConfirm = t.plugin_type)}
>Reset</Button>
```

- [ ] **Step 1.9: Replace editor mode toggle buttons (lines ~1202–1248)**

Old "Advanced: Edit as JSON":

```svelte
<button
  type="button"
  class="btn btn-sm preset-tonal"
  onclick={() => {
    configForm.config = JSON.stringify(unflattenConfig(formValues, currentFormFields), null, 2);
    configJsonError = '';
    showJsonEditor = true;
  }}
>
  Advanced: Edit as JSON
</button>
```

New:

```svelte
<Button
  variant="secondary"
  size="sm"
  onclick={() => {
    configForm.config = JSON.stringify(unflattenConfig(formValues, currentFormFields), null, 2);
    configJsonError = '';
    showJsonEditor = true;
  }}
>Advanced: Edit as JSON</Button>
```

Old "Return to form editor":

```svelte
<button
  type="button"
  class="btn btn-sm preset-tonal"
  onclick={() => {
    try {
      const parsed = JSON.parse(configForm.config || '{}');
      formValues = flattenConfig(parsed, currentFormFields);
      configFieldErrors = {};
      configJsonError = '';
      showJsonEditor = false;
    } catch {
```

New (preserve the full onclick body, only change the wrapper element):

```svelte
<Button
  variant="secondary"
  size="sm"
  onclick={() => {
    try {
      const parsed = JSON.parse(configForm.config || '{}');
      formValues = flattenConfig(parsed, currentFormFields);
      configFieldErrors = {};
      configJsonError = '';
      showJsonEditor = false;
    } catch {
```

Keep closing `}}>` and child text content unchanged; replace the closing `</button>` with `</Button>`.

- [ ] **Step 1.10: Replace config modal footer buttons (lines ~1304–1316)**

Old Cancel:

```svelte
<button class="btn preset-tonal-surface" onclick={closeConfigModal}>Cancel</button>
```

New:

```svelte
<Button variant="secondary" onclick={closeConfigModal}>Cancel</Button>
```

Old Test:

```svelte
<button
  class="btn preset-tonal"
  disabled={configTesting || !configForm.plugin_type}
  onclick={testCurrentConfig}
>
  {configTesting ? 'Testing...' : 'Test'}
</button>
```

New:

```svelte
<Button
  variant="secondary"
  loading={configTesting}
  disabled={!configForm.plugin_type}
  onclick={testCurrentConfig}
>Test</Button>
```

Old Create/Update:

```svelte
<button class="btn preset-filled-primary-500" onclick={saveConfig}>
  {editingConfig ? 'Update' : 'Create'}
</button>
```

New:

```svelte
<Button variant="primary" onclick={saveConfig}>
  {editingConfig ? 'Update' : 'Create'}
</Button>
```

- [ ] **Step 1.11: Replace allowlist modal footer buttons (lines ~1356–1357)**

Old:

```svelte
<button class="btn preset-tonal-surface" onclick={closeAllowlistModal}>Cancel</button>
<button class="btn preset-filled-primary-500" onclick={saveAllowlistEntry}>Add</button>
```

New:

```svelte
<Button variant="secondary" onclick={closeAllowlistModal}>Cancel</Button>
<Button variant="primary" onclick={saveAllowlistEntry}>Add</Button>
```

- [ ] **Step 1.12: Replace type settings modal footer buttons (lines ~1448–1449)**

Old:

```svelte
<button class="btn preset-tonal-surface" onclick={closeTypeSettingsModal}>Cancel</button>
<button class="btn preset-filled-primary-500" onclick={saveTypeSettings}>Save</button>
```

New:

```svelte
<Button variant="secondary" onclick={closeTypeSettingsModal}>Cancel</Button>
<Button variant="primary" onclick={saveTypeSettings}>Save</Button>
```

- [ ] **Step 1.13: Run type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 1.14: Run tests — expect PASS**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/PluginConfigsTab.test.ts 2>&1 | tail -10
```

Expected: 4 passed.

- [ ] **Step 1.15: Run full suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: all tests pass, 0 failures.

- [ ] **Step 1.16: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/PluginConfigsTab.svelte src/routes/settings/PluginConfigsTab.test.ts
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/PluginConfigsTab.svelte frontend/src/routes/settings/PluginConfigsTab.test.ts
git commit -m "feat(ui): migrate PluginConfigsTab to Button primitive (#3d step 1)"
```

---

## Task 2: Migrate `SchedulerTab.svelte`

**Files:**

- Modify: `frontend/src/routes/settings/SchedulerTab.svelte`
- Create: `frontend/src/routes/settings/SchedulerTab.test.ts`

### Context

Full button inventory (verified from source):

| Line (approx) | Label | Old class | Target |
| --- | --- | --- | --- |
| 102 | Retry | `btn preset-filled-primary-500 mt-2` | `variant="primary"` — keep `mt-2` via `class="mt-2"` on Button |
| 147 | Edit (row) | `btn btn-sm preset-tonal` | `variant="secondary" size="sm"` |
| 148–154 | Run (row) | `btn btn-sm preset-tonal` + text-swap `'...' : 'Run'` | `variant="ghost" size="sm" loading={triggeringId === task.id} disabled={task.is_running}`, static text `Run` |
| 184 | Cancel (modal) | `btn preset-tonal-surface` | `variant="secondary"` |
| 185–187 | Save (modal) | `btn preset-filled-primary-500` + text-swap `'Saving...' : 'Save'` | `variant="primary" loading={saving}`, static text `Save` |

`saving` and `triggeringId` state variables already exist in the script block. No new state needed.

The Retry button currently has `mt-2` in its class. Button accepts a `class` prop — pass `class="mt-2"` to preserve layout.

- [ ] **Step 2.1: Write failing test**

Create `frontend/src/routes/settings/SchedulerTab.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
  listSchedulerTasks: vi.fn().mockResolvedValue([
    {
      id: 'task-1',
      name: 'Test Task',
      cron_expression: '0 * * * *',
      is_enabled: true,
      is_running: false,
      last_run_at: null,
      next_run_at: null,
      interval_seconds: 3600,
      jitter_seconds: 0
    }
  ]),
  updateSchedulerTask: vi.fn(),
  triggerSchedulerTask: vi.fn()
}));
vi.mock('$lib/auth.svelte', () => ({
  getUser: vi.fn(() => ({
    id: 'u1', email: 'a@b.com', first_name: 'A', last_name: 'B',
    permissions: ['manage_scheduler']
  }))
}));
vi.mock('$lib/notifications.svelte', () => ({ showSuccess: vi.fn(), showError: vi.fn() }));

import * as api from '$lib/api';
import { Permission } from '$lib/types';
import * as auth from '$lib/auth.svelte';
import SchedulerTab from './SchedulerTab.svelte';

afterEach(() => vi.clearAllMocks());

function makeUser() {
  return {
    id: 'u1', email: 'a@b.com', first_name: 'A', last_name: 'B',
    permissions: [Permission.ManageScheduler]
  } as ReturnType<typeof auth.getUser>;
}

describe('SchedulerTab button variants', () => {
  it('has no raw preset-filled-primary-500 buttons', async () => {
    vi.mocked(auth.getUser).mockReturnValue(makeUser());
    const { container } = render(SchedulerTab);
    await waitFor(() => screen.getByText('Test Task'));
    expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
  });

  it('has no raw preset-tonal-surface buttons', async () => {
    vi.mocked(auth.getUser).mockReturnValue(makeUser());
    const { container } = render(SchedulerTab);
    await waitFor(() => screen.getByText('Test Task'));
    expect(container.querySelector('button.preset-tonal-surface')).toBeNull();
  });

  it('Run button has ghost (bg-transparent) class', async () => {
    vi.mocked(auth.getUser).mockReturnValue(makeUser());
    render(SchedulerTab);
    await waitFor(() => screen.getByText('Test Task'));
    const runBtn = screen.getByRole('button', { name: 'Run' });
    expect(runBtn.className).toContain('bg-transparent');
  });

  it('Save button carries aria-busy=true while saving', async () => {
    vi.mocked(auth.getUser).mockReturnValue(makeUser());
    let resolve!: (v: unknown) => void;
    vi.mocked(api.updateSchedulerTask).mockReturnValue(
      new Promise((r) => { resolve = r; })
    );
    render(SchedulerTab);
    await waitFor(() => screen.getByText('Test Task'));
    await fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
    const saveBtn = await screen.findByRole('button', { name: 'Save' });
    await fireEvent.click(saveBtn);
    await waitFor(() => expect(saveBtn).toHaveAttribute('aria-busy', 'true'));
    resolve({ id: 'task-1', name: 'Test Task', cron_expression: '0 * * * *', is_enabled: true,
               is_running: false, last_run_at: null, next_run_at: null, interval_seconds: 3600, jitter_seconds: 0 });
    await waitFor(() => expect(saveBtn).not.toHaveAttribute('aria-busy'));
  });

  it('Save button text is static "Save" during loading — no text swap', async () => {
    vi.mocked(auth.getUser).mockReturnValue(makeUser());
    let resolve!: (v: unknown) => void;
    vi.mocked(api.updateSchedulerTask).mockReturnValue(
      new Promise((r) => { resolve = r; })
    );
    render(SchedulerTab);
    await waitFor(() => screen.getByText('Test Task'));
    await fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
    const saveBtn = await screen.findByRole('button', { name: 'Save' });
    await fireEvent.click(saveBtn);
    await waitFor(() => expect(saveBtn).toHaveAttribute('aria-busy', 'true'));
    expect(saveBtn).toHaveTextContent('Save');
    resolve({});
  });
});
```

- [ ] **Step 2.2: Run test — expect FAIL**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/SchedulerTab.test.ts 2>&1 | tail -20
```

Expected: failures — raw preset classes found, aria-busy never set.

- [ ] **Step 2.3: Add Button import to `SchedulerTab.svelte`**

After line 10 (`import { Callout, FormFieldRow, SectionCard, StatusBadge } from '$lib/components/ui';`), add:

```svelte
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 2.4: Replace Retry button (line ~102)**

Old:

```svelte
<button class="btn preset-filled-primary-500 mt-2" onclick={loadTasks}>Retry</button>
```

New:

```svelte
<Button variant="primary" class="mt-2" onclick={loadTasks}>Retry</Button>
```

- [ ] **Step 2.5: Replace Edit row button (line ~147)**

Old:

```svelte
<button class="btn btn-sm preset-tonal" onclick={() => openEdit(task)}>Edit</button>
```

New:

```svelte
<Button variant="secondary" size="sm" onclick={() => openEdit(task)}>Edit</Button>
```

- [ ] **Step 2.6: Replace Run row button (lines ~148–154)**

Old:

```svelte
<button
  class="btn btn-sm preset-tonal"
  disabled={task.is_running || triggeringId === task.id}
  onclick={() => triggerNow(task)}
>
  {triggeringId === task.id ? '...' : 'Run'}
</button>
```

New:

```svelte
<Button
  variant="ghost"
  size="sm"
  loading={triggeringId === task.id}
  disabled={task.is_running}
  onclick={() => triggerNow(task)}
>Run</Button>
```

`task.is_running` is the only explicit disabled gate; the in-flight `triggeringId` check moves into `loading`.

- [ ] **Step 2.7: Replace Cancel modal button (line ~184)**

Old:

```svelte
<button class="btn preset-tonal-surface" onclick={closeEdit}>Cancel</button>
```

New:

```svelte
<Button variant="secondary" onclick={closeEdit}>Cancel</Button>
```

- [ ] **Step 2.8: Replace Save modal button (lines ~185–187)**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={saveEdit} disabled={saving}>
  {saving ? 'Saving...' : 'Save'}
</button>
```

New:

```svelte
<Button variant="primary" loading={saving} onclick={saveEdit}>Save</Button>
```

- [ ] **Step 2.9: Run type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 2.10: Run tests — expect PASS**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/SchedulerTab.test.ts 2>&1 | tail -10
```

Expected: 4 passed.

- [ ] **Step 2.11: Run full suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: all tests pass, 0 failures.

- [ ] **Step 2.12: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/SchedulerTab.svelte src/routes/settings/SchedulerTab.test.ts
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/SchedulerTab.svelte frontend/src/routes/settings/SchedulerTab.test.ts
git commit -m "feat(ui): migrate SchedulerTab to Button primitive (#3d step 2)"
```

---

## Task 3: Migrate `SystemServicesSettings.svelte`

**Files:**

- Modify: `frontend/src/routes/settings/SystemServicesSettings.svelte`
- Create: `frontend/src/routes/settings/SystemServicesSettings.test.ts`

### Context

Full button inventory (verified from source):

| Line (approx) | Label | Old class | Target |
| --- | --- | --- | --- |
| 184–186 | Load Tokens | `btn preset-filled-primary-500` + text-swap | `variant="primary" loading={loading}`, static text `Load Tokens` |
| 188–190 | Refresh | `btn preset-tonal` | `variant="secondary" loading={loading}` |
| 192–199 | Create Token (launcher) | `btn preset-filled-primary-500` | `variant="primary"` |
| 213–215 | Copy | `btn btn-sm preset-tonal flex-shrink-0` | `variant="ghost" size="sm" class="flex-shrink-0"` |
| 270–278 | Cancel (modal) | `btn preset-tonal` | `variant="secondary"` |
| 279–281 | Create (modal footer) | `btn preset-filled-primary-500` + text-swap | `variant="primary" loading={creating}`, static text `Create` |
| 370–372 | Revoke | `btn btn-sm preset-filled-error-500` | `variant="danger" size="sm"` |

Note: `SystemServicesSettings.svelte` does NOT currently have `import Button`. Add it.

`loading` is used on both Load Tokens and Refresh — pass `loading={loading}` so the spinner replaces the text-swap
on Load Tokens; for Refresh there is no text-swap so just `loading={loading}` adds visual feedback.

`creating` is used on the modal Create button — move into `loading={creating}`.

- [ ] **Step 3.1: Write failing test**

Create `frontend/src/routes/settings/SystemServicesSettings.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
  listSystemEnrollmentTokens: vi.fn().mockResolvedValue({ items: [], total: 0, page: 1, per_page: 20, pages: 0 }),
  createSystemEnrollmentToken: vi.fn(),
  revokeSystemEnrollmentToken: vi.fn()
}));

import * as api from '$lib/api';
import SystemServicesSettings from './SystemServicesSettings.svelte';

const props = { onSuccess: vi.fn(), onError: vi.fn() };

afterEach(() => vi.clearAllMocks());

describe('SystemServicesSettings button variants', () => {
  it('has no raw preset-filled-primary-500 buttons', () => {
    const { container } = render(SystemServicesSettings, props);
    expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
  });

  it('has no raw preset-filled-error-500 buttons', () => {
    const { container } = render(SystemServicesSettings, props);
    expect(container.querySelector('button.preset-filled-error-500')).toBeNull();
  });

  it('has no raw preset-tonal buttons', () => {
    const { container } = render(SystemServicesSettings, props);
    expect(container.querySelector('button.preset-tonal')).toBeNull();
  });

  it('modal Create button carries aria-busy=true while creating', async () => {
    let resolve!: (v: unknown) => void;
    vi.mocked(api.createSystemEnrollmentToken).mockReturnValue(
      new Promise((r) => { resolve = r; })
    );
    render(SystemServicesSettings, props);
    // Open modal
    const createLauncher = screen.getByRole('button', { name: 'Create Token' });
    await fireEvent.click(createLauncher);
    // Fill name
    const nameInput = await screen.findByLabelText(/Name/i);
    await fireEvent.input(nameInput, { target: { value: 'My Token' } });
    const createBtn = screen.getByRole('button', { name: 'Create' });
    await fireEvent.click(createBtn);
    await waitFor(() => expect(createBtn).toHaveAttribute('aria-busy', 'true'));
    resolve({ token: 'tok_abc', id: 't1', name: 'My Token', created_at: new Date().toISOString(),
               revoked_at: null, expires_at: null, max_uses: null, use_count: 0 });
    await waitFor(() => expect(createBtn).not.toHaveAttribute('aria-busy'));
  });
});
```

- [ ] **Step 3.2: Run test — expect FAIL**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/SystemServicesSettings.test.ts 2>&1 | tail -20
```

Expected: failures — raw preset classes found, aria-busy never set.

- [ ] **Step 3.3: Add Button import to `SystemServicesSettings.svelte`**

After line 22 (`} from '$lib/components/ui';`), add:

```svelte
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 3.4: Replace Load Tokens button (lines ~184–186)**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={() => void loadTokens(1)} disabled={loading}>
  {loading ? 'Loading...' : 'Load Tokens'}
</button>
```

New:

```svelte
<Button variant="primary" loading={loading} disabled={loading} onclick={() => void loadTokens(1)}>
  Load Tokens
</Button>
```

Note: `disabled={loading}` is kept in addition to `loading={loading}` to prevent re-clicks when loading state
transitions; Button's `inert` already handles it but being explicit is safe and consistent with
EnrollmentTokenSettings.

- [ ] **Step 3.5: Replace Refresh button (lines ~188–190)**

Old:

```svelte
<button class="btn preset-tonal" onclick={() => void loadTokens(currentPage)} disabled={loading}>
  Refresh
</button>
```

New:

```svelte
<Button variant="secondary" loading={loading} disabled={loading} onclick={() => void loadTokens(currentPage)}>
  Refresh
</Button>
```

- [ ] **Step 3.6: Replace Create Token launcher button (lines ~192–199)**

Old:

```svelte
<button
  class="btn preset-filled-primary-500"
  onclick={() => {
    showCreateDialog = true;
  }}
>
  Create Token
</button>
```

New:

```svelte
<Button
  variant="primary"
  onclick={() => {
    showCreateDialog = true;
  }}
>Create Token</Button>
```

- [ ] **Step 3.7: Replace Copy button (lines ~213–215)**

Old:

```svelte
<button class="btn btn-sm preset-tonal flex-shrink-0" onclick={handleCopy}>
  {copied ? 'Copied!' : 'Copy'}
</button>
```

New:

```svelte
<Button variant="ghost" size="sm" class="flex-shrink-0" onclick={handleCopy}>
  {copied ? 'Copied!' : 'Copy'}
</Button>
```

The copied/Copy text-swap is kept because it is UI feedback, not a loading state. No `loading` prop added here.

- [ ] **Step 3.8: Replace modal Cancel button (lines ~270–278)**

Old:

```svelte
<button
  class="btn preset-tonal"
  onclick={() => {
    showCreateDialog = false;
    resetForm();
  }}
>
  Cancel
</button>
```

New:

```svelte
<Button
  variant="secondary"
  onclick={() => {
    showCreateDialog = false;
    resetForm();
  }}
>Cancel</Button>
```

- [ ] **Step 3.9: Replace modal Create button (lines ~279–281)**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={handleCreate} disabled={creating}>
  {creating ? 'Creating...' : 'Create'}
</button>
```

New:

```svelte
<Button variant="primary" loading={creating} onclick={handleCreate}>Create</Button>
```

- [ ] **Step 3.10: Replace Revoke button (lines ~370–372)**

Old:

```svelte
<button class="btn btn-sm preset-filled-error-500" onclick={() => (confirmRevokeId = token.id)}>
  Revoke
</button>
```

New:

```svelte
<Button variant="danger" size="sm" onclick={() => (confirmRevokeId = token.id)}>Revoke</Button>
```

- [ ] **Step 3.11: Run type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 3.12: Run tests — expect PASS**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/SystemServicesSettings.test.ts 2>&1 | tail -10
```

Expected: 4 passed.

- [ ] **Step 3.13: Run full suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: all tests pass, 0 failures.

- [ ] **Step 3.14: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/SystemServicesSettings.svelte src/routes/settings/SystemServicesSettings.test.ts
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/SystemServicesSettings.svelte frontend/src/routes/settings/SystemServicesSettings.test.ts
git commit -m "feat(ui): migrate SystemServicesSettings to Button primitive (#3d step 3)"
```

---

## Task 4: Migrate `EnrollmentTokenSettings.svelte`

**Files:**

- Modify: `frontend/src/routes/settings/EnrollmentTokenSettings.svelte`
- Create: `frontend/src/routes/settings/EnrollmentTokenSettings.test.ts`

### Context

`import Button from '$lib/components/Button.svelte'` already exists (line 23). Revoke is already migrated to `<Button variant="danger" size="sm" ...>`.

Remaining unmigrated buttons (verified from source):

| Line (approx) | Label | Old class | Target |
| --- | --- | --- | --- |
| 201–203 | Load Tokens | `btn preset-filled-primary-500` + text-swap | `variant="primary" loading={loading} disabled={loading}`, static text `Load Tokens` |
| 205–207 | Refresh | `btn preset-tonal` | `variant="secondary" loading={loading} disabled={loading}` |
| 209–215 | Create Token (launcher) | `btn preset-filled-primary-500` | `variant="primary"` |
| 231–233 | Copy | `btn btn-sm preset-tonal flex-shrink-0` | `variant="ghost" size="sm" class="flex-shrink-0"` |
| 302–309 | Cancel (modal) | `btn preset-tonal` | `variant="secondary"` |
| 311–313 | Create (modal footer) | `btn preset-filled-primary-500` + text-swap | `variant="primary" loading={creating}`, static text `Create` |

- [ ] **Step 4.1: Write failing test**

Create `frontend/src/routes/settings/EnrollmentTokenSettings.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({
  listEnrollmentTokens: vi.fn().mockResolvedValue({ items: [], total: 0, page: 1, per_page: 20, pages: 0 }),
  createEnrollmentToken: vi.fn(),
  revokeEnrollmentToken: vi.fn()
}));

import * as api from '$lib/api';
import EnrollmentTokenSettings from './EnrollmentTokenSettings.svelte';

const props = {
  summary: { total: 0, active: 0, revoked: 0, expired: 0 },
  onSuccess: vi.fn(),
  onError: vi.fn()
};

afterEach(() => vi.clearAllMocks());

describe('EnrollmentTokenSettings button variants', () => {
  it('has no raw preset-filled-primary-500 buttons', () => {
    const { container } = render(EnrollmentTokenSettings, props);
    expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
  });

  it('has no raw preset-tonal buttons (unqualified)', () => {
    const { container } = render(EnrollmentTokenSettings, props);
    expect(container.querySelector('button.preset-tonal')).toBeNull();
  });

  it('modal Create button carries aria-busy=true while creating', async () => {
    let resolve!: (v: unknown) => void;
    vi.mocked(api.createEnrollmentToken).mockReturnValue(
      new Promise((r) => { resolve = r; })
    );
    render(EnrollmentTokenSettings, props);
    const createLauncher = screen.getByRole('button', { name: 'Create Token' });
    await fireEvent.click(createLauncher);
    const nameInput = await screen.findByLabelText(/Name/i);
    await fireEvent.input(nameInput, { target: { value: 'My Token' } });
    const createBtn = screen.getByRole('button', { name: 'Create' });
    await fireEvent.click(createBtn);
    await waitFor(() => expect(createBtn).toHaveAttribute('aria-busy', 'true'));
    resolve({ token: 'tok_xyz', id: 't1', name: 'My Token', created_at: new Date().toISOString(),
               revoked_at: null, expires_at: null, max_uses: null, use_count: 0 });
    await waitFor(() => expect(createBtn).not.toHaveAttribute('aria-busy'));
  });

});
```

- [ ] **Step 4.2: Run test — expect FAIL**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/EnrollmentTokenSettings.test.ts 2>&1 | tail -20
```

Expected: failures — raw preset classes found, aria-busy never set, disabled assertion fails.

- [ ] **Step 4.3: Replace Load Tokens button (lines ~201–203)**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={() => void loadTokens(1)} disabled={loading}>
  {loading ? 'Loading...' : 'Load Tokens'}
</button>
```

New:

```svelte
<Button variant="primary" loading={loading} disabled={loading} onclick={() => void loadTokens(1)}>
  Load Tokens
</Button>
```

- [ ] **Step 4.4: Replace Refresh button (lines ~205–207)**

Old:

```svelte
<button class="btn preset-tonal" onclick={() => void loadTokens(currentPage)} disabled={loading}>
  Refresh
</button>
```

New:

```svelte
<Button variant="secondary" loading={loading} disabled={loading} onclick={() => void loadTokens(currentPage)}>
  Refresh
</Button>
```

- [ ] **Step 4.5: Replace Create Token launcher button (lines ~209–215)**

Old:

```svelte
<button
  class="btn preset-filled-primary-500"
  onclick={() => {
    showCreateModal = true;
  }}
>
  Create Token
</button>
```

New:

```svelte
<Button
  variant="primary"
  onclick={() => {
    showCreateModal = true;
  }}
>Create Token</Button>
```

Note: the exact `onclick` body is in the source; preserve it verbatim.

- [ ] **Step 4.6: Replace Copy button (lines ~231–233)**

Old:

```svelte
<button class="btn btn-sm preset-tonal flex-shrink-0" onclick={handleCopy}>
  {copied ? 'Copied!' : 'Copy'}
</button>
```

New:

```svelte
<Button variant="ghost" size="sm" class="flex-shrink-0" onclick={handleCopy}>
  {copied ? 'Copied!' : 'Copy'}
</Button>
```

- [ ] **Step 4.7: Replace modal Cancel button (lines ~302–309)**

Old:

```svelte
<button
  class="btn preset-tonal"
  onclick={() => {
    showCreateModal = false;
    resetForm();
  }}
>
  Cancel
</button>
```

New:

```svelte
<Button
  variant="secondary"
  onclick={() => {
    showCreateModal = false;
    resetForm();
  }}
>Cancel</Button>
```

Note: the exact `onclick` body and variable names are in the source; preserve them verbatim.

- [ ] **Step 4.8: Replace modal Create footer button (lines ~311–313)**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={handleCreate} disabled={creating}>
  {creating ? 'Creating...' : 'Create'}
</button>
```

New:

```svelte
<Button variant="primary" loading={creating} onclick={handleCreate}>Create</Button>
```

`disabled={creating}` moves to `loading={creating}`. No other `disabled` gate — spec does not mandate one and
adding it would be a behavior change beyond the migration scope.

- [ ] **Step 4.9: Run type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 4.10: Run tests — expect PASS**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/EnrollmentTokenSettings.test.ts 2>&1 | tail -10
```

Expected: 3 passed.

- [ ] **Step 4.11: Run full suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: all tests pass, 0 failures.

- [ ] **Step 4.12: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/EnrollmentTokenSettings.svelte src/routes/settings/EnrollmentTokenSettings.test.ts
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/EnrollmentTokenSettings.svelte frontend/src/routes/settings/EnrollmentTokenSettings.test.ts
git commit -m "feat(ui): migrate EnrollmentTokenSettings to Button primitive (#3d step 4)"
```

---

## Task 5: Migrate `AgentCertificateSettings.svelte` — introduce `saving` + Button

**Files:**

- Modify: `frontend/src/routes/settings/AgentCertificateSettings.svelte`
- Create: `frontend/src/routes/settings/AgentCertificateSettings.test.ts`

### Context

One button: `<button class="btn preset-filled-primary-500" onclick={saveCertificates}> Save </button>` at line 79.

`saveCertificates` is currently a bare async function with no loading guard. No `saving` state exists. Add it,
wrap the async body in try/finally, then migrate the button.

- [ ] **Step 5.1: Write failing test**

Create `frontend/src/routes/settings/AgentCertificateSettings.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({ updateAgentCertificateSettings: vi.fn() }));

import * as api from '$lib/api';
import type { AgentCertificateSettings } from '$lib/types';
import AgentCertificateSettingsComponent from './AgentCertificateSettings.svelte';

const mockSettings: AgentCertificateSettings = {
  lifetime_days: 7,
  renewal_window_hours_override: null,
  effective_renewal_window_hours: 24
};

const props = {
  settings: mockSettings,
  onSuccess: vi.fn(),
  onError: vi.fn()
};

afterEach(() => vi.clearAllMocks());

describe('AgentCertificateSettings Save button', () => {
  it('Save button has no raw preset-filled-primary-500 class', () => {
    const { container } = render(AgentCertificateSettingsComponent, props);
    expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
  });

  it('Save button has primary gradient class', () => {
    render(AgentCertificateSettingsComponent, props);
    const btn = screen.getByRole('button', { name: 'Save' });
    expect(btn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
  });

  it('Save button carries aria-busy=true while saving', async () => {
    let resolve!: (v: AgentCertificateSettings) => void;
    vi.mocked(api.updateAgentCertificateSettings).mockReturnValue(
      new Promise<AgentCertificateSettings>((r) => { resolve = r; })
    );
    render(AgentCertificateSettingsComponent, props);
    const btn = screen.getByRole('button', { name: 'Save' });
    await fireEvent.click(btn);
    await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
    resolve(mockSettings);
    await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy'));
  });

  it('Save button text is static "Save" during loading — no text swap', async () => {
    let resolve!: (v: AgentCertificateSettings) => void;
    vi.mocked(api.updateAgentCertificateSettings).mockReturnValue(
      new Promise<AgentCertificateSettings>((r) => { resolve = r; })
    );
    render(AgentCertificateSettingsComponent, props);
    const btn = screen.getByRole('button', { name: 'Save' });
    await fireEvent.click(btn);
    await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
    expect(btn).toHaveTextContent('Save');
    resolve(mockSettings);
  });
});
```

- [ ] **Step 5.2: Run test — expect FAIL**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/AgentCertificateSettings.test.ts 2>&1 | tail -20
```

Expected: failures — raw class found, no gradient class, aria-busy never set.

- [ ] **Step 5.3: Add `saving` state and Button import to `AgentCertificateSettings.svelte`**

Old script block (lines 1–44):

```svelte
<script lang="ts">
	import { updateAgentCertificateSettings } from '$lib/api';
	import type { AgentCertificateSettings } from '$lib/types';
	import { FormFieldRow, SectionCard } from '$lib/components/ui';

	let {
		settings,
		onSuccess,
		onError
	}: {
		settings: AgentCertificateSettings | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let certLifetimeDays: number = $state(7);
	let useAutoRenewal: boolean = $state(true);
	let certRenewalWindowHours: number = $state(24);

	$effect(() => {
		if (settings) {
			certLifetimeDays = settings.lifetime_days;
			useAutoRenewal = settings.renewal_window_hours_override === null;
			certRenewalWindowHours = settings.renewal_window_hours_override ?? settings.effective_renewal_window_hours;
		}
	});

	async function saveCertificates() {
		try {
			// Send 0 to reset to automatic, or the explicit value for a custom override.
			const renewalHours = useAutoRenewal ? 0 : certRenewalWindowHours;
			const res = await updateAgentCertificateSettings({
				lifetime_days: certLifetimeDays,
				renewal_window_hours: renewalHours
			});
			certLifetimeDays = res.lifetime_days;
			useAutoRenewal = res.renewal_window_hours_override === null;
			certRenewalWindowHours = res.renewal_window_hours_override ?? res.effective_renewal_window_hours;
			onSuccess('Agent certificate settings saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save agent certificate settings');
		}
	}
</script>
```

New script block:

```svelte
<script lang="ts">
	import { updateAgentCertificateSettings } from '$lib/api';
	import type { AgentCertificateSettings } from '$lib/types';
	import { FormFieldRow, SectionCard } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';

	let {
		settings,
		onSuccess,
		onError
	}: {
		settings: AgentCertificateSettings | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let certLifetimeDays: number = $state(7);
	let useAutoRenewal: boolean = $state(true);
	let certRenewalWindowHours: number = $state(24);
	let saving: boolean = $state(false);

	$effect(() => {
		if (settings) {
			certLifetimeDays = settings.lifetime_days;
			useAutoRenewal = settings.renewal_window_hours_override === null;
			certRenewalWindowHours = settings.renewal_window_hours_override ?? settings.effective_renewal_window_hours;
		}
	});

	async function saveCertificates() {
		saving = true;
		try {
			// Send 0 to reset to automatic, or the explicit value for a custom override.
			const renewalHours = useAutoRenewal ? 0 : certRenewalWindowHours;
			const res = await updateAgentCertificateSettings({
				lifetime_days: certLifetimeDays,
				renewal_window_hours: renewalHours
			});
			certLifetimeDays = res.lifetime_days;
			useAutoRenewal = res.renewal_window_hours_override === null;
			certRenewalWindowHours = res.renewal_window_hours_override ?? res.effective_renewal_window_hours;
			onSuccess('Agent certificate settings saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save agent certificate settings');
		} finally {
			saving = false;
		}
	}
</script>
```

- [ ] **Step 5.4: Replace Save button in the template (line ~79)**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={saveCertificates}> Save </button>
```

New:

```svelte
<Button variant="primary" loading={saving} onclick={saveCertificates}>Save</Button>
```

- [ ] **Step 5.5: Run type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 5.6: Run tests — expect PASS**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/AgentCertificateSettings.test.ts 2>&1 | tail -10
```

Expected: 4 passed.

- [ ] **Step 5.7: Run full suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: all tests pass, 0 failures.

- [ ] **Step 5.8: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/AgentCertificateSettings.svelte src/routes/settings/AgentCertificateSettings.test.ts
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/AgentCertificateSettings.svelte frontend/src/routes/settings/AgentCertificateSettings.test.ts
git commit -m "feat(ui): migrate AgentCertificateSettings to Button primitive with saving state (#3d step 5)"
```

---

## Task 6: Extend unit tests

**Files:**

- Review: all five test files created in tasks 1–5

- [ ] **Step 6.1: Run the full settings test suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/ 2>&1 | tail -30
```

Expected: all tests in all settings test files pass. If any failures, fix them before proceeding.

- [ ] **Step 6.2: Verify no raw `<button class="btn` remain in the five files**

```bash
grep -n '<button class="btn' \
  /Users/andreyyantsen/Development/uptrakit/frontend/src/routes/settings/PluginConfigsTab.svelte \
  /Users/andreyyantsen/Development/uptrakit/frontend/src/routes/settings/SchedulerTab.svelte \
  /Users/andreyyantsen/Development/uptrakit/frontend/src/routes/settings/SystemServicesSettings.svelte \
  /Users/andreyyantsen/Development/uptrakit/frontend/src/routes/settings/EnrollmentTokenSettings.svelte \
  /Users/andreyyantsen/Development/uptrakit/frontend/src/routes/settings/AgentCertificateSettings.svelte
```

Expected: no output. If any lines appear, fix the missed migration before committing.

- [ ] **Step 6.3: Commit any additional test fixes**

If step 6.1 required fixes:

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/
git commit -m "test(ui): extend Button migration unit tests for settings #3d"
```

If no fixes were needed, skip this commit.

---

## Task 7: Frontend gate

**Files:** none unless lint/type fixes are required

- [ ] **Step 7.1: Run lint**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run lint 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 7.2: Run format check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run format:check 2>&1 | tail -5
```

Expected: no unformatted files.

- [ ] **Step 7.3: Run svelte-check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run check 2>&1 | grep -E "error|Error" | head -20
```

Expected: no errors.

- [ ] **Step 7.4: Run full Vitest suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run test 2>&1 | tail -10
```

Expected: all tests pass, 0 failures.

- [ ] **Step 7.5: Run build**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run build 2>&1 | tail -10
```

Expected: build succeeds with no errors.

- [ ] **Step 7.6: Commit fixes if needed**

If any of steps 7.1–7.5 required changes:

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/
git commit -m "chore(ui): lint+type fixes for #3d Button migration"
```

If no changes needed, skip this commit.

---

## Self-Review Checklist

| Spec requirement | Task / Step |
| --- | --- |
| PluginConfigsTab: Add Config → primary | Task 1, Step 1.4 |
| PluginConfigsTab: Edit (config row) → secondary sm | Task 1, Step 1.5 |
| PluginConfigsTab: Discover → secondary sm + loading | Task 1, Step 1.5 |
| PluginConfigsTab: Delete → danger sm | Task 1, Step 1.5 |
| PluginConfigsTab: Add Plugin Type → primary | Task 1, Step 1.6 |
| PluginConfigsTab: Remove (allowlist row) → danger sm | Task 1, Step 1.7 |
| PluginConfigsTab: Edit (type settings row) → secondary sm | Task 1, Step 1.8 |
| PluginConfigsTab: Reset (type settings row) → danger sm | Task 1, Step 1.8 |
| PluginConfigsTab: Advanced: Edit as JSON → secondary sm | Task 1, Step 1.9 |
| PluginConfigsTab: Return to form editor → secondary sm | Task 1, Step 1.9 |
| PluginConfigsTab: Config modal Cancel → secondary | Task 1, Step 1.10 |
| PluginConfigsTab: Config modal Test → secondary + loading | Task 1, Step 1.10 |
| PluginConfigsTab: Config modal Create/Update → primary | Task 1, Step 1.10 |
| PluginConfigsTab: Allowlist modal Cancel → secondary | Task 1, Step 1.11 |
| PluginConfigsTab: Allowlist modal Add → primary | Task 1, Step 1.11 |
| PluginConfigsTab: Type settings modal Cancel → secondary | Task 1, Step 1.12 |
| PluginConfigsTab: Type settings modal Save → primary | Task 1, Step 1.12 |
| SchedulerTab: Retry → primary | Task 2, Step 2.4 |
| SchedulerTab: Edit (row) → secondary sm | Task 2, Step 2.5 |
| SchedulerTab: Run (row) → ghost sm + loading | Task 2, Step 2.6 |
| SchedulerTab: Cancel (modal) → secondary | Task 2, Step 2.7 |
| SchedulerTab: Save (modal) → primary + loading + no text-swap | Task 2, Step 2.8 |
| SystemServicesSettings: Load Tokens → primary + loading | Task 3, Step 3.4 |
| SystemServicesSettings: Refresh → secondary + loading | Task 3, Step 3.5 |
| SystemServicesSettings: Create Token launcher → primary | Task 3, Step 3.6 |
| SystemServicesSettings: Copy → ghost sm | Task 3, Step 3.7 |
| SystemServicesSettings: Modal Cancel → secondary | Task 3, Step 3.8 |
| SystemServicesSettings: Modal Create → primary + loading + no text-swap | Task 3, Step 3.9 |
| SystemServicesSettings: Revoke → danger sm | Task 3, Step 3.10 |
| EnrollmentTokenSettings: Load Tokens → primary + loading | Task 4, Step 4.3 |
| EnrollmentTokenSettings: Refresh → secondary + loading | Task 4, Step 4.4 |
| EnrollmentTokenSettings: Create Token launcher → primary | Task 4, Step 4.5 |
| EnrollmentTokenSettings: Copy → ghost sm | Task 4, Step 4.6 |
| EnrollmentTokenSettings: Modal Cancel → secondary | Task 4, Step 4.7 |
| EnrollmentTokenSettings: Modal Create → primary + loading (no extra disabled gate) | Task 4, Step 4.8 |
| AgentCertificateSettings: introduce saving state | Task 5, Step 5.3 |
| AgentCertificateSettings: Save → primary + loading + no text-swap | Task 5, Step 5.4 |
| `<input>`, `<select>`, `<textarea>`, toggles — OUT OF SCOPE | Not touched |
| Scheduler enable/disable toggle — OUT OF SCOPE | Not touched |
| Full frontend gate | Task 7 |
