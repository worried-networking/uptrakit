<!-- markdownlint-disable MD013 -->

# Icon Rollout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Roll out lucide-svelte icons across nav shell, inline SVG replacements, Callout, EmptyState, toast dismiss, and logout button — with full-stack `nav_icon` support on `SurfaceDescriptor`.

**Architecture:** Rust `SurfaceDescriptor` gains an optional `nav_icon: Option<String>` field validated in the wire layer. The frontend resolves nav icon strings to Svelte component references via a curated allowlist (`nav-icons.ts`). All four nav render paths (desktop sidebar, tablet overlay, mobile primary, mobile overflow) and five component sites (Callout, EmptyState, toast, logout, inline SVGs) are updated.

**Tech Stack:** Rust (serde, `#[non_exhaustive]` builder pattern), Svelte 5 (runes, parametric snippets), TypeScript, lucide-svelte 1.0.1, Vitest + @testing-library/svelte, `cargo test`, `npm run test`

---

## Task 1: Rust — `nav_icon` field on `SurfaceDescriptor` + wire validation

**Files:**

- Modify: `crates/shared/surfaces/src/surface.rs:243-421`
- Modify: `crates/shared/wire/src/limits.rs` (add `MAX_NAV_ICON_LEN` constant)
- Modify: `crates/shared/wire/src/wire_validate_impls.rs:728-735` (add check after `required_permission`)
- Test: `crates/shared/wire/src/wire_validate_impls.rs` (append tests near line 1835)

- [ ] **Step 1: Write three failing tests**

  Append to the `#[cfg(test)]` block in `crates/shared/wire/src/wire_validate_impls.rs` (after `surface_registration_rejects_invalid_interaction_confirmation_text`):

  ```rust
  #[test]
  fn surface_registration_rejects_empty_nav_icon() {
      let mut payload = test_surface_registration();
      payload.surfaces[0].descriptor.nav_icon = Some(String::new());
      let err = payload.wire_validate().unwrap_err();
      assert_eq!(err.field, "surfaces[].descriptor.nav_icon");
  }

  #[test]
  fn surface_registration_rejects_oversized_nav_icon() {
      let mut payload = test_surface_registration();
      payload.surfaces[0].descriptor.nav_icon = Some("x".repeat(65)); // 65 > MAX_NAV_ICON_LEN (64)
      let err = payload.wire_validate().unwrap_err();
      assert_eq!(err.field, "surfaces[].descriptor.nav_icon");
  }

  #[test]
  fn surface_registration_accepts_valid_nav_icon() {
      let mut payload = test_surface_registration();
      payload.surfaces[0].descriptor.nav_icon = Some("Package".to_string());
      assert!(payload.wire_validate().is_ok());
  }
  ```

````text

