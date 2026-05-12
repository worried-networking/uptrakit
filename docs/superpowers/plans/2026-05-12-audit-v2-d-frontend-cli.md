# Semantic Audit Logs V2 — Plan D: Frontend and CLI

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface V2's new audit fields in the Dashboard and the CLI. Add a "State" tab to the Audit Logs detail drawer with two
key-value tables and computed-diff highlighting, add a `correlation_id` filter input, and extend CLI human + JSON output to include
the new fields.

**Architecture:** Frontend uses existing primitives (`<TabStrip>`, `<DataTable>`, `<SectionCard>`, `<Button>`). The diff computation
runs client-side over the two snapshot objects — added/removed/changed keys are highlighted via existing `--color-*` tokens. The
correlation_id input is a controlled text field that round-trips through the URL query string. CLI JSON output is additive; CLI
human output gains a "State changes" section rendered only for Stateful entries.

**Tech Stack:** Svelte 5 + TypeScript strict (`frontend/tsconfig.json`), design tokens from `docs/development/ui/tokens.md`,
existing audit-logs page at `frontend/src/routes/audit-logs/+page.svelte`. CLI in `crates/ui/cli/src/commands/audit_logs.rs`.
Source of truth: spec §"Product surface changes".

**Quality gates:** Rust gates as Plan A. Frontend gates: `cd frontend && npm run lint && npm run format:check && npm run check &&
npm run test && npm run build`. Playwright E2E if a fixture is added.

---

## File structure

| File                                                  | Status | Responsibility                                                                                                |
| ----------------------------------------------------- | ------ | ------------------------------------------------------------------------------------------------------------- |
| `crates/shared/web-api-types/src/audit_log.rs`        | modify | Response DTO gains `before_snapshot`, `after_snapshot`, `correlation_id`, `action_kind` (nullable / additive) |
| `crates/ui/web-api/src/routes/audit_logs.rs`          | modify | List endpoint accepts `?correlation_id=`, `?action_kind=`; response serialises new fields                     |
| `crates/ui/web-api-queries/src/queries/audit_logs.rs` | modify | Query layer reads new columns; new filter handlers                                                            |
| `frontend/src/routes/audit-logs/+page.svelte`         | modify | Detail drawer State tab + filter                                                                              |
| `frontend/src/routes/audit-logs/StateTab.svelte`      | create | Two-column key/value diff view                                                                                |
| `frontend/src/routes/audit-logs/diff.ts`              | create | Pure client-side diff helper                                                                                  |
| `frontend/src/routes/audit-logs/diff.test.ts`         | create | Vitest unit tests for diff                                                                                    |
| `frontend/src/lib/api/audit-logs.ts`                  | modify | TypeScript types + query-param handling for `correlation_id`, `action_kind`                                   |
| `crates/ui/cli/src/commands/audit_logs.rs`            | modify | JSON output additive; human output gains "State changes" section                                              |

---

## Task 1: Branch

- [ ] `git checkout -b feat/audit-v2-frontend-cli` from Plan C's branch.

---

## Task 2: Backend DTO extension

**Files:**

- Modify: `crates/shared/web-api-types/src/audit_log.rs`

- [ ] **Step 1: Add the new fields**

  ```rust
  #[derive(Clone, Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
  #[non_exhaustive]
  pub struct AuditLogResponse {
      // … existing fields …
      pub action_kind: String,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub before_snapshot: Option<serde_json::Value>,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub after_snapshot: Option<serde_json::Value>,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub correlation_id: Option<uuid::Uuid>,
  }
  ```

- [ ] **Step 2: Compile + commit**

  ```bash
  cargo check --all-features
  git commit -am "feat(audit-v2): response DTO gains action_kind, snapshots, correlation_id"
  ```

---

