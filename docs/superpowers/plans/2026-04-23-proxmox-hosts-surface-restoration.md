# Proxmox Hosts Surface Restoration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans
> to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore `proxmox.hosts` surface to a fully functional host-mappings table with a generic `context_selector` framework
extension that drives `baseParams` for the table and gated action buttons.

**Architecture:** A new `SurfaceContextSelectorDescriptor` struct on `SurfaceDescriptor` (Rust + TS contract) signals that
`SurfaceReadPanel` should render a `ProviderSelector` dropdown above the surface content. The selection merges a param key
into `effectiveBaseParams`, which gates action buttons via `requiredContextParam`/`requiredForInteractionIds` props threaded
through `SurfaceRenderer` → `SurfaceActionBar` → `SurfaceInteractionButton`. `handle_list` is made optional on
`plugin_config_id` and adds a secondary batch lookup for `config_name`.

**Tech Stack:** Rust / SeaORM 2.x, Svelte 5 (runes), TypeScript, Vitest + @testing-library/svelte

---

## Spec reference

`docs/superpowers/specs/2026-04-22-proxmox-hosts-surface-restoration-design.md`

## File Map

| File | Action | Purpose |
| --- | --- | --- |
| `crates/shared/surfaces/src/surface.rs` | Modify | Add `#[non_exhaustive]` to `Capability`+`SurfaceDescriptor`, add `Capability::ContextSelector`, add `SurfaceContextSelectorDescriptor` struct, add `context_selector` field to `SurfaceDescriptor` |
| `frontend/src/lib/surfaces/contract.ts` | Modify | Add `SurfaceContextSelector` interface, add `'context_selector'` to `SurfaceCapability`, add `context_selector?` to `SurfaceDescriptor` |
| `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte` | Modify | Add `requiredContextParam?: string` prop; disabled `<span>` wrapper guard |
| `frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts` | Modify | New tests for disabled guard |
| `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte` | Modify | Add `requiredContextParam?: string` and `requiredForInteractionIds?: string[]` props; per-button dispatch |
| `frontend/src/lib/components/surfaces/SurfaceActionBar.test.ts` | Modify | New test for prop threading through to disabled button |
| `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte` | Modify | Add same two props; forward to `SurfaceActionBar` on `action_bar` branch only |
| `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte` | Modify | Context selector state + fetch + `effectiveBaseParams` + `baseParamsFingerprint` fix + prop pass-through |
| `frontend/src/lib/components/surfaces/SurfaceReadPanel.test.ts` | Modify | New tests: selector render, option fetch, selection updates params |
| `crates/plugins/infrastructure/proxmox/src/surfaces.rs` | Modify | `handle_list`: optional `plugin_config_id`, updated serialization keys, secondary config-name batch lookup |
| `crates/plugins/infrastructure/proxmox/src/plugin.rs` | Modify | Replace `proxmox_hosts_selector_boundary_surface()` with `proxmox_hosts_surface()` |

## Spec alignment notes

**Column key / serialization mismatch (spec fix):** The spec's `SurfaceTableColumn` keys (`proxmox_name`, `proxmox_node`,
`proxmox_vmid`, `proxmox_type`, `proxmox_status`) do not match `handle_list`'s current JSON serialization (`name`, `node`,
`vmid`, `type`, `status`). This plan updates the serialization to use the `proxmox_` prefixed names so the column keys
render data. This is safe because the surface was degraded (no table was consuming this data).

**Config name lookup strategy (spec deviation):** The spec suggests `inner_join` + `FromQueryResult` DTO. This plan uses a
simpler secondary batch query for config names (collect unique `plugin_config_id`s from the page, query
`plugin_config::Entity::find().filter(id.is_in(...))`, build a `HashMap<Uuid, String>`). Result identical; simpler to implement.

---

## Task 1: Rust surface types — `ContextSelector` capability + descriptor struct

**Files:**

- Modify: `crates/shared/surfaces/src/surface.rs`

### Step-by-step

- [ ] **Step 1: Add `#[non_exhaustive]` to `Capability` and `SurfaceDescriptor`**

In `surface.rs`, change:

```rust
// Before Capability (line ~161):
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
```

to:

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
```

> **Note:** Adding `#[non_exhaustive]` to `Capability` does not remove `Copy` (it's still `Copy`). However,
> `#[non_exhaustive]` on an enum prevents exhaustive match in external crates. All external call sites use iterator-based
> capability checks (`.contains()`, `.contains_all()`), not match arms — so no external changes are needed.

Change:

```rust
// Before SurfaceDescriptor (line ~122):
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceDescriptor {
```