- [ ] **Step 2: Run tests — confirm compile failure (field doesn't exist yet)**

```bash
  cargo test -p uptrakit-wire -- surface_registration_rejects_empty_nav_icon
```text

  Expected: compile error referencing `nav_icon` field not found.

- [ ] **Step 3: Add `nav_icon` field to `SurfaceDescriptor` struct**

  In `crates/shared/surfaces/src/surface.rs`, replace the `context_selector` field + struct closing brace (lines 255-257) with:

  ```rust
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub context_selector: Option<SurfaceContextSelectorDescriptor>,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub nav_icon: Option<String>,
  }
```text

  (The `}` at the end is the struct's existing closing brace — keep it. Only new content is the two `nav_icon` lines.)

- [ ] **Step 4: Add `nav_icon` field to `SurfaceDescriptorBuilder`**

  In `crates/shared/surfaces/src/surface.rs`, `SurfaceDescriptorBuilder` struct (after `context_selector` field, line 306):

  ```rust
      context_selector: Option<SurfaceContextSelectorDescriptor>,
      nav_icon: Option<String>,
```text

- [ ] **Step 5: Add `nav_icon()` setter to `SurfaceDescriptorBuilder` impl**

  After the `context_selector` setter method (around line 385):

  ```rust
      /// Sets the nav icon name (optional; must match a key in the frontend `SURFACE_NAV_ICONS` allowlist).
      #[must_use]
      pub fn nav_icon(mut self, nav_icon: impl Into<String>) -> Self {
          self.nav_icon = Some(nav_icon.into());
          self
      }
```text

- [ ] **Step 6: Wire `nav_icon` in `build()`**

  In `build()` (around line 418), after `context_selector: self.context_selector,`:

  ```rust
              context_selector: self.context_selector,
              nav_icon: self.nav_icon,
```text

- [ ] **Step 7: Run tests — confirm they compile but the validation tests fail**

```bash
  cargo test -p uptrakit-wire -- surface_registration_rejects_empty_nav_icon surface_registration_rejects_oversized_nav_icon surface_registration_accepts_valid_nav_icon
```text

  Expected: `accepts_valid_nav_icon` passes, the two rejection tests fail (no error returned yet).

- [ ] **Step 8a: Add `MAX_NAV_ICON_LEN` constant to `limits.rs`**

  In `crates/shared/wire/src/limits.rs`, insert the following two lines immediately after the `MAX_SHORT_STRING_LEN` block (after line 196):

  ```rust
  /// Maximum byte length for surface nav icon names.
  pub const MAX_NAV_ICON_LEN: usize = 64;
```text

- [ ] **Step 8b: Add wire validation for `nav_icon`**

  In `crates/shared/wire/src/wire_validate_impls.rs`, after the `required_permission` check (after line 732):

  ```rust
              check_opt_string_len(
                  &surface.descriptor.required_permission,
                  MAX_SHORT_STRING_LEN,
                  "surfaces[].descriptor.required_permission",
              )?;
              if let Some(nav_icon) = &surface.descriptor.nav_icon {
                  if nav_icon.is_empty() {
                      return Err(WireValidationError {
                          field: "surfaces[].descriptor.nav_icon",
                          message: "must not be empty".to_string(),
                      });
                  }
                  if nav_icon.len() > MAX_NAV_ICON_LEN {
                      return Err(WireValidationError {
                          field: "surfaces[].descriptor.nav_icon",
                          message: format!("string is {} bytes, max {MAX_NAV_ICON_LEN}", nav_icon.len()),
                      });
                  }
              }
```text

  Note: `MAX_NAV_ICON_LEN` is imported via `use crate::limits::*` at the top of `wire_validate_impls.rs`.

- [ ] **Step 9: Run tests — all three pass**

```bash
  cargo test -p uptrakit-wire -- surface_registration_rejects_empty_nav_icon surface_registration_rejects_oversized_nav_icon surface_registration_accepts_valid_nav_icon
```text

  Expected: all three PASS.

- [ ] **Step 10: Full Rust quality gate**

```bash
  cargo fmt --all && cargo check --all-features && cargo clippy --all-targets --all-features && cargo test -p uptrakit-wire -p uptrakit-surfaces
```bash

  Expected: no errors, no warnings, all tests pass.

- [ ] **Step 11: Commit**

  ```bash
  git add crates/shared/surfaces/src/surface.rs crates/shared/wire/src/limits.rs crates/shared/wire/src/wire_validate_impls.rs
  git commit -m "feat(surfaces): add optional nav_icon field to SurfaceDescriptor with wire validation"
```text

---

## Task 2: Frontend — TS contract + `nav-icons.ts` allowlist

**Files:**

- Modify: `frontend/src/lib/surfaces/contract.ts:138-150`
- Create: `frontend/src/lib/nav-icons.ts`
- Create: `frontend/src/lib/nav-icons.test.ts`

- [ ] **Step 1: Write failing tests for `nav-icons.ts`**

  Create `frontend/src/lib/nav-icons.test.ts`:

  ```typescript
  import { describe, expect, it } from "vitest";
  import { SURFACE_NAV_ICONS, resolveNavIcon } from "./nav-icons";

  describe("resolveNavIcon", () => {
    it("returns Box for an unknown icon name", () => {
      const result = resolveNavIcon("SomeUnknownIcon");
      expect(result).toBe(SURFACE_NAV_ICONS["Box"]);
    });

    it("returns the correct component for a known icon name", () => {
      const result = resolveNavIcon("Package");
      expect(result).toBe(SURFACE_NAV_ICONS["Package"]);
      expect(result).not.toBe(SURFACE_NAV_ICONS["Box"]);
    });

    it("returns Box for empty string", () => {
      const result = resolveNavIcon("");
      expect(result).toBe(SURFACE_NAV_ICONS["Box"]);
    });

    it("SURFACE_NAV_ICONS contains expected keys", () => {
      const expectedKeys = [
        "Box",
        "Cpu",
        "Database",
        "FileText",
        "Globe",
        "HardDrive",
        "History",
        "Layers",
        "Package",
        "Puzzle",
        "ScrollText",
        "Server",
        "ServerCog",
        "Settings",
        "Shield",
        "Tag",
        "Tags",
        "Wrench",
      ];
      for (const key of expectedKeys) {
        expect(
          SURFACE_NAV_ICONS[key],
          `expected key "${key}" in SURFACE_NAV_ICONS`,
        ).toBeDefined();
      }
    });
  });
```text

- [ ] **Step 2: Run test — confirm it fails (module not found)**

```bash
  cd frontend && npm run test -- nav-icons
```text

  Expected: FAIL — `Cannot find module './nav-icons'`.

- [ ] **Step 3: Create `frontend/src/lib/nav-icons.ts`**

  ```typescript
  import type { ComponentType, SvelteComponent } from "svelte";
  import {
    Box,
    Cpu,
    Database,
    FileText,
    Globe,
    HardDrive,
    History,
    Layers,
    Package,
    Puzzle,
    ScrollText,
    Server,
    ServerCog,
    Settings,
    Shield,
    Tag,
    Tags,
    Wrench,
  } from "lucide-svelte";

  export const SURFACE_NAV_ICONS: Record<
    string,
    ComponentType<SvelteComponent>
  > = {
    Box,
    Cpu,
    Database,
    FileText,
    Globe,
    HardDrive,
    History,
    Layers,
    Package,
    Puzzle,
    ScrollText,
    Server,
    ServerCog,
    Settings,
    Shield,
    Tag,
    Tags,
    Wrench,
  };

  export function resolveNavIcon(name: string): ComponentType<SvelteComponent> {
    return SURFACE_NAV_ICONS[name] ?? Box;
  }
```text

- [ ] **Step 4: Run tests — all four pass**

```bash
  cd frontend && npm run test -- nav-icons
```text

  Expected: 4 tests PASS.

- [ ] **Step 5: Add `nav_icon` to `SurfaceDescriptor` in `contract.ts`**

  In `frontend/src/lib/surfaces/contract.ts`, add `nav_icon?: string` after `context_selector?`:

  ```typescript
  export interface SurfaceDescriptor {
    surface_id: SurfaceId;
    label: string;
    priority: number;
    slot: string;
    scope: SurfaceScope;
    targeting: SurfaceTargeting;
    required_permission?: string;
    provider_kind: SurfaceProviderKind;
    required_capabilities: SurfaceCapability[];
    root_node: SurfaceNode;
    context_selector?: SurfaceContextSelector;
    nav_icon?: string;
  }
```text

- [ ] **Step 6: Frontend type-check**

```bash
  cd frontend && npm run check
```bash

  Expected: no errors.

- [ ] **Step 7: Commit**

  ```bash
  git add frontend/src/lib/nav-icons.ts frontend/src/lib/nav-icons.test.ts frontend/src/lib/surfaces/contract.ts
  git commit -m "feat(frontend): add nav-icons allowlist and nav_icon field on SurfaceDescriptor"
```text

---

## Task 3: Frontend — `SurfacePageNavItem` icon + `resolveSurfacePageNavItems`

**Files:**

- Modify: `frontend/src/lib/surfaces/registry.svelte.ts:34-57`
- Modify: `frontend/src/lib/surfaces/registry.test.ts` (update existing expectations + add nav_icon test)

- [ ] **Step 1: Update registry tests to expect `icon` field**

  In `frontend/src/lib/surfaces/registry.test.ts`:
  1. Update the `makeSurface` factory to accept an optional `nav_icon` parameter:

  ```typescript
  function makeSurface({
    surfaceId,
    label,
    priority,
    slot,
    targeting,
    nav_icon,
  }: {
    surfaceId: string;
    label: string;
    priority: number;
    slot: string;
    targeting: "universal" | "targeted";
    nav_icon?: string;
  }): SurfaceResponse {
    return {
      surface_id: surfaceId,
      label,
      priority,
      slot,
      scope: "tenant",
      targeting,
      provider_kind: "service",
      required_capabilities: [],
      root_node: {
        kind: "text_block",
        text: label,
      },
      provider_count: targeting === "targeted" ? 2 : 1,
      nav_icon,
    };
  }
```text

  1. Update the `toEqual` assertion at line 222 to include `icon: 'Box'` (the default):

  ```typescript
  expect(resolveSurfacePageNavItems(slotSurfaces)).toEqual([
    {
      id: "surface.only",
      href: "/surfaces/surface.only",
      label: "Surface Only",
      priority: 50,
      icon: "Box",
    },
    {
      id: "surface.settings",
      href: "/surfaces/surface.settings",
      label: "Surface Settings",
      priority: 100,
      icon: "Box",
    },
  ]);
```text

  1. Add a new test for `nav_icon` pass-through. Append to the `resolveSurfacePageNavItems` describe block:

  ```typescript
  it("passes nav_icon through as icon when set", () => {
    const slotSurfaces = [
      makeSurface({
        surfaceId: "surface.plugin",
        label: "Plugin",
        priority: 100,
        slot: "surface.page",
        targeting: "universal",
        nav_icon: "Package",
      }),
    ];
    expect(resolveSurfacePageNavItems(slotSurfaces)[0].icon).toBe("Package");
  });

  it("defaults icon to Box when nav_icon is absent", () => {
    const slotSurfaces = [
      makeSurface({
        surfaceId: "surface.plugin",
        label: "Plugin",
        priority: 100,
        slot: "surface.page",
        targeting: "universal",
      }),
    ];
    expect(resolveSurfacePageNavItems(slotSurfaces)[0].icon).toBe("Box");
  });
```text

- [ ] **Step 2: Run tests — confirm failures**

```bash
  cd frontend && npm run test -- registry
```text

  Expected: FAIL — `icon` property missing from `SurfacePageNavItem`.

- [ ] **Step 3: Update `SurfacePageNavItem` interface and `resolveSurfacePageNavItems`**

  In `frontend/src/lib/surfaces/registry.svelte.ts`:

  Replace lines 34-57:

  ```typescript
  export interface SurfacePageNavItem {
    id: string;
    href: string;
    label: string;
    priority: number;
    icon: string;
  }

  export function resolveSurfacePageNavItems(
    slotSurfaces: SurfaceResponse[],
  ): SurfacePageNavItem[] {
    const seenSurfaceIds = new SvelteSet<string>();
    const navItems: SurfacePageNavItem[] = [];
    for (const surface of [...slotSurfaces].sort(compareSurfaces)) {
      if (seenSurfaceIds.has(surface.surface_id)) {
        continue;
      }
      seenSurfaceIds.add(surface.surface_id);
      navItems.push({
        id: surface.surface_id,
        href: `/surfaces/${surface.surface_id}`,
        label: surface.label,
        priority: surface.priority,
        icon: surface.nav_icon ?? "Box",
      });
    }
    return navItems;
  }
```text

- [ ] **Step 4: Run tests — all pass**

```bash
  cd frontend && npm run test -- registry
```text

  Expected: all tests PASS.

- [ ] **Step 5: Frontend type-check**

```bash
  cd frontend && npm run check
```bash

  Expected: no errors.

- [ ] **Step 6: Commit**

  ```bash
  git add frontend/src/lib/surfaces/registry.svelte.ts frontend/src/lib/surfaces/registry.test.ts
  git commit -m "feat(frontend): add icon field to SurfacePageNavItem, default to Box"
```text

---

## Task 4: `+layout.svelte` — Script: imports, `ShellNavItem`, built-in icons, map chains

**Files:**

- Modify: `frontend/src/routes/+layout.svelte` (script section)

- [ ] **Step 1: Add lucide imports to `+layout.svelte`**

  At the top of the `<script lang="ts">` block, add after the existing imports (after `Button` import, around line 29):

  ```typescript
  import type { ComponentType, SvelteComponent } from "svelte";
  import {
    History,
    HardDrive,
    House,
    LogOut,
    Menu,
    Monitor,
    Moon,
    Package,
    ScrollText,
    Server,
    ServerCog,
    Settings,
    Sun,
    Tags,
  } from "lucide-svelte";
  import { resolveNavIcon } from "$lib/nav-icons";
```text

- [ ] **Step 2: Add `icon` to `ShellNavItem` type**

  Replace the `ShellNavItem` type definition (lines 62-69):

  ```typescript
  type ShellNavItem = {
    href: string;
    label: string;
    priority: number;
    origin: NavItemOrigin;
    stableId: string;
    badge?: string;
    icon?: ComponentType<SvelteComponent>;
  };
```text

- [ ] **Step 3: Add icons to `builtInNavItems`**

  Replace the `builtInNavItems` const (lines 150-176):

  ```typescript
  const builtInNavItems: {
    href: string;
    label: string;
    priority: number;
    icon: ComponentType<SvelteComponent>;
    permission?: Permission | Permission[];
  }[] = [
    { href: "/", label: "Home", priority: 100, icon: House },
    { href: "/services", label: "Services", priority: 200, icon: Server },
    {
      href: "/system-services",
      label: "System Services",
      priority: 300,
      icon: ServerCog,
      permission: Permission.ViewSystemServices,
    },
    { href: "/hosts", label: "Hosts", priority: 400, icon: HardDrive },
    {
      href: "/host-tags",
      label: "Tags",
      priority: 450,
      icon: Tags,
      permission: Permission.ViewHosts,
    },
    {
      href: "/software",
      label: "Software",
      priority: 500,
      icon: Package,
      permission: Permission.ViewSoftware,
    },
    {
      href: "/history",
      label: "History",
      priority: 800,
      icon: History,
      permission: Permission.ViewSoftware,
    },
    {
      href: "/audit-logs",
      label: "Audit Logs",
      priority: 900,
      icon: ScrollText,
      permission: Permission.ViewAuditLogs,
    },
    {
      href: "/settings",
      label: "Settings",
      priority: 1000,
      icon: Settings,
      permission: [
        Permission.ViewSettings,
        Permission.ManageAuthSettings,
        Permission.ManageEnrollmentTokens,
        Permission.ManageAgentCerts,
        Permission.ViewSoftware,
        Permission.CreateSoftware,
        Permission.UpdateSoftware,
        Permission.DeleteSoftware,
        Permission.ManageScheduler,
        Permission.ManageGlobalSettings,
      ],
    },
  ];
```text

- [ ] **Step 4: Forward `icon` in both derived map chains**

  Replace the `surfacePageNavItems` derived (lines 178-187):

  ```typescript
  const surfacePageNavItems = $derived(
    resolveSurfacePageNavItems(
      getSurfacesBySlot("surface.page").filter((surface) =>
        hasPermissionValue(getUser(), surface.required_permission),
      ),
    ).map((item) => ({
      id: item.id,
      href: item.href,
      label: item.label,
      priority: item.priority,
      icon: item.icon,
    })),
  );
```text

  Replace the `navItems` derived (lines 191-219). In the built-in items `.map()`, add `icon: item.icon`. In the surface items `.map()`, add `icon: resolveNavIcon(item.icon)`:

  ```typescript
  const navItems = $derived(
    [
      ...builtInNavItems
        .filter((item) => {
          if (!item.permission) return true;
          const perms = Array.isArray(item.permission)
            ? item.permission
            : [item.permission];
          return perms.some((p) => getUser()?.permissions.includes(p));
        })
        .map(
          (item): ShellNavItem => ({
            href: item.href,
            label: item.label,
            priority: item.priority,
            origin: "built-in",
            stableId: item.href,
            icon: item.icon,
            badge:
              item.href === "/software"
                ? formatBadge(getUpdatableSoftwareCount())
                : undefined,
          }),
        ),
      ...surfacePageNavItems.map(
        (item): ShellNavItem => ({
          href: item.href,
          label: item.label,
          priority: item.priority,
          origin: "surface.page",
          stableId: item.id,
          icon: resolveNavIcon(item.icon),
        }),
      ),
    ].sort(compareShellNavItems),
  );
```text

- [ ] **Step 5: Frontend type-check**

```bash
  cd frontend && npm run check
```bash

  Expected: no errors.

- [ ] **Step 6: Commit**

  ```bash
  git add frontend/src/routes/+layout.svelte
  git commit -m "feat(frontend): add lucide imports and icon field to ShellNavItem and nav item arrays"
```text

---

## Task 5: `+layout.svelte` — Template: all four nav render paths

All four paths use `{@const NavIcon = item.icon}` as a direct child of `{#each}` (before the `<li>`/`<a>`) — Svelte 5 requires `{@const}` to be a direct child of a block tag, not inside a regular element like `<li>`. Icon size is 16px for sidebar/overlay/overflow, 20px for mobile primary.

**Files:**

- Modify: `frontend/src/routes/+layout.svelte` (template section)

- [ ] **Step 1: Update desktop sidebar `{#each}` block (around lines 495-516)**

  `{@const}` must be a direct child of `{#each}` — placing it inside `<li>` is a Svelte compile error. Replace the entire `{#each navItems as item (item.href)}` block in the desktop sidebar:

  ```svelte
  {#each navItems as item (item.href)}
    {@const NavIcon = item.icon}
    <li>
      <a
        href={item.href}
        class={`flex h-7 items-center gap-2 rounded-card px-2.5 text-nav-item font-medium tracking-nav transition-colors ${
          isNavItemActive(item)
            ? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
            : 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
        }`}
        aria-current={isNavItemActive(item) ? 'page' : undefined}
        data-ui="app-shell-nav-item"
      >
        {#if NavIcon}<NavIcon size={16} aria-hidden="true" />{/if}
        <span>{item.label}</span>
        {#if item.badge}
          <span class="ml-auto pl-1.5">
            <StatusBadge tone="info" label={item.badge} />
          </span>
        {/if}
      </a>
    </li>
  {/each}
```text

- [ ] **Step 2: Update tablet sidebar `{#each}` block (around lines 544-567)**

  Replace the entire `{#each navItems as item (item.href)}` block in the tablet sidebar:

  ```svelte
  {#each navItems as item (item.href)}
    {@const NavIcon = item.icon}
    <li>
      <a
        href={item.href}
        class={`flex h-7 items-center gap-2 rounded-card px-2.5 text-nav-item font-medium tracking-nav transition-colors ${
          isNavItemActive(item)
            ? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
            : 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
        }`}
        aria-current={isNavItemActive(item) ? 'page' : undefined}
        data-ui="app-shell-nav-item"
        onclick={() => (sidebarOverlayOpen = false)}
      >
        {#if NavIcon}<NavIcon size={16} aria-hidden="true" />{/if}
        <span>{item.label}</span>
        {#if item.badge}
          <span class="ml-auto pl-1.5">
            <StatusBadge tone="info" label={item.badge} />
          </span>
        {/if}
      </a>
    </li>
  {/each}
```text

- [ ] **Step 3: Update mobile primary nav item `<a>` (around lines 592-609)**

  Replace the entire `{#each mobilePrimaryNavItems ...}` block (stacked layout, 20px icons):

  ```svelte
  {#each mobilePrimaryNavItems as item (item.href)}
    {@const NavIcon = item.icon}
    <a
      href={item.href}
      class={`flex min-w-0 flex-1 flex-col items-center gap-0.5 justify-center rounded-card px-1 py-1.5 text-center text-nav-item font-medium leading-tight transition-colors ${
        isNavItemActive(item)
          ? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
          : 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
      }`}
      aria-current={isNavItemActive(item) ? 'page' : undefined}
      data-ui="app-shell-mobile-nav-item"
      onclick={closeTransientNavigation}
    >
      {#if NavIcon}<NavIcon size={20} aria-hidden="true" />{/if}
      <span class="truncate">{item.label}</span>
      {#if item.badge}
        <span class="mt-0.5 shrink-0 pl-1.5">
          <StatusBadge tone="info" label={item.badge} />
        </span>
      {/if}
    </a>
  {/each}
```text

- [ ] **Step 4: Update mobile overflow sheet `{#each}` block (around lines 647-668)**

  Replace the entire `{#each mobileOverflowNavItems as item (item.href)}` block:

  ```svelte
  {#each mobileOverflowNavItems as item (item.href)}
    {@const NavIcon = item.icon}
    <li>
      <a
        href={item.href}
        class={`flex h-7 items-center gap-2 rounded-card px-2.5 text-nav-item font-medium tracking-nav transition-colors ${
          isNavItemActive(item)
            ? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
            : 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
        }`}
        aria-current={isNavItemActive(item) ? 'page' : undefined}
        data-ui="app-shell-nav-item"
        onclick={() => (mobileOverflowOpen = false)}
      >
        {#if NavIcon}<NavIcon size={16} aria-hidden="true" />{/if}
        <span>{item.label}</span>
        {#if item.badge}
          <span class="ml-auto pl-1.5">
            <StatusBadge tone="info" label={item.badge} />
          </span>
        {/if}
      </a>
    </li>
  {/each}
```text

- [ ] **Step 6: Frontend type-check**

```bash
  cd frontend && npm run check
```text

  Expected: no errors.

- [ ] **Step 7: Run frontend tests**

```bash
  cd frontend && npm run test
```bash

  Expected: all existing tests pass. If `layout-button-migration.test.ts` or `surface-migration.test.ts` check nav item content, update assertions to match the new icon+label structure.

- [ ] **Step 8: Commit**

  ```bash
  git add frontend/src/routes/+layout.svelte
  git commit -m "feat(frontend): render lucide icons in all four nav templates"
```text

---

## Task 6: `+layout.svelte` — Inline SVG replacement + logout icon

**Files:**

- Modify: `frontend/src/routes/+layout.svelte` (header template section)

- [ ] **Step 1: Replace hamburger SVG with `<Menu>`**

  In the tablet sidebar toggle button (`leadingIcon` snippet, around lines 395-404), replace the inline SVG:

  ```svelte
  {#snippet leadingIcon()}
    <Menu size={16} aria-hidden="true" />
  {/snippet}
```text

- [ ] **Step 2: Replace theme-toggle SVGs with lucide components**

  In the theme toggle button (`leadingIcon` snippet, around lines 423-447), replace all three SVG branches:

  ```svelte
  {#snippet leadingIcon()}
    {#if getThemeMode() === 'light'}
      <Sun size={20} aria-hidden="true" />
    {:else if getThemeMode() === 'dark'}
      <Moon size={20} aria-hidden="true" />
    {:else}
      <Monitor size={20} aria-hidden="true" />
    {/if}
  {/snippet}
```text

- [ ] **Step 3: Add `LogOut` leading icon to logout button**

  Replace the logout button (line 450):

  ```svelte
  <Button variant="danger" onclick={handleLogout}>
    {#snippet leadingIcon()}
      <LogOut size={14} aria-hidden="true" />
    {/snippet}
    Logout
  </Button>
```text

- [ ] **Step 4: Frontend type-check**

```bash
  cd frontend && npm run check
```text

  Expected: no errors.

- [ ] **Step 5: Run frontend tests**

```bash
  cd frontend && npm run test
```bash

  Expected: all tests pass.

- [ ] **Step 6: Commit**

  ```bash
  git add frontend/src/routes/+layout.svelte
  git commit -m "feat(frontend): replace inline SVGs with lucide Menu/Sun/Moon/Monitor, add LogOut to logout button"
```text

---

## Task 7: `Callout` — tone icons

**Files:**

- Modify: `frontend/src/lib/components/ui/Callout.svelte`
- Modify: `frontend/src/lib/components/ui/Callout.test.ts`

- [ ] **Step 1: Write failing test for Callout icon rendering**

  Add to `frontend/src/lib/components/ui/Callout.test.ts`:

  ```typescript
  it("renders a tone icon inside the callout", () => {
    const { container } = render(Callout, {
      tone: "warning",
      title: "Watch out",
      message: "Something needs your attention.",
    });

    const callout = container.querySelector('[data-ui="callout"]');
    expect(callout?.querySelector("svg")).toBeInTheDocument();
  });

  it("renders a danger callout with its icon", () => {
    const { container } = render(Callout, {
      tone: "danger",
      message: "Critical error.",
    });
    const callout = container.querySelector('[data-ui="callout"]');
    expect(callout?.querySelector("svg")).toBeInTheDocument();
  });
```text

- [ ] **Step 2: Run test — confirm it fails (no SVG yet)**

```bash
  cd frontend && npm run test -- Callout
```text

  Expected: FAIL — no `svg` found inside `[data-ui="callout"]`.

- [ ] **Step 3: Update `Callout.svelte`**

  Replace the entire file with:

  ```svelte
  <script lang="ts">
    import type { Snippet } from 'svelte';
    import { Info, CircleCheck, TriangleAlert, OctagonAlert } from 'lucide-svelte';

    export type CalloutTone = 'info' | 'success' | 'warning' | 'danger';

    const toneClasses: Record<CalloutTone, string> = {
      info: 'border-[var(--color-info-border)] bg-[var(--color-info-bg)] text-[var(--color-info)]',
      success: 'border-[var(--color-success-border)] bg-[var(--color-success-bg)] text-[var(--color-success)]',
      warning: 'border-[var(--color-warning-border)] bg-[var(--color-warning-bg)] text-[var(--color-warning)]',
      danger: 'border-[var(--color-danger-border)] bg-[var(--color-danger-bg)] text-[var(--color-danger)]'
    };

    const toneIcons = {
      info: Info,
      success: CircleCheck,
      warning: TriangleAlert,
      danger: OctagonAlert
    };

    let {
      tone = 'info',
      title,
      message,
      children
    }: {
      tone?: CalloutTone;
      title?: string;
      message?: string;
      children?: Snippet;
    } = $props();

    const ToneIcon = $derived(toneIcons[tone]);
    const role = $derived(tone === 'danger' || tone === 'warning' ? 'alert' : 'status');
  </script>

  <aside class={`rounded-panel border px-4 py-3 text-sm ${toneClasses[tone]}`} data-ui="callout" data-tone={tone} {role}>
    <div class="flex items-start gap-3">
      <ToneIcon size={16} class="mt-0.5 shrink-0" aria-hidden="true" />
      <div class="flex-1 min-w-0 space-y-1">
        {#if title}
          <p class="font-semibold">{title}</p>
        {/if}
        {#if message}
          <p class="text-current/90">{message}</p>
        {/if}
        {#if children}
          <div>
            {@render children()}
          </div>
        {/if}
      </div>
    </div>
  </aside>
```text

- [ ] **Step 4: Run tests — all pass**

```bash
  cd frontend && npm run test -- Callout
```text

  Expected: all tests PASS.

- [ ] **Step 5: Frontend type-check and full test run**

```bash
  cd frontend && npm run check && npm run test
```bash

  Expected: no errors, all tests pass.

- [ ] **Step 6: Commit**

  ```bash
  git add frontend/src/lib/components/ui/Callout.svelte frontend/src/lib/components/ui/Callout.test.ts
  git commit -m "feat(frontend): add tone icons to Callout component"
```text

---

## Task 8: `EmptyState` — optional icon prop

**Files:**

- Modify: `frontend/src/lib/components/ui/EmptyState.svelte`
- Modify: `frontend/src/lib/components/ui/EmptyState.test.ts`

- [ ] **Step 1: Write failing test for EmptyState icon rendering**

  Add to `frontend/src/lib/components/ui/EmptyState.test.ts`:

  ```typescript
  import { Package } from "lucide-svelte";

  it("renders an icon above the title when provided", () => {
    const { container } = render(EmptyState, {
      title: "No software items",
      icon: Package,
    });

    const emptyState = container.querySelector('[data-ui="empty-state"]');
    expect(emptyState?.querySelector("svg")).toBeInTheDocument();
    expect(screen.getByText("No software items")).toBeInTheDocument();
  });

  it("renders without an icon when prop is omitted", () => {
    const { container } = render(EmptyState, { title: "Nothing here" });
    const emptyState = container.querySelector('[data-ui="empty-state"]');
    expect(emptyState?.querySelector("svg")).not.toBeInTheDocument();
  });
```text

- [ ] **Step 2: Run test — confirm it fails**

```bash
  cd frontend && npm run test -- EmptyState
```text

  Expected: FAIL — `icon` prop not accepted / no `svg` found.

- [ ] **Step 3: Update `EmptyState.svelte`**

  Replace the entire file with:

  ```svelte
  <script lang="ts">
    import type { ComponentType, SvelteComponent } from 'svelte';
    import type { Snippet } from 'svelte';

    let {
      title,
      description,
      actions,
      icon: IconComponent
    }: {
      title: string;
      description?: string;
      actions?: Snippet;
      icon?: ComponentType<SvelteComponent>;
    } = $props();
  </script>

  <section
    class="rounded-card border border-dashed border-[var(--border-default)] bg-[var(--bg-surface)] px-6 py-8 text-center"
    data-ui="empty-state"
  >
    <div class="mx-auto max-w-md space-y-2">
      {#if IconComponent}
        <div class="mb-3 flex justify-center">
          <IconComponent size={32} class="text-[var(--text-muted)]" aria-hidden="true" />
        </div>
      {/if}
      <h2 class="text-section-title font-semibold text-[var(--text-primary)]">{title}</h2>
      {#if description}
        <p class="text-sm text-[var(--text-secondary)]">{description}</p>
      {/if}
      {#if actions}
        <div class="flex justify-center pt-2">
          {@render actions()}
        </div>
      {/if}
    </div>
  </section>
```text

- [ ] **Step 4: Run tests — all pass**

```bash
  cd frontend && npm run test -- EmptyState
```text

  Expected: all tests PASS.

- [ ] **Step 5: Frontend type-check**

```bash
  cd frontend && npm run check
```bash

  Expected: no errors.

- [ ] **Step 6: Commit**

  ```bash
  git add frontend/src/lib/components/ui/EmptyState.svelte frontend/src/lib/components/ui/EmptyState.test.ts
  git commit -m "feat(frontend): add optional icon prop to EmptyState component"
```text

---

## Task 9: Toast dismiss `X` icon + Logout already done

**Files:**

- Modify: `frontend/src/lib/components/ToastNotifications.svelte`

The `LogOut` icon on the logout button was added in Task 6. This task covers the toast dismiss button. No test is needed beyond the full test run — the dismiss button's behavior is unchanged (onclick still calls `dismissToast`), only the label changes to icon-only.

- [ ] **Step 1: Import `X` from lucide-svelte in `ToastNotifications.svelte`**

  In the `<script lang="ts">` block of `frontend/src/lib/components/ToastNotifications.svelte`, add after the existing imports:

  ```typescript
  import { X } from "lucide-svelte";
```text

- [ ] **Step 2: Replace text "Dismiss" button with icon-only button**

  Find line 406:

  ```svelte
  <Button variant="ghost" size="sm" onclick={() => dismissToast(item)}>Dismiss</Button>
```text

  Replace with:

  ```svelte
  <Button variant="ghost" size="sm" ariaLabel="Dismiss" onclick={() => dismissToast(item)}>
    {#snippet leadingIcon()}
      <X size={14} aria-hidden="true" />
    {/snippet}
  </Button>
```text

- [ ] **Step 3: Frontend type-check and full test run**

```bash
  cd frontend && npm run check && npm run test
```bash

  Expected: no errors, all tests pass.

- [ ] **Step 4: Commit**

  ```bash
  git add frontend/src/lib/components/ToastNotifications.svelte
  git commit -m "feat(frontend): replace toast Dismiss text with X icon button"
```text

---

## Task 10: Quality gates

- [ ] **Step 1: Full Rust quality gate**

```bash
  cargo fmt --all
  cargo check --no-default-features --features db-sqlite
  cargo check --all-features
  cargo clippy --all-targets --no-default-features --features db-sqlite
  cargo clippy --all-targets --all-features
  cargo test --all-features
  cargo deny check
```text

  Expected: all pass, no warnings.

- [ ] **Step 2: Full frontend quality gate**

```bash
  cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```text

  Expected: all pass.

- [ ] **Step 3: Markdown lint**

```bash
  markdownlint --config .markdownlint.json '**/*.md'
```text

  Expected: no errors.

- [ ] **Step 4: Update E2E visual regression snapshots**

  The nav now renders icons, so all existing Playwright snapshots of the nav shell will be outdated. Update them:

```bash
  cd frontend && npx playwright test ui-parity --update-snapshots
  cd frontend && npx playwright test ui-parity-responsive --update-snapshots
```bash

  Review the diff visually to confirm icons appear in the correct positions. Commit the updated snapshots.

  ```bash
  git add frontend/tests/e2e/ui-parity.test.ts-snapshots/ frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots/
  git commit -m "chore(snapshots): update E2E parity snapshots for icon rollout"
```text
````
