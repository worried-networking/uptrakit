# Clone Reduction — Plan B: surface-proxy or_else

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans`
> to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `Option::or(x.clone())` with `Option::or_else(|| x.clone())` at 7 sites in
`surface-proxy/src/proxy.rs` to avoid cloning when the receiver is already `Some`.

**Architecture:** `Option::or(expr)` evaluates `expr` eagerly — the clone allocates even when the
receiver is `Some` and the fallback is discarded. `Option::or_else(|| expr)` is lazy — the
closure only runs when the receiver is `None`. All 7 sites are in plain `match` arms in
synchronous functions; no async or `move`-closure complications. The values being cloned are
`Option<String>` fields from request params; the clone on the `None` path is unavoidable (the
downstream audit struct requires owned `String`), but deferring it avoids the allocation when
unnecessary.

**Tech Stack:** Rust, `crates/ui/surface-proxy`

---

## Task 1: Replace or(x.clone()) with or_else in proxy.rs

**Files:**

- Modify: `crates/ui/surface-proxy/src/proxy.rs` (7 sites at approx lines 1014, 1182, 1187, 1207, 1212, 1366, 1372)

Some adjacent lines in this file already use `or_else` — this task normalises the whole file to
use `or_else` consistently.

Scan for any additional sites before applying: run
`grep -n '\.or(.*\.clone())' crates/ui/surface-proxy/src/proxy.rs`

- [ ] **Step 1: Apply all changes**

At line ~1014, replace:

```rust
                .or(requested_name.clone());
```

with:

```rust
                .or_else(|| requested_name.clone());
```

At line ~1182, replace:

```rust
                    .or(requested_plugin_config_id.clone()),
```

with:

```rust
                    .or_else(|| requested_plugin_config_id.clone()),
```

At line ~1187, replace:

```rust
                    .or(requested_plugin_config_id.clone()),
```

with:

```rust
                    .or_else(|| requested_plugin_config_id.clone()),
```

At line ~1207, replace:

```rust
                    .or(requested_software_item_id.clone()),
```

with:

```rust
                    .or_else(|| requested_software_item_id.clone()),
```

At line ~1212, replace:

```rust
                    .or(requested_plugin_config_id.clone()),
```

with:

```rust
                    .or_else(|| requested_plugin_config_id.clone()),
```

At line ~1366, replace:

```rust
                .or(requested_id.clone())
```

with:

```rust
                .or_else(|| requested_id.clone())
```

At line ~1372, replace:

```rust
                .or(requested_name.clone());
```

with:

```rust
                .or_else(|| requested_name.clone());
```

- [ ] **Step 2: Verify no sites missed**

```sh
grep -n '\.or(.*\.clone())' crates/ui/surface-proxy/src/proxy.rs
```

Expected: no output (all sites converted).

- [ ] **Step 3: Verify compilation**

```sh
cargo check -p uptrakit-surface-proxy --all-features
```

Expected: no errors.

- [ ] **Step 4: Commit**

```sh
git add crates/ui/surface-proxy/src/proxy.rs
git commit -m "refactor(surface-proxy): replace or(x.clone()) with or_else(|| x.clone()) in audit emit functions"
```

---

## Task 2: Quality gates

- [ ] **Step 1: Format**

```sh
cargo fmt --all
```

- [ ] **Step 2: Full check (both feature sets)**

```sh
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

Expected: no errors.

- [ ] **Step 3: Clippy**

```sh
cargo clippy --all-targets --all-features
```

Expected: no new warnings.

- [ ] **Step 4: Tests**

```sh
cargo test -p uptrakit-surface-proxy --all-features
```

Expected: all pass.