to:

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceDescriptor {
```

> **Note:** `SurfaceDescriptor` is constructed with struct literal syntax in `plugin.rs` within the same crate.
> `#[non_exhaustive]` only restricts external crates — no changes needed in the proxmox plugin.

- [ ] **Step 2: Add `Capability::ContextSelector` variant**

In `surface.rs`, add to the `Capability` enum (after `ProviderInitiatedActions`):

```rust
    ProviderInitiatedActions,
    ContextSelector,
```

- [ ] **Step 3: Add `SurfaceContextSelectorDescriptor` struct**

Add after the `CapabilitySet` impl block in `surface.rs`:

```rust
/// Describes a context-selector dropdown rendered above a surface's content.
///
/// When present on a `SurfaceDescriptor`, `SurfaceReadPanel` fetches the
/// options from `rest_api_path` and renders a `ProviderSelector` above the
/// surface content. The selected value is merged into `baseParams` under
/// `param_key`, driving both the table data load and optional interaction gates.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceContextSelectorDescriptor {
    /// Param key injected into `baseParams` when a specific option is selected.
    pub param_key: String,
    /// Label shown above the selector dropdown.
    pub label: String,
    /// Label for the "show all" option (no param injected).
    pub all_option_label: String,
    /// REST API path returning a JSON array or paginated `items` list.
    pub rest_api_path: String,
    /// Field in each item used as the option value.
    pub value_field: String,
    /// Field in each item used as the option label.
    pub label_field: String,
    /// Interaction IDs disabled (with tooltip) when no specific option is selected.
    #[serde(default)]
    pub required_for_interactions: Vec<String>,
}
```

- [ ] **Step 4: Add `context_selector` field to `SurfaceDescriptor`**

In `surface.rs`, add to `SurfaceDescriptor` after `root_node`:

```rust
    pub root_node: SurfaceNode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_selector: Option<SurfaceContextSelectorDescriptor>,
```

- [ ] **Step 5: Write unit test for capability serialization**

Add inside `#[cfg(test)]` at the bottom of `surface.rs` (or add a `mod tests` block if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_selector_capability_serializes_to_snake_case() {
        let cap = Capability::ContextSelector;
        let serialized = serde_json::to_string(&cap).expect("serialize");
        assert_eq!(serialized, r#""context_selector""#);
    }

    #[test]
    fn surface_descriptor_context_selector_round_trips() {
        let descriptor = SurfaceDescriptor {
            surface_id: SurfaceId::new("test.surface").unwrap(),
            label: "Test".to_string(),
            priority: 100,
            slot: "surface.page".to_string(),
            scope: Scope::Global,
            targeting: Targeting::Universal,
            required_permission: None,
            provider_kind: ProviderKind::Plugin,
            required_capabilities: CapabilitySet::from_capabilities([
                Capability::ContextSelector,
            ]),
            root_node: SurfaceNode::Section {
                title: None,
                children: vec![],
            },
            context_selector: Some(SurfaceContextSelectorDescriptor {
                param_key: "plugin_config_id".to_string(),
                label: "Configuration".to_string(),
                all_option_label: "All Configurations".to_string(),
                rest_api_path: "/api/v1/plugin-configs".to_string(),
                value_field: "id".to_string(),
                label_field: "name".to_string(),
                required_for_interactions: vec!["discover".to_string()],
            }),
        };

        let json = serde_json::to_string(&descriptor).expect("serialize");
        let deserialized: SurfaceDescriptor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(descriptor, deserialized);

        let context_selector = deserialized.context_selector.unwrap();
        assert_eq!(context_selector.param_key, "plugin_config_id");
        assert_eq!(context_selector.required_for_interactions, vec!["discover"]);
    }

    #[test]
    fn surface_descriptor_without_context_selector_omits_field_in_json() {
        let descriptor = SurfaceDescriptor {
            surface_id: SurfaceId::new("test.surface").unwrap(),
            label: "Test".to_string(),
            priority: 100,
            slot: "surface.page".to_string(),
            scope: Scope::Global,
            targeting: Targeting::Universal,
            required_permission: None,
            provider_kind: ProviderKind::Plugin,
            required_capabilities: CapabilitySet::default(),
            root_node: SurfaceNode::Section { title: None, children: vec![] },
            context_selector: None,
        };

        let json = serde_json::to_string(&descriptor).expect("serialize");
        assert!(!json.contains("context_selector"), "absent context_selector must be omitted from JSON");
    }
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p uptrakit-shared-surfaces --all-features
```

Expected: all tests pass, including the three new ones.

- [ ] **Step 7: Commit**

```bash
git add crates/shared/surfaces/src/surface.rs
git commit -m "feat(surfaces): add ContextSelector capability and SurfaceContextSelectorDescriptor"
```

---

## Task 2: Frontend contract types

**Files:**

- Modify: `frontend/src/lib/surfaces/contract.ts`

- [ ] **Step 1: Add `SurfaceContextSelector` interface**

In `contract.ts`, add before `SurfaceDescriptor`:

```typescript
export interface SurfaceContextSelector {
    param_key: string;
    label: string;
    all_option_label: string;
    rest_api_path: string;
    value_field: string;
    label_field: string;
    required_for_interactions: string[];
}
```

- [ ] **Step 2: Add `'context_selector'` to `SurfaceCapability`**

In `contract.ts`, add to the `SurfaceCapability` union (after `'provider_initiated_actions'`):

```typescript
    | 'provider_initiated_actions'
    | 'context_selector';
```

- [ ] **Step 3: Add `context_selector` field to `SurfaceDescriptor`**

In `contract.ts`, add to `SurfaceDescriptor` after `root_node`:

```typescript
export interface SurfaceDescriptor {
    // ... existing fields ...
    root_node: SurfaceNode;
    context_selector?: SurfaceContextSelector;
}
```

- [ ] **Step 4: Type-check**

```bash
cd frontend && npm run check
```

Expected: no type errors.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/surfaces/contract.ts
git commit -m "feat(surfaces): add SurfaceContextSelector contract type"
```

---

## Task 3: `SurfaceInteractionButton` — required context param guard

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts`

### Background

`Button` does not accept a `title` prop and applies `pointer-events-none` on disabled state — so native `title` tooltips are
invisible. Wrap the disabled button in `<span title="...">` instead (the span retains pointer events).

- [ ] **Step 1: Write failing tests**

Add to `SurfaceInteractionButton.test.ts` inside the `describe` block:

```typescript
describe('requiredContextParam guard', () => {
    const interaction: InteractionDescriptor = {
        interaction_id: 'discover',
        kind: 'mutation_action',
        label: 'Discover',
        transport: { mode: 'controller_local' }
    };

    it('renders button disabled with tooltip wrapper when requiredContextParam absent from baseParams', () => {
        render(SurfaceInteractionButton, {
            surfaceId: 'proxmox.hosts',
            interaction,
            interactions: [interaction],
            baseParams: {},
            requiredContextParam: 'plugin_config_id'
        });

        const button = screen.getByRole('button', { name: 'Discover' });
        expect(button).toBeDisabled();
        const wrapper = button.closest('span[title]');
        expect(wrapper).not.toBeNull();
        expect(wrapper?.getAttribute('title')).toBe('Select a configuration first');
    });

    it('renders button enabled when requiredContextParam present in baseParams', () => {
        render(SurfaceInteractionButton, {
            surfaceId: 'proxmox.hosts',
            interaction,
            interactions: [interaction],
            baseParams: { plugin_config_id: '01944c3c-6a3a-7000-8000-000000000001' },
            requiredContextParam: 'plugin_config_id'
        });

        const button = screen.getByRole('button', { name: 'Discover' });
        expect(button).not.toBeDisabled();
        expect(button.closest('span[title]')).toBeNull();
    });

    it('renders button normally when requiredContextParam is undefined', () => {
        render(SurfaceInteractionButton, {
            surfaceId: 'proxmox.hosts',
            interaction,
            interactions: [interaction],
            baseParams: {}
            // requiredContextParam omitted
        });

        const button = screen.getByRole('button', { name: 'Discover' });
        expect(button).not.toBeDisabled();
        expect(button.closest('span[title]')).toBeNull();
    });
});
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd frontend && npm run test -- SurfaceInteractionButton
```

Expected: the three new tests fail (`requiredContextParam` is not a known prop).

- [ ] **Step 3: Add `requiredContextParam` prop and implement guard**

In `SurfaceInteractionButton.svelte`, update the props block and add the guard:

```svelte
<script lang="ts">
    // ... existing imports ...

    let {
        surfaceId,
        interaction,
        interactions = [],
        targetProviderId,
        encryptionContext,
        baseParams = {},
        rowSeed,
        size = 'md',
        oncomplete,
        requiredContextParam  // NEW
    }: {
        surfaceId: string;
        interaction: InteractionDescriptor;
        interactions?: InteractionDescriptor[];
        targetProviderId?: string;
        encryptionContext?: SurfaceEncryptionContext;
        baseParams?: Record<string, unknown>;
        rowSeed?: Record<string, unknown>;
        size?: 'sm' | 'md';
        oncomplete?: (result: unknown) => void | Promise<void>;
        requiredContextParam?: string;  // NEW
    } = $props();

    // NEW: button is gated when param key is set but value is absent/empty
    const isContextGated = $derived(
        !!requiredContextParam &&
        (!baseParams[requiredContextParam] || baseParams[requiredContextParam] === '')
    );

    // ... rest of existing script unchanged ...
</script>
```

In the template section, locate the final `{:else}` branch (after `{:else if isWorkflow}`) that renders `<Button>` directly.
Replace that `<Button>` with a nested conditional:

```svelte
<!-- Inside the final {:else} branch (after {:else if isWorkflow}): replace the existing <Button> with: -->
{:else}
    {#if isContextGated}
        <span title="Select a configuration first">
            <Button
                variant={interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'}
                {size}
                disabled
            >
                {actionLabel}
            </Button>
        </span>
    {:else}
        <Button
            variant={interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'}
            {size}
            {loading}
            onclick={requestAction}
        >
            {actionLabel}
        </Button>
    {/if}

    <!-- ... existing modal and confirm dialog blocks unchanged ... -->
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd frontend && npm run test -- SurfaceInteractionButton
```

Expected: all tests pass including the three new ones.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte \
        frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts
git commit -m "feat(surfaces): add requiredContextParam disabled guard to SurfaceInteractionButton"
```

---

## Task 4: `SurfaceActionBar` — required context param props

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceActionBar.test.ts`

- [ ] **Step 1: Write failing test**

Add to `SurfaceActionBar.test.ts` inside the `describe` block:

```typescript
it('passes requiredContextParam to button whose id is in requiredForInteractionIds', async () => {
    const discoverInteraction: InteractionDescriptor = {
        interaction_id: 'discover',
        kind: 'mutation_action',
        label: 'Discover',
        transport: { mode: 'controller_local' }
    };
    const testInteraction: InteractionDescriptor = {
        interaction_id: 'test-connection',
        kind: 'mutation_action',
        label: 'Test Connection',
        transport: { mode: 'controller_local' }
    };

    render(SurfaceActionBar, {
        surfaceId: 'proxmox.hosts',
        actionIds: ['discover', 'test-connection'],
        interactions: [discoverInteraction, testInteraction],
        baseParams: {},                           // no plugin_config_id
        requiredContextParam: 'plugin_config_id',
        requiredForInteractionIds: ['discover', 'test-connection']
    });

    const discoverBtn = screen.getByRole('button', { name: 'Discover' });
    const testBtn = screen.getByRole('button', { name: 'Test Connection' });
    expect(discoverBtn).toBeDisabled();
    expect(testBtn).toBeDisabled();
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd frontend && npm run test -- SurfaceActionBar
```

Expected: new test fails (`requiredContextParam` / `requiredForInteractionIds` not recognized).

- [ ] **Step 3: Add props and per-button dispatch**

In `SurfaceActionBar.svelte`, update the props block:

```svelte
<script lang="ts">
    // ... existing imports ...

    let {
        surfaceId,
        actionIds = [],
        interactions = [],
        targetProviderId,
        encryptionContext,
        baseParams = {},
        requiredContextParam,       // NEW
        requiredForInteractionIds = [] // NEW
    }: {
        surfaceId: string;
        actionIds?: InteractionId[];
        interactions?: InteractionDescriptor[];
        targetProviderId?: string;
        encryptionContext?: SurfaceEncryptionContext;
        baseParams?: Record<string, unknown>;
        requiredContextParam?: string;        // NEW
        requiredForInteractionIds?: string[]; // NEW
    } = $props();

    // ... existing derived state unchanged ...
</script>
```

In the template, update `SurfaceInteractionButton` to pass `requiredContextParam` conditionally:

```svelte
{#each resolvedActions as interaction (interaction.interaction_id)}
    <SurfaceInteractionButton
        {surfaceId}
        {interaction}
        {interactions}
        {targetProviderId}
        {encryptionContext}
        {baseParams}
        requiredContextParam={requiredForInteractionIds.includes(interaction.interaction_id)
            ? requiredContextParam
            : undefined}
        oncomplete={async () => {
            notifySurfaceReload();
        }}
    />
{/each}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd frontend && npm run test -- SurfaceActionBar
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/components/surfaces/SurfaceActionBar.svelte \
        frontend/src/lib/components/surfaces/SurfaceActionBar.test.ts
git commit -m "feat(surfaces): thread requiredContextParam/requiredForInteractionIds through SurfaceActionBar"
```

---

## Task 5: `SurfaceRenderer` — forward new props to `SurfaceActionBar`

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`

No new tests needed — `SurfaceRenderer` is a pass-through for these props. Coverage comes from the `SurfaceReadPanel` integration tests in Task 6.

- [ ] **Step 1: Add props to `SurfaceRenderer`**

In `SurfaceRenderer.svelte`, update the props block:

```svelte
let {
    surfaceId,
    node,
    interactions = [],
    dataSources = [],
    targetProviderId,
    encryptionContext,
    dataBySource = {},
    baseParams = {},
    requiredContextParam,           // NEW
    requiredForInteractionIds = []  // NEW
}: {
    surfaceId: string;
    node: SurfaceNode;
    interactions?: InteractionDescriptor[];
    dataSources?: DataSourceDescriptor[];
    targetProviderId?: string;
    encryptionContext?: SurfaceEncryptionContext;
    dataBySource?: Record<string, unknown>;
    baseParams?: Record<string, unknown>;
    requiredContextParam?: string;        // NEW
    requiredForInteractionIds?: string[]; // NEW
} = $props();
```

- [ ] **Step 2: Forward props to `SurfaceActionBar` in the `action_bar` branch**

In `SurfaceRenderer.svelte`, find the `{:else if node.kind === 'action_bar'}` (or equivalent) branch and add the two props:

```svelte
{:else if node.kind === 'action_bar'}
    <SurfaceActionBar
        {surfaceId}
        actionIds={node.action_ids}
        {interactions}
        {targetProviderId}
        {encryptionContext}
        {baseParams}
        {requiredContextParam}
        {requiredForInteractionIds}
    />
```

All other node branches (`section`, `table`, `key_value`, `form`, `tabs`, `callout`, `modal_trigger`, `workflow_trigger`,
`empty_state`) do **not** receive these props.

- [ ] **Step 3: Type-check**

```bash
cd frontend && npm run check
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/components/surfaces/SurfaceRenderer.svelte
git commit -m "feat(surfaces): forward requiredContextParam props through SurfaceRenderer to action bars"
```

---

## Task 6: `SurfaceReadPanel` — context selector

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceReadPanel.test.ts`

### What changes

1. New reactive state: `selectedContextValue`, `selectorOptions`
2. New `$effect` to fetch selector options from `descriptor.context_selector.rest_api_path`
3. New derived: `effectiveBaseParams` — merges selected value into `baseParams`
4. **Critical:** `baseParamsFingerprint` computed from `effectiveBaseParams` (not `baseParams`)
5. Template: render `ProviderSelector` when `descriptor.context_selector` present (non-targeted branch)
6. Pass `effectiveBaseParams` to `SurfaceRenderer` in place of `baseParams`
7. Pass `requiredContextParam` and `requiredForInteractionIds` to `SurfaceRenderer`

- [ ] **Step 1: Write failing tests**

Add to `SurfaceReadPanel.test.ts`:

```typescript
import { waitFor } from '@testing-library/svelte';

describe('context selector', () => {
    function makeReadWithContextSelector(): SurfaceReadResponse {
        const base = makeRead();
        return {
            ...base,
            descriptor: {
                ...base.descriptor,
                context_selector: {
                    param_key: 'plugin_config_id',
                    label: 'Configuration',
                    all_option_label: 'All Configurations',
                    rest_api_path: '/api/v1/plugin-configs',
                    value_field: 'id',
                    label_field: 'name',
                    required_for_interactions: []
                }
            }
        };
    }

    beforeEach(() => {
        vi.stubGlobal('fetch', vi.fn());
    });

    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it('renders ProviderSelector when context_selector is present', async () => {
        vi.mocked(fetch).mockResolvedValue({
            ok: true,
            json: async () => [
                { id: 'cfg-1', name: 'Cluster 1' },
                { id: 'cfg-2', name: 'Cluster 2' }
            ]
        } as Response);

        render(SurfaceReadPanel, {
            surface: makeSurface(),
            read: makeReadWithContextSelector()
        });

        await waitFor(() => {
            expect(screen.getByLabelText('Configuration')).toBeInTheDocument();
        });

        const select = screen.getByLabelText('Configuration') as HTMLSelectElement;
        const options = Array.from(select.options).map((o) => o.text);
        expect(options[0]).toBe('All Configurations');
        expect(options).toContain('Cluster 1');
        expect(options).toContain('Cluster 2');
    });

    it('selecting an option merges param_key into baseParams passed to SurfaceRenderer', async () => {
        vi.mocked(fetch).mockResolvedValue({
            ok: true,
            json: async () => [{ id: 'cfg-1', name: 'Cluster 1' }]
        } as Response);

        // Spy on invokeSurfaceInteraction to capture baseParams used in hydration
        vi.mocked(invokeSurfaceInteraction).mockResolvedValue({});

        const read = makeReadWithContextSelector();
        // Add a hydration-triggering data source so effectiveBaseParams flows into requests
        read.data_sources = [{
            data_source_id: 'ds1',
            kind: { kind: 'provider_query', operation_id: 'list' },
            result_schema: 'any',
            refresh_policy: { type: 'manual' }
        }];
        read.interactions = [{
            interaction_id: 'list',
            kind: 'data_load',
            label: 'List',
            transport: { mode: 'controller_local' }
        }];
        read.descriptor.root_node = { kind: 'key_value', data_source_id: 'ds1' };

        render(SurfaceReadPanel, { surface: makeSurface(), read });

        await waitFor(() => {
            expect(screen.getByLabelText('Configuration')).toBeInTheDocument();
        });

        const select = screen.getByLabelText('Configuration') as HTMLSelectElement;
        await fireEvent.change(select, { target: { value: 'cfg-1' } });

        await waitFor(() => {
            const calls = vi.mocked(invokeSurfaceInteraction).mock.calls;
            const lastCall = calls[calls.length - 1];
            expect(lastCall[2].params).toMatchObject({ plugin_config_id: 'cfg-1' });
        });
    });

    it('does not render a selector when context_selector is absent', () => {
        render(SurfaceReadPanel, {
            surface: makeSurface(),
            read: makeRead()
        });

        expect(screen.queryByLabelText('Configuration')).toBeNull();
    });

    it('falls back to All Configurations on fetch failure', async () => {
        vi.mocked(fetch).mockRejectedValue(new Error('network error'));

        render(SurfaceReadPanel, {
            surface: makeSurface(),
            read: makeReadWithContextSelector()
        });

        await waitFor(() => {
            const select = screen.queryByLabelText('Configuration');
            if (select) {
                const options = Array.from((select as HTMLSelectElement).options);
                expect(options.length).toBe(1);
                expect(options[0].text).toBe('All Configurations');
            }
        });
    });
});
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd frontend && npm run test -- SurfaceReadPanel
```

Expected: new `context selector` tests fail.

- [ ] **Step 3: Add state, effect, and derived values**

In `SurfaceReadPanel.svelte`, add after the existing `let hydrationRetryNonce = $state(0);` line:

```svelte
let selectedContextValue = $state('');
let selectorOptions = $state<{ id: string; label: string }[]>([]);
```

Add after the existing `const encryptionContext = $derived.by(...)` block:

```svelte
const contextSelector = $derived(descriptor.context_selector);

const effectiveBaseParams = $derived(
    contextSelector && selectedContextValue
        ? { ...baseParams, [contextSelector.param_key]: selectedContextValue }
        : { ...baseParams }
);

const requiredContextParam = $derived(contextSelector?.param_key);
const requiredForInteractionIds = $derived(
    contextSelector?.required_for_interactions ?? []
);
```

- [ ] **Step 4: Fix `baseParamsFingerprint` to use `effectiveBaseParams`**

Find this line (currently around line 61):

```svelte
const baseParamsFingerprint = $derived(stableStringify(baseParams));
```

Replace with:

```svelte
const baseParamsFingerprint = $derived(stableStringify(effectiveBaseParams));
```

> **Why:** The hydration `$effect` watches `hydrationFingerprint`, which includes `base_params: baseParamsFingerprint`.
> Without this change, the table never re-fetches when the selector value changes.

- [ ] **Step 5: Add context selector fetch effect**

Add after the existing hydration `$effect` block:

```svelte
$effect(() => {
    const cs = contextSelector;
    if (!cs) {
        selectorOptions = [];
        selectedContextValue = '';
        return;
    }
    let cancelled = false;
    void (async () => {
        try {
            const response = await fetch(cs.rest_api_path);
            if (cancelled) return;
            if (!response.ok) {
                selectorOptions = [];
                return;
            }
            const data: unknown = await response.json();
            if (cancelled) return;
            let rawItems: unknown[] = [];
            if (Array.isArray(data)) {
                rawItems = data;
            } else if (
                data &&
                typeof data === 'object' &&
                'items' in data &&
                Array.isArray((data as { items: unknown[] }).items)
            ) {
                rawItems = (data as { items: unknown[] }).items;
            }
            selectorOptions = rawItems
                .filter(
                    (item): item is Record<string, unknown> =>
                        !!item && typeof item === 'object' && !Array.isArray(item)
                )
                .map((item) => ({
                    id: String(item[cs.value_field] ?? ''),
                    label: String(item[cs.label_field] ?? '')
                }))
                .filter((opt) => opt.id);
        } catch {
            selectorOptions = [];
        }
    })();
    return () => {
        cancelled = true;
    };
});
```

- [ ] **Step 6: Update template — render selector and pass props to `SurfaceRenderer`**

Find the non-targeted `else` branch at the bottom of the template. It currently looks like:

```svelte
{:else}
    <SurfaceRenderer
        surfaceId={descriptor.surface_id}
        node={descriptor.root_node}
        interactions={read.interactions}
        dataSources={read.data_sources}
        {dataBySource}
        {baseParams}
    />
```

Replace with:

```svelte
{:else}
    {#if contextSelector}
        <div class="mb-4 max-w-[280px]">
            <ProviderSelector
                label={contextSelector.label}
                providers={[
                    { id: '', label: contextSelector.all_option_label },
                    ...selectorOptions
                ]}
                selectedId={selectedContextValue}
                onSelect={(id) => {
                    selectedContextValue = id;
                }}
            />
        </div>
    {/if}
    {#if hydrationLoading}
        <p class="py-8 text-center text-[var(--text-muted)]">Loading...</p>
    {:else if hydrationError}
        <Callout tone="danger" title="Unable to load surface data" message={hydrationError}>
            <Button variant="danger" size="sm" type="button" onclick={retryHydration}>Try again</Button>
        </Callout>
    {:else}
        <SurfaceRenderer
            surfaceId={descriptor.surface_id}
            node={descriptor.root_node}
            interactions={read.interactions}
            dataSources={read.data_sources}
            {dataBySource}
            baseParams={effectiveBaseParams}
            {requiredContextParam}
            {requiredForInteractionIds}
        />
    {/if}
```

> **Note:** The `hydrationLoading` / `hydrationError` blocks were previously at the top level alongside `SurfaceRenderer`.
> Move them inside the `else` branch so they appear after the selector. Verify the full template structure is correct after
> the edit.

- [ ] **Step 7: Run tests to verify they pass**

```bash
cd frontend && npm run test -- SurfaceReadPanel
```

Expected: all tests pass including the four new context selector tests.

- [ ] **Step 8: Type-check**

```bash
cd frontend && npm run check
```

Expected: no errors.

- [ ] **Step 9: Commit**

```bash
git add frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte \
        frontend/src/lib/components/surfaces/SurfaceReadPanel.test.ts
git commit -m "feat(surfaces): add context selector to SurfaceReadPanel with effectiveBaseParams"
```

---

## Task 7: `handle_list` — optional `plugin_config_id` + serialization + config names

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/surfaces.rs`

### What changes

1. `parse_uuid_param` → `parse_optional_uuid_param` for `plugin_config_id`
2. Filter becomes conditional on whether `plugin_config_id` is `Some`
3. Inner-join `plugin_config` so orphaned rows (no config) are excluded
4. Serialization field names: `name` → `proxmox_name`, `node` → `proxmox_node`, `vmid` → `proxmox_vmid`, `type` → `proxmox_type`, `status` → `proxmox_status`
5. Add `plugin_config_id` field (already on model, now explicit in JSON)
6. Secondary batch query for `config_name` after mapping load
7. `tracing::debug!` format string updated to handle `Option<Uuid>`

- [ ] **Step 1: Write failing tests**

Add inside `mod tests` at the bottom of `surfaces.rs`:

```rust
fn mock_proxmox_host_mapping(
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    name: &str,
) -> uptrakit_shared_db::entity::proxmox_host_mapping::Model {
    use uptrakit_shared_db::entity::proxmox_host_mapping;
    let now = time::OffsetDateTime::now_utc();
    proxmox_host_mapping::Model {
        id: Uuid::now_v7(),
        tenant_id,
        plugin_config_id,
        host_id: None,
        proxmox_node: "node1".to_string(),
        proxmox_vmid: 100,
        proxmox_type: "qemu".to_string(),
        proxmox_name: Some(name.to_string()),
        proxmox_status: "running".to_string(),
        hostname: None,
        ip_addresses: None,
        machine_id: None,
        match_method: None,
        discovered_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn handle_list_without_plugin_config_id_returns_all_tenant_mappings() {
    let tenant_id = Uuid::now_v7();
    let config_id1 = Uuid::now_v7();
    let config_id2 = Uuid::now_v7();

    // Query order:
    // 1. COUNT (paginator)
    // 2. paginated SELECT (proxmox_host_mapping rows)
    // 3. host SELECT for suggestions (tenant scope, all unmatched → empty for simplicity)
    // 4. plugin_config batch SELECT for config names
    let db = MockDatabase::new(DbBackend::MySql)
        .append_query_results([[serde_json::json!({"num_items": 2_u64})]])
        .append_query_results([[
            mock_proxmox_host_mapping(tenant_id, config_id1, "vm1"),
            mock_proxmox_host_mapping(tenant_id, config_id2, "vm2"),
        ]])
        .append_query_results([Vec::<uptrakit_shared_db::entity::host::Model>::new()])
        .append_query_results([[
            mock_plugin_config_model(tenant_id, config_id1),
            mock_plugin_config_model(tenant_id, config_id2),
        ]])
        .into_connection();

    let result = handle_list(&db, Some(tenant_id), serde_json::json!({}))
        .await
        .expect("handle_list should succeed without plugin_config_id");

    let items = result["items"].as_array().expect("items must be an array");
    assert_eq!(items.len(), 2, "both mappings returned");

    // Both rows must include config_name
    for item in items {
        assert!(item["config_name"].as_str().is_some(), "config_name must be present");
        assert!(item["plugin_config_id"].as_str().is_some(), "plugin_config_id must be present");
        assert!(item["proxmox_name"].is_string() || item["proxmox_name"].is_null());
    }
}

#[tokio::test]
async fn handle_list_with_plugin_config_id_filters_to_that_config() {
    let tenant_id = Uuid::now_v7();
    let config_id = Uuid::now_v7();

    let db = MockDatabase::new(DbBackend::MySql)
        .append_query_results([[serde_json::json!({"num_items": 1_u64})]])
        .append_query_results([[mock_proxmox_host_mapping(tenant_id, config_id, "filtered-vm")]])
        .append_query_results([Vec::<uptrakit_shared_db::entity::host::Model>::new()])
        .append_query_results([[mock_plugin_config_model(tenant_id, config_id)]])
        .into_connection();

    let result = handle_list(
        &db,
        Some(tenant_id),
        serde_json::json!({ "plugin_config_id": config_id.to_string() }),
    )
    .await
    .expect("handle_list should succeed with plugin_config_id");

    let items = result["items"].as_array().expect("items must be an array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["plugin_config_id"].as_str().unwrap(), config_id.to_string());
    assert_eq!(items[0]["config_name"].as_str().unwrap(), "PVE Main");
}
```

> **Note on COUNT mock:** SeaORM 2.x MockDatabase's COUNT query result format may require adjustment. If
> `serde_json::json!({"num_items": 2_u64})` does not work, check SeaORM 2.x `MockDatabase` docs or source for the correct
> `IntoMockRow` format for paginator count queries. An alternative is
> `sea_orm::mock::MockExecResult { rows_affected: 0, last_insert_id: 0 }` for exec results, but COUNT is a select result.

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox handle_list -- --nocapture
```

Expected: compile error (test references new fields/behavior not yet implemented) or test logic failure.

- [ ] **Step 3: Implement changes to `handle_list`**

Replace the entire `handle_list` function body with:

```rust
async fn handle_list(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    use uptrakit_shared_db::entity::{host, plugin_config, proxmox_host_mapping};

    let plugin_config_id: Option<Uuid> = parse_optional_uuid_param(&params, "plugin_config_id")?;
    let page = parse_pagination_page(&params);
    let per_page = parse_pagination_per_page(&params);

    tracing::debug!(?plugin_config_id, %page, %per_page, "listing Proxmox host mappings");

    let mut base_query = proxmox_host_mapping::Entity::find()
        .inner_join(plugin_config::Entity); // FK guarantees no orphaned mappings

    if let Some(pcid) = plugin_config_id {
        base_query =
            base_query.filter(proxmox_host_mapping::Column::PluginConfigId.eq(pcid));
    }
    if let Some(tid) = tenant_id {
        base_query =
            base_query.filter(proxmox_host_mapping::Column::TenantId.eq(tid));
    }

    let base_query = base_query
        .order_by(
            sea_orm::sea_query::Func::lower(sea_orm::sea_query::Expr::col(
                proxmox_host_mapping::Column::ProxmoxName,
            )),
            sea_orm::sea_query::Order::Asc,
        )
        .order_by_asc(proxmox_host_mapping::Column::ProxmoxVmid);

    let total = base_query
        .clone()
        .count(db)
        .await
        .map_err(|e| format!("database error counting mappings: {e}"))?;

    let offset = (page.saturating_sub(1)) * per_page;
    let mappings = base_query
        .offset(Some(offset))
        .limit(Some(per_page))
        .all(db)
        .await
        .map_err(|e| format!("database error: {e}"))?;

    let total_pages = if per_page == 0 {
        0
    } else {
        total.div_ceil(per_page)
    };

    // Collect IDs of already-matched hosts on this page for suggestion filtering
    let matched_host_ids: std::collections::HashSet<Uuid> =
        mappings.iter().filter_map(|m| m.host_id).collect();

    // Collect unmatched mappings on this page for suggestion computation
    let unmatched_mappings: Vec<&proxmox_host_mapping::Model> =
        mappings.iter().filter(|m| m.host_id.is_none()).collect();

    // Load active hosts for suggestions (only if there are unmatched mappings on this page)
    let suggestion_map = if !unmatched_mappings.is_empty() {
        if let Some(tid) = tenant_id {
            let all_hosts: Vec<host::Model> = host::Entity::find()
                .filter(host::Column::TenantId.eq(tid))
                .filter(host::Column::DeactivatedAt.is_null())
                .all(db)
                .await
                .map_err(|e| format!("database error loading hosts: {e}"))?;

            let available_hosts: Vec<host::Model> = all_hosts
                .into_iter()
                .filter(|h| !matched_host_ids.contains(&h.id))
                .collect();

            let unmatched_owned: Vec<proxmox_host_mapping::Model> =
                unmatched_mappings.into_iter().cloned().collect();

            let suggestions =
                crate::matching::compute_suggestions(&unmatched_owned, &available_hosts);
            crate::matching::suggestions_by_mapping_id(suggestions)
        } else {
            std::collections::HashMap::new()
        }
    } else {
        std::collections::HashMap::new()
    };

    // Batch-load config names for all plugin_config_ids on this page.
    // Keyed by plugin_config_id so each row can look up its config name.
    let config_ids_on_page: Vec<Uuid> = mappings
        .iter()
        .map(|m| m.plugin_config_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let config_name_map: std::collections::HashMap<Uuid, String> =
        if config_ids_on_page.is_empty() {
            std::collections::HashMap::new()
        } else {
            let configs = plugin_config::Entity::find()
                .filter(plugin_config::Column::Id.is_in(config_ids_on_page))
                .all(db)
                .await
                .map_err(|e| format!("database error loading config names: {e}"))?;
            configs.into_iter().map(|c| (c.id, c.name)).collect()
        };

    let items: Vec<serde_json::Value> = mappings
        .into_iter()
        .map(|m| {
            let mapping_id = m.id;
            let config_name = config_name_map
                .get(&m.plugin_config_id)
                .cloned()
                .unwrap_or_default();

            let mut row = serde_json::json!({
                "id": m.id.to_string(),
                "mapping_id": m.id.to_string(),
                "plugin_config_id": m.plugin_config_id.to_string(),
                "config_name": config_name,
                "proxmox_name": m.proxmox_name,
                "proxmox_node": m.proxmox_node,
                "proxmox_vmid": m.proxmox_vmid,
                "proxmox_type": m.proxmox_type,
                "proxmox_status": m.proxmox_status,
                "hostname": m.hostname,
                "ip_addresses": m.ip_addresses,
                "matched_host": m.host_id.map(|id| id.to_string()),
                "match_method": m.match_method,
            });

            if let Some(suggestion) = suggestion_map.get(&mapping_id) {
                row["suggested_host"] = serde_json::json!(suggestion.host_name);
                row["suggested_host_id"] = serde_json::json!(suggestion.host_id.to_string());
                row["match_confidence"] = serde_json::json!(suggestion.confidence.as_str());
                row["match_reason"] = serde_json::json!(suggestion.reason);
                row["suggested_match_method"] =
                    serde_json::json!(suggestion.match_method.as_str());
            }

            row
        })
        .collect();

    tracing::debug!(?plugin_config_id, item_count = items.len(), %total, "host mappings listed");
    Ok(serde_json::json!({
        "items": items,
        "total": total,
        "page": page,
        "per_page": per_page,
        "total_pages": total_pages,
    }))
}
```

> **Import note:** `plugin_config` is imported via `use uptrakit_shared_db::entity::plugin_config;` inside the function
> (matching the existing pattern in `load_proxmox_config`). The `is_in` filter uses `sea_orm::prelude::Iterable` — verify
> `ColumnTrait` (already imported at top) provides the `is_in` method.

- [ ] **Step 4: Run tests**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox handle_list -- --nocapture
```

Expected: both `handle_list` tests pass.

- [ ] **Step 5: Run full crate tests**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox --all-features
```

Expected: all tests pass (existing tests continue to pass).

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/surfaces.rs
git commit -m "feat(proxmox): make handle_list optional on plugin_config_id, add config_name batch lookup"
```

---

## Task 8: Proxmox plugin registration — replace boundary surface

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/plugin.rs`

- [ ] **Step 1: Write failing test**

Add inside the `#[cfg(test)]` / `mod tests` block in `plugin.rs` (check if one exists; add if not):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxmox_hosts_surface_has_full_table_layout() {
        let registrations = proxmox_surface_registrations();
        let reg = &registrations[0];
        let hosts = reg
            .surfaces
            .iter()
            .find(|s| s.descriptor.surface_id.as_str() == "proxmox.hosts")
            .expect("proxmox.hosts surface must be registered");

        // context_selector present with correct param key
        let selector = hosts
            .descriptor
            .context_selector
            .as_ref()
            .expect("proxmox.hosts must declare a context_selector");
        assert_eq!(selector.param_key, "plugin_config_id");
        assert!(selector
            .required_for_interactions
            .contains(&"discover".to_string()));
        assert!(selector
            .required_for_interactions
            .contains(&"test-connection".to_string()));

        // root is a section containing action bar + table
        let children = match &hosts.descriptor.root_node {
            surfaces::SurfaceNode::Section { children, .. } => children,
            other => panic!("expected section root, got {other:?}"),
        };
        assert!(
            children
                .iter()
                .any(|n| matches!(n, surfaces::SurfaceNode::ActionBar { .. })),
            "root section must contain an ActionBar"
        );
        let row_actions = children
            .iter()
            .find_map(|n| match n {
                surfaces::SurfaceNode::Table { row_actions, .. } => Some(row_actions),
                _ => None,
            })
            .expect("root section must contain a Table node");

        // row actions
        let action_ids: Vec<&str> = row_actions
            .iter()
            .map(|ra| ra.interaction_id.as_str())
            .collect();
        assert!(action_ids.contains(&"approve-match"));
        assert!(action_ids.contains(&"match"));
        assert!(action_ids.contains(&"unmatch"));

        // unmatch has danger confirmation
        let unmatch = hosts
            .interactions
            .iter()
            .find(|i| i.interaction_id.as_str() == "unmatch")
            .expect("unmatch interaction must be declared");
        assert!(matches!(
            unmatch.confirmation.as_ref().map(|c| &c.severity),
            Some(surfaces::ConfirmationSeverity::Danger)
        ));

        // list interaction present (data load)
        assert!(
            hosts
                .interactions
                .iter()
                .any(|i| i.interaction_id.as_str() == "list"),
            "list interaction must be declared"
        );

        // one paginated data source
        assert_eq!(hosts.data_sources.len(), 1);
        assert!(
            hosts.data_sources[0].pagination.is_some(),
            "data source must have pagination"
        );

        // required_capabilities includes ContextSelector
        assert!(
            hosts
                .descriptor
                .required_capabilities
                .0
                .contains(&surfaces::Capability::ContextSelector),
            "must declare ContextSelector capability"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox proxmox_hosts_surface_has_full_table_layout -- --nocapture
```

Expected: fail — `proxmox_hosts_surface` doesn't exist yet.

- [ ] **Step 3: Add `proxmox_hosts_surface()` function**

Add to `plugin.rs` after the existing `proxmox_hosts_selector_boundary_surface()` function (do NOT delete it yet — delete in the next step):

```rust
fn proxmox_hosts_surface() -> surfaces::RegisteredSurface {
    let data_source_id = surfaces::DataSourceId::new("proxmox.hosts.mappings")
        .expect("literal data source id is valid");

    surfaces::RegisteredSurface {
        descriptor: surfaces::SurfaceDescriptor {
            surface_id: surfaces::SurfaceId::new("proxmox.hosts")
                .expect("literal surface id is valid"),
            label: "Proxmox VE Hosts".to_string(),
            priority: 650,
            slot: surfaces::SLOT_SURFACE_PAGE.to_string(),
            scope: surfaces::Scope::Global,
            targeting: surfaces::Targeting::Universal,
            required_permission: Some(Permission::UpdateHosts.to_string()),
            provider_kind: surfaces::ProviderKind::Plugin,
            required_capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::SectionNode,
                surfaces::Capability::ActionBarNode,
                surfaces::Capability::TableNode,
                surfaces::Capability::DataLoad,
                surfaces::Capability::MutationAction,
                surfaces::Capability::ConfirmableAction,
                surfaces::Capability::ProviderQueryDataSource,
                surfaces::Capability::UniversalTargeting,
                surfaces::Capability::ContextSelector,
            ]),
            root_node: surfaces::SurfaceNode::Section {
                title: None,
                children: vec![
                    surfaces::SurfaceNode::ActionBar {
                        action_ids: vec![
                            surfaces::InteractionId::new("discover")
                                .expect("literal interaction id is valid"),
                            surfaces::InteractionId::new("test-connection")
                                .expect("literal interaction id is valid"),
                        ],
                    },
                    surfaces::SurfaceNode::Table {
                        data_source_id: data_source_id.clone(),
                        columns: vec![
                            surfaces::SurfaceTableColumn {
                                key: "proxmox_name".to_string(),
                                label: "Name".to_string(),
                            },
                            surfaces::SurfaceTableColumn {
                                key: "config_name".to_string(),
                                label: "Configuration".to_string(),
                            },
                            surfaces::SurfaceTableColumn {
                                key: "proxmox_node".to_string(),
                                label: "Node".to_string(),
                            },
                            surfaces::SurfaceTableColumn {
                                key: "proxmox_vmid".to_string(),
                                label: "VMID".to_string(),
                            },
                            surfaces::SurfaceTableColumn {
                                key: "proxmox_type".to_string(),
                                label: "Type".to_string(),
                            },
                            surfaces::SurfaceTableColumn {
                                key: "proxmox_status".to_string(),
                                label: "Status".to_string(),
                            },
                            surfaces::SurfaceTableColumn {
                                key: "hostname".to_string(),
                                label: "Hostname".to_string(),
                            },
                            surfaces::SurfaceTableColumn {
                                key: "matched_host".to_string(),
                                label: "Matched Host".to_string(),
                            },
                            surfaces::SurfaceTableColumn {
                                key: "suggested_host".to_string(),
                                label: "Suggested Match".to_string(),
                            },
                        ],
                        row_actions: vec![
                            surfaces::SurfaceTableRowAction {
                                interaction_id: surfaces::InteractionId::new("approve-match")
                                    .expect("literal interaction id is valid"),
                                visible_when: Some(surfaces::SurfaceRowVisibleWhen {
                                    field: "suggested_host_id".to_string(),
                                    condition: surfaces::SurfaceRowCondition::Present,
                                }),
                            },
                            surfaces::SurfaceTableRowAction {
                                interaction_id: surfaces::InteractionId::new("match")
                                    .expect("literal interaction id is valid"),
                                visible_when: None,
                            },
                            surfaces::SurfaceTableRowAction {
                                interaction_id: surfaces::InteractionId::new("unmatch")
                                    .expect("literal interaction id is valid"),
                                visible_when: Some(surfaces::SurfaceRowVisibleWhen {
                                    field: "matched_host".to_string(),
                                    condition: surfaces::SurfaceRowCondition::Present,
                                }),
                            },
                        ],
                    },
                ],
            },
            context_selector: Some(surfaces::SurfaceContextSelectorDescriptor {
                param_key: "plugin_config_id".to_string(),
                label: "Configuration".to_string(),
                all_option_label: "All Configurations".to_string(),
                rest_api_path: "/api/v1/plugin-configs?plugin_type=infrastructure_proxmox"
                    .to_string(),
                value_field: "id".to_string(),
                label_field: "name".to_string(),
                required_for_interactions: vec![
                    "discover".to_string(),
                    "test-connection".to_string(),
                ],
            }),
        },
        interactions: vec![
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("list")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::DataLoad,
                label: "List Hosts".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: None,
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("discover")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Discover".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: Some(120),
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("test-connection")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Test Connection".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: Some(30),
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("approve-match")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Approve Match".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("match")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::FormSubmit,
                label: "Manual Match".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: Some(surfaces::FormUiDescriptor {
                    fields: vec![
                        surfaces::FormFieldDescriptor {
                            key: "mapping_id".to_string(),
                            label: "Mapping ID".to_string(),
                            field_type: "hidden".to_string(),
                            required: true,
                            placeholder: None,
                            help_text: None,
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                        surfaces::FormFieldDescriptor {
                            key: "host_id".to_string(),
                            label: "Host".to_string(),
                            field_type: "select".to_string(),
                            required: true,
                            placeholder: Some("Select a host".to_string()),
                            help_text: None,
                            default_value: None,
                            options: vec![],
                            select_source: Some(surfaces::FormSelectSource::RestApi {
                                path: "/api/v1/hosts".to_string(),
                                value_field: "id".to_string(),
                                label_field: "friendly_name".to_string(),
                            }),
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                    ],
                    pre_load_interaction_id: None,
                }),
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("unmatch")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Remove Match".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: Some(surfaces::InteractionConfirmation {
                    title: "Remove Match".to_string(),
                    message: "Remove the host mapping for".to_string(),
                    confirm_label: Some("Remove".to_string()),
                    cancel_label: None,
                    severity: surfaces::ConfirmationSeverity::Danger,
                }),
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
        ],
        data_sources: vec![surfaces::DataSourceDescriptor {
            data_source_id,
            kind: surfaces::DataSourceKind::ProviderQuery {
                operation_id: "list".to_string(),
            },
            result_schema: surfaces::SchemaContract::Any,
            pagination: Some(surfaces::DataSourcePagination {
                default_page_size: 50,
                max_page_size: 200,
            }),
            sorting: None,
            filtering: None,
            refresh_policy: surfaces::RefreshPolicy::Manual,
            empty_state: Some(surfaces::DataSourceEmptyState {
                title: "No Proxmox guests found".to_string(),
                description: Some(
                    "Run Discover on a configuration to populate this table.".to_string(),
                ),
            }),
        }],
    }
}
```

> **Field verification:** Before finalizing, check `surfaces::FormSelectSource`, `surfaces::ConfirmationSeverity`,
> `surfaces::DataSourceKind`, `surfaces::RefreshPolicy`, `surfaces::InteractionConfirmation` exist in the
> `uptrakit_plugin_infrastructure_core::surfaces` re-export. If any variant name differs, adjust.

- [ ] **Step 4: Replace call in `proxmox_surface_registrations()`**

In `proxmox_surface_registrations()`, replace:

```rust
    let surfaces = vec![
        proxmox_hosts_selector_boundary_surface(),
```

with:

```rust
    let surfaces = vec![
        proxmox_hosts_surface(),
```

- [ ] **Step 5: Delete `proxmox_hosts_selector_boundary_surface()` function**

Remove the entire `fn proxmox_hosts_selector_boundary_surface() -> surfaces::RegisteredSurface { ... }` function body.
Also delete the old test `proxmox_hosts_surface_makes_selector_boundary_explicit` if it exists in `mod tests`.

- [ ] **Step 6: Run the new test**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox proxmox_hosts_surface_has_full_table_layout -- --nocapture
```

Expected: test passes.

- [ ] **Step 7: Run full crate tests**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox --all-features
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/plugin.rs
git commit -m "feat(proxmox): replace degraded boundary surface with full proxmox_hosts_surface"
```

---

## Task 9: Full quality gate

- [ ] **Step 1: Rust format and checks**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
```

Expected: no errors, no warnings, all tests pass.

- [ ] **Step 2: Frontend checks**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: no lint errors, no type errors, all tests pass, build succeeds.

- [ ] **Step 3: Final commit (if any formatting changes)**

```bash
# Only if cargo fmt or npm run format:check produced changes
git add -p
git commit -m "chore: apply formatting fixes from quality gate"
```