## Task 3: Query layer + filter handlers

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/audit_logs.rs`
- Modify: `crates/ui/web-api/src/routes/audit_logs.rs`

- [ ] **Step 1: Write a failing query test**

  ```rust
  #[tokio::test]
  async fn list_filters_by_correlation_id() {
      let app = TestApp::new().await;
      let cid = uuid::Uuid::now_v7();
      app.insert_audit_row_with(|r| { r.correlation_id = Some(cid); }).await;
      app.insert_audit_row_with(|r| { r.correlation_id = None; }).await;

      let rows = app.list_audit_logs(&ListQuery { correlation_id: Some(cid), ..Default::default() }).await;
      assert_eq!(rows.len(), 1);
      assert_eq!(rows[0].correlation_id, Some(cid));
  }

  #[tokio::test]
  async fn list_filters_by_action_kind_stateful() {
      let app = TestApp::new().await;
      app.insert_audit_row_with(|r| { r.action_type = "plugin_config.update".into(); r.action_kind = "stateful".into(); }).await;
      app.insert_audit_row_with(|r| { r.action_type = "auth.login".into(); r.action_kind = "event".into(); }).await;
      let rows = app.list_audit_logs(&ListQuery { action_kind: Some("stateful".into()), ..Default::default() }).await;
      assert_eq!(rows.len(), 1);
  }
  ```

- [ ] **Step 2: Extend `ListQuery` and the SeaORM query builder to accept the new filters**

  ```rust
  #[derive(Debug, Default, serde::Deserialize, utoipa::IntoParams)]
  #[non_exhaustive]
  pub struct ListQuery {
      // … existing filters …
      pub correlation_id: Option<uuid::Uuid>,
      pub action_kind: Option<String>,
  }

  fn apply_filters(query: Select<audit_logs::Entity>, q: &ListQuery) -> Select<audit_logs::Entity> {
      let mut s = query;
      // … existing filter chain …
      if let Some(c) = q.correlation_id { s = s.filter(audit_logs::Column::CorrelationId.eq(c)); }
      if let Some(k) = &q.action_kind { s = s.filter(audit_logs::Column::ActionKind.eq(k.as_str())); }
      s
  }
  ```

- [ ] **Step 3: Run tests, then commit**

  ```bash
  cargo test -p uptrakit-web-api-queries audit_logs
  git commit -am "feat(audit-v2): list-audit-logs accepts correlation_id and action_kind filters"
  ```

---

## Task 4: TypeScript API client + types

**Files:**

- Modify: `frontend/src/lib/api/audit-logs.ts`

- [ ] **Step 1: Extend the TypeScript response type**

  ```typescript
  export type AuditLogEntry = {
    // … existing fields …
    action_kind: "stateful" | "event";
    before_snapshot: Record<string, unknown> | null;
    after_snapshot: Record<string, unknown> | null;
    correlation_id: string | null;
  };

  export type AuditLogQuery = {
    // … existing filters …
    correlation_id?: string;
    action_kind?: "stateful" | "event";
  };
  ```

- [ ] **Step 2: Thread the new filters into the existing fetch helper**

  No structural change; the existing query-string builder picks up the new fields by virtue of `URLSearchParams`.

- [ ] **Step 3: Verify type-check + commit**

  ```bash
  cd frontend && npm run check
  git commit -am "feat(audit-v2): TypeScript types for snapshot + correlation_id filter"
  ```

---

## Task 5: Diff helper + tests (pure TS)

**Files:**

- Create: `frontend/src/routes/audit-logs/diff.ts`
- Create: `frontend/src/routes/audit-logs/diff.test.ts`

- [ ] **Step 1: Write failing tests**

  ```typescript
  import { describe, expect, it } from "vitest";
  import { computeDiff, type DiffEntry } from "./diff";

  describe("computeDiff", () => {
    it("marks added/removed/changed/unchanged keys", () => {
      const before = { name: "alpha", enabled: false, removed_only: 1 };
      const after = { name: "alpha", enabled: true, added_only: "x" };
      const rows = computeDiff(before, after);
      const get = (k: string): DiffEntry => rows.find((r) => r.key === k)!;

      expect(get("name").status).toBe("unchanged");
      expect(get("enabled").status).toBe("changed");
      expect(get("removed_only").status).toBe("removed");
      expect(get("added_only").status).toBe("added");
    });

    it("handles null snapshots", () => {
      expect(computeDiff(null, { a: 1 })).toEqual([
        { key: "a", status: "added", before: undefined, after: 1 },
      ]);
      expect(computeDiff({ a: 1 }, null)).toEqual([
        { key: "a", status: "removed", before: 1, after: undefined },
      ]);
    });

    it("preserves declared key order from after when possible", () => {
      const before = { a: 1, b: 2, c: 3 };
      const after = { c: 30, a: 10, b: 20 };
      const rows = computeDiff(before, after);
      expect(rows.map((r) => r.key)).toEqual(["c", "a", "b"]);
    });
  });
  ```

- [ ] **Step 2: Run tests (expected fail — module missing)**

  Run: `cd frontend && npx vitest run src/routes/audit-logs/diff.test.ts`
  Expected: FAIL.

- [ ] **Step 3: Implement `diff.ts`**

  ```typescript
  export type DiffStatus = "unchanged" | "changed" | "added" | "removed";

  export type DiffEntry = {
    key: string;
    status: DiffStatus;
    before: unknown;
    after: unknown;
  };

  /**
   * Compute a key-by-key diff between two snapshot objects.
   * Order: keys present in `after` first (in `after`'s insertion order),
   * then keys present only in `before` (in `before`'s insertion order).
   */
  export function computeDiff(
    before: Record<string, unknown> | null,
    after: Record<string, unknown> | null,
  ): DiffEntry[] {
    const out: DiffEntry[] = [];
    const beforeKeys = before ? Object.keys(before) : [];
    const afterKeys = after ? Object.keys(after) : [];
    const seen = new Set<string>();

    for (const key of afterKeys) {
      seen.add(key);
      const a = after![key];
      if (!before || !(key in before)) {
        out.push({ key, status: "added", before: undefined, after: a });
        continue;
      }
      const b = before[key];
      const status: DiffStatus = jsonEqual(a, b) ? "unchanged" : "changed";
      out.push({ key, status, before: b, after: a });
    }

    for (const key of beforeKeys) {
      if (seen.has(key)) continue;
      out.push({
        key,
        status: "removed",
        before: before![key],
        after: undefined,
      });
    }

    return out;
  }

  function jsonEqual(a: unknown, b: unknown): boolean {
    return JSON.stringify(a) === JSON.stringify(b);
  }
  ```

- [ ] **Step 4: Run tests**

  Run: same command. Expected: PASS.

- [ ] **Step 5: Commit**

  ```bash
  git add frontend/src/routes/audit-logs/diff.ts frontend/src/routes/audit-logs/diff.test.ts
  git commit -m "feat(audit-v2): client-side diff helper + tests"
  ```

---

## Task 6: `StateTab.svelte` component

**Files:**

- Create: `frontend/src/routes/audit-logs/StateTab.svelte`

- [ ] **Step 1: Build the component using existing primitives**

  ```svelte
  <script lang="ts">
      import SectionCard from '$lib/primitives/SectionCard.svelte';
      import DataTable from '$lib/primitives/DataTable.svelte';
      import { computeDiff, type DiffEntry } from './diff';

      type Props = {
          before: Record<string, unknown> | null;
          after: Record<string, unknown> | null;
      };
      let { before, after }: Props = $props();

      let rows = $derived(computeDiff(before, after));

      function renderValue(v: unknown): string {
          if (v === undefined) return '—';
          if (v === null) return 'null';
          if (typeof v === 'string') return v;
          return JSON.stringify(v);
      }

      const statusToToken: Record<DiffEntry['status'], string> = {
          unchanged: 'var(--text-muted)',
          changed: 'var(--color-warning)',
          added: 'var(--color-success)',
          removed: 'var(--color-danger)',
      };
  </script>

  <SectionCard title="State">
      <DataTable>
          <thead>
              <tr>
                  <th>Key</th>
                  <th>Before</th>
                  <th>After</th>
              </tr>
          </thead>
          <tbody>
              {#each rows as row (row.key)}
                  <tr style:color={statusToToken[row.status]}>
                      <td>{row.key}</td>
                      <td>{renderValue(row.before)}</td>
                      <td>{renderValue(row.after)}</td>
                  </tr>
              {/each}
          </tbody>
      </DataTable>
  </SectionCard>
  ```

  All colors are design-token variables (`--color-warning`, `--color-success`, `--color-danger`, `--text-muted`) per
  `docs/development/ui/tokens.md`. No raw hex, no Tailwind palette utilities.

- [ ] **Step 2: Type-check + commit**

  ```bash
  cd frontend && npm run check
  git commit -am "feat(audit-v2): StateTab component renders before/after diff"
  ```

---

## Task 7: Wire `StateTab` into the audit-logs detail drawer

**Files:**

- Modify: `frontend/src/routes/audit-logs/+page.svelte`

- [ ] **Step 1: Add the tab**

  Locate the existing `<TabStrip>` for the detail drawer. Add a new tab "State" gated by `entry.action_kind === 'stateful'`:

  ```svelte
  {#if selected}
      <TabStrip
          tabs={[
              { id: 'details', label: 'Details' },
              ...(selected.action_kind === 'stateful' ? [{ id: 'state', label: 'State' }] : []),
              { id: 'raw', label: 'Raw' },
          ]}
          bind:active={activeTab}
      />

      {#if activeTab === 'state'}
          <StateTab before={selected.before_snapshot} after={selected.after_snapshot} />
      {/if}
  {/if}
  ```

  Import the component at the top: `import StateTab from './StateTab.svelte';`.

- [ ] **Step 2: Manual smoke**

  Run: `cd frontend && npm run dev`. Open `/audit-logs`, click a Stateful row, confirm the "State" tab renders with the two-column
  diff. Click an Event row, confirm the tab is hidden.

- [ ] **Step 3: Commit**

  ```bash
  git commit -am "feat(audit-v2): detail drawer State tab for Stateful audit rows"
  ```

---

## Task 8: `correlation_id` filter input

**Files:**

- Modify: `frontend/src/routes/audit-logs/+page.svelte`

- [ ] **Step 1: Add the filter input**

  Locate the existing filter bar `<SectionCard title="Filters">` (or equivalent). Add an Input primitive bound to a new state value
  that round-trips via `$page.url.searchParams`:

  ```svelte
  <FormFieldRow label="Correlation ID">
      <Input
          type="text"
          placeholder="00000000-0000-0000-0000-000000000000"
          bind:value={filters.correlation_id}
          oninput={pushFilterToQuery}
      />
  </FormFieldRow>
  ```

  `pushFilterToQuery` mirrors the existing pattern in this page for query-string round-trips.

- [ ] **Step 2: Add a "copy correlation_id" button to detail drawer**

  Adjacent to the `correlation_id` value display, a small `<Button size="sm" variant="ghost">` that copies the UUID to clipboard via
  `navigator.clipboard.writeText(...)`. Toast "Copied" via the existing toast helper.

- [ ] **Step 3: Manual smoke**

  Paste a known correlation_id into the input; confirm the list filters to rows with that correlation_id. Use the copy-button on
  one row, paste into the filter on another tab — confirm the filter applies.

- [ ] **Step 4: Commit**

  ```bash
  git commit -am "feat(audit-v2): correlation_id filter input + copy-button"
  ```

---

## Task 9: Playwright E2E for the State tab

**Files:**

- Create: `frontend/tests/e2e/audit-logs-state-tab.spec.ts` (path follows existing E2E layout — adjust to actual location)

- [ ] **Step 1: Add a seeded fixture row in the test harness**

  Use the existing seeded-tenant fixture. Insert one Stateful audit row with a known before/after and one Event audit row.

- [ ] **Step 2: Write the test**

  ```typescript
  import { expect, test } from "@playwright/test";

  test("state tab renders diff for stateful row and is hidden for event row", async ({
    page,
  }) => {
    await page.goto("/audit-logs");
    await page.getByText("plugin_config.update").first().click();
    await expect(page.getByRole("tab", { name: "State" })).toBeVisible();
    await page.getByRole("tab", { name: "State" }).click();
    await expect(page.getByText("enabled")).toBeVisible();

    await page.goBack();
    await page.getByText("auth.login").first().click();
    await expect(page.getByRole("tab", { name: "State" })).not.toBeVisible();
  });

  test("correlation_id filter narrows list", async ({ page }) => {
    await page.goto("/audit-logs");
    const cid = await page.evaluate(() => /* read from a seeded element */ "");
    await page.getByLabel("Correlation ID").fill(cid);
    await expect(page.getByRole("row")).toHaveCount(/* expected seed count */);
  });
  ```

- [ ] **Step 3: Run tests**

  Run: `cd frontend && npm run test:e2e`. Note: E2E snapshots regenerate on macOS+Chromium only per `docs/development/testing.md`.

- [ ] **Step 4: Commit**

  ```bash
  git commit -am "test(audit-v2): playwright e2e for State tab and correlation_id filter"
  ```

---

## Task 10: CLI JSON output additive

**Files:**

- Modify: `crates/ui/cli/src/commands/audit_logs.rs`

- [ ] **Step 1:** The CLI's JSON output already serialises the response DTO via `serde_json::to_string`. New fields land
      automatically once Task 2 ships. Add a test asserting the four new fields appear in JSON output for a stateful entry:

  ```rust
  #[test]
  fn audit_logs_json_output_includes_v2_fields() {
      let entry = AuditLogResponse {
          // … fields including action_kind = "stateful", before_snapshot = Some(...), after_snapshot = Some(...), correlation_id = Some(...)
          ..stub()
      };
      let json = serde_json::to_string(&entry).expect("serialize");
      assert!(json.contains("\"action_kind\":\"stateful\""));
      assert!(json.contains("\"before_snapshot\""));
      assert!(json.contains("\"after_snapshot\""));
      assert!(json.contains("\"correlation_id\""));
  }
  ```

- [ ] **Step 2: Run + commit**

  ```bash
  cargo test -p uptrakit-cli
  git commit -am "test(audit-v2): CLI JSON output includes V2 fields"
  ```

---

## Task 11: CLI human output gains "State changes" section

**Files:**

- Modify: `crates/ui/cli/src/commands/audit_logs.rs`

- [ ] **Step 1: Add a renderer**

  ```rust
  fn render_state_changes(out: &mut impl std::fmt::Write, entry: &AuditLogResponse) -> std::fmt::Result {
      if entry.action_kind != "stateful" { return Ok(()); }
      let (Some(before), Some(after)) = (&entry.before_snapshot, &entry.after_snapshot) else { return Ok(()); };
      let before_map = before.as_object();
      let after_map = after.as_object();
      writeln!(out, "  State changes:")?;
      // Same diff algorithm as the frontend: keys present in after (in order), then keys only in before.
      // Render only `changed`/`added`/`removed` rows; suppress `unchanged` for compactness.
      // …
      Ok(())
  }
  ```

- [ ] **Step 2: Hook the renderer into the human-output codepath**

  Where the CLI today prints the audit list row, append the State changes section for Stateful rows.

- [ ] **Step 3: Snapshot-test the human output**

  Use `insta` (existing workspace test dep — confirm via `grep insta crates/ui/cli/Cargo.toml`). One snapshot per fixture row.

- [ ] **Step 4: Commit**

  ```bash
  git commit -am "feat(audit-v2): CLI human output renders State changes for Stateful rows"
  ```

---

## Task 12: Final quality gates + push

- [ ] **Step 1: Backend gates**

  ```bash
  cargo fmt --all
  cargo check --all-features
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --all-features
  cargo deny check
  ```

- [ ] **Step 2: Frontend gates**

  ```bash
  cd frontend
  npm run lint
  npm run format:check
  npm run check
  npm run test
  npm run build
  ```

- [ ] **Step 3: Markdownlint**

  ```bash
  markdownlint --config .markdownlint.json '**/*.md'
  ```

- [ ] **Step 4: Push**

  ```bash
  git push -u origin feat/audit-v2-frontend-cli
  ```

---

## Spec coverage check (Plan D scope)

This plan delivers:

- Spec §"Product surface changes" — API DTO additive, list filters extended, UI State tab with diff highlighting, correlation_id
  filter input + copy button, CLI JSON additive, CLI human "State changes" section.
- Frontend uses only existing primitives (`<SectionCard>`, `<DataTable>`, `<TabStrip>`, `<Button>`, `<Input>`, `<FormFieldRow>`) and
  design tokens (`--color-success`, `--color-warning`, `--color-danger`, `--text-muted`).
- E2E coverage for Stateful and Event detail-tab visibility + correlation_id filter behaviour (Task 9).

Deferred to Plan E: documentation deliverables + new ADR.
