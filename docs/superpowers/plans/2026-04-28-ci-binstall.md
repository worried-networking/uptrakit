# CI/binstall Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable `cargo binstall` for all 6 binary crates and fix 5 gaps in the
`build-artifacts` CI workflow: versioned archives, SHA-256 checksums, pinned `cross`,
`x86_64-unknown-linux-musl` build target, and per-package version extraction.

**Architecture:** Two independent workstreams that can land in either order.
(A) Add `[package.metadata.binstall]` stanzas to 6 `Cargo.toml` files — no CI changes needed,
verifiable locally with `cargo metadata`. (B) Rework `.github/workflows/release-plz.yml`:
pin cross, add musl matrix entry, and replace the bare-binary upload step with a versioned
tar.gz packaging + checksum step that reads per-package versions from the `release-plz`
JSON output.

**Tech Stack:** GitHub Actions YAML, Bash (tar, sha256sum/shasum), cargo-binstall metadata
TOML format, `jq` for JSON parsing.

---

## File Structure

**Task 1 — binstall metadata:**

- Modify: `crates/core/controller/Cargo.toml` — add `[package.metadata.binstall]`
- Modify: `crates/core/agent/Cargo.toml` — add `[package.metadata.binstall]`
- Modify: `crates/core/agent-ssh/Cargo.toml` — add `[package.metadata.binstall]`
- Modify: `crates/core/mqtt/Cargo.toml` — add `[package.metadata.binstall]`
- Modify: `crates/core/scheduler/Cargo.toml` — add `[package.metadata.binstall]`
- Modify: `crates/ui/cli/Cargo.toml` — add `[package.metadata.binstall]`
  with `bin-name` override (package = `uptrakit-cli`, binary = `uptrakit`)

**Task 2 — workflow: pin cross:**

- Modify: `.github/workflows/release-plz.yml` — replace "Install cross" step with
  cache + pinned install

**Task 3 — workflow: musl matrix entry:**

- Modify: `.github/workflows/release-plz.yml` — add matrix entry

**Task 4 — workflow: versioned archives + checksums + per-package version:**

- Modify: `.github/workflows/release-plz.yml` — replace "Upload release assets" step;
  update "Attest build provenance" glob

---

### Task 1: Add binstall metadata to 6 binary crates

**Files:**

- Modify: `crates/core/controller/Cargo.toml`
- Modify: `crates/core/agent/Cargo.toml`
- Modify: `crates/core/agent-ssh/Cargo.toml`
- Modify: `crates/core/mqtt/Cargo.toml`
- Modify: `crates/core/scheduler/Cargo.toml`
- Modify: `crates/ui/cli/Cargo.toml`

**Background:** release-plz creates per-package git tags using the convention
`{package-name}-v{version}` (e.g. `uptrakit-agent-v0.1.0`). Each binstall stanza must
point to that tag pattern so `cargo binstall` can find the correct release. The archive
format will be `{package}-{version}-{target}.tar.gz` containing a single binary at the
archive root (matching the packaging in Task 4).

For `uptrakit-cli`: the cargo package is named `uptrakit-cli` but the binary inside
is named `uptrakit` (no `-cli` suffix). The `[[package.metadata.binstall.overrides]]`
section maps the binary name correctly.

- [ ] **Step 1: Add binstall metadata to `crates/core/controller/Cargo.toml`**

Append at the end of the file:

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-controller-v{ version }/uptrakit-controller-{ version }-{ target }.tar.gz"
bin-dir = "{ bin }{ binary-ext }"
pkg-fmt = "tgz"
```

- [ ] **Step 2: Add binstall metadata to `crates/core/agent/Cargo.toml`**

Append at the end of the file:

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-agent-v{ version }/uptrakit-agent-{ version }-{ target }.tar.gz"
bin-dir = "{ bin }{ binary-ext }"
pkg-fmt = "tgz"
```

- [ ] **Step 3: Add binstall metadata to `crates/core/agent-ssh/Cargo.toml`**

Append at the end of the file:

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-agent-ssh-v{ version }/uptrakit-agent-ssh-{ version }-{ target }.tar.gz"
bin-dir = "{ bin }{ binary-ext }"
pkg-fmt = "tgz"
```

- [ ] **Step 4: Add binstall metadata to `crates/core/mqtt/Cargo.toml`**

Append at the end of the file:

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-mqtt-v{ version }/uptrakit-mqtt-{ version }-{ target }.tar.gz"
bin-dir = "{ bin }{ binary-ext }"
pkg-fmt = "tgz"
```

- [ ] **Step 5: Add binstall metadata to `crates/core/scheduler/Cargo.toml`**

Append at the end of the file:

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-scheduler-v{ version }/uptrakit-scheduler-{ version }-{ target }.tar.gz"
bin-dir = "{ bin }{ binary-ext }"
pkg-fmt = "tgz"
```

- [ ] **Step 6: Add binstall metadata to `crates/ui/cli/Cargo.toml`**

The `uptrakit-cli` package produces a binary named `uptrakit`. The `overrides` section tells
binstall the on-disk name differs from the package name.

Append at the end of the file:

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-cli-v{ version }/uptrakit-cli-{ version }-{ target }.tar.gz"
pkg-fmt = "tgz"

[[package.metadata.binstall.overrides]]
bin-name = "uptrakit"
bin-dir = "uptrakit{ binary-ext }"
```

- [ ] **Step 7: Verify metadata is parseable**

```bash
cargo metadata --format-version 1 --no-deps | \
  jq '[.packages[] | select(.metadata.binstall != null) | .name]'
```

Expected output (order may vary):

```json
[
  "uptrakit-agent",
  "uptrakit-agent-ssh",
  "uptrakit-cli",
  "uptrakit-controller",
  "uptrakit-mqtt",
  "uptrakit-scheduler"
]
```

- [ ] **Step 8: Commit**

```bash
git add \
  crates/core/controller/Cargo.toml \
  crates/core/agent/Cargo.toml \
  crates/core/agent-ssh/Cargo.toml \
  crates/core/mqtt/Cargo.toml \
  crates/core/scheduler/Cargo.toml \
  crates/ui/cli/Cargo.toml
git commit -m "feat(binstall): add cargo-binstall metadata to 6 binary crates"
```

---

### Task 2: Pin cross to a specific version with caching

**Files:**

- Modify: `.github/workflows/release-plz.yml`

**Background:** The current step compiles `cross` from HEAD on every CI run:

```yaml
- name: Install cross
  if: matrix.cross
  run: cargo install cross --git https://github.com/cross-rs/cross
```

This is slow (full compile), unpinned (broken HEAD can silently break cross-builds), and
unacceptable for reproducible CI. Replace with a pinned version + binary cache.

- [ ] **Step 1: Check the latest stable cross release**

Visit `https://github.com/cross-rs/cross/releases` and note the latest stable tag
(e.g. `v0.2.5`). Use this version in the steps below. The plan uses `0.2.5` — substitute
the actual latest stable if newer.

- [ ] **Step 2: Replace the "Install cross" step in `.github/workflows/release-plz.yml`**

Find this block (around line 184):

```yaml
      - name: Install cross
        if: matrix.cross
        run: cargo install cross --git https://github.com/cross-rs/cross
```

Replace with:

```yaml
      - name: Cache cross binary
        if: matrix.cross
        id: cache-cross
        uses: actions/cache@v4
        with:
          path: ~/.cargo/bin/cross
          key: cross-0.2.5

      - name: Install cross
        if: matrix.cross && steps.cache-cross.outputs.cache-hit != 'true'
        run: cargo install cross --version 0.2.5 --locked
```

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release-plz.yml
git commit -m "ci(cross): pin cross to 0.2.5 with binary caching"
```

---

### Task 3: Add x86_64-unknown-linux-musl build target

**Files:**

- Modify: `.github/workflows/release-plz.yml`

**Background:** GNU-linked Linux binaries dynamically link against glibc. Systems running
Ubuntu 20.04, RHEL 8, or similar with glibc < 2.31 cannot run these binaries. musl-linked
binaries are statically linked and run on any Linux. `cross` handles musl cross-compilation
via Docker.

- [ ] **Step 1: Add musl entry to the build matrix**

In `.github/workflows/release-plz.yml`, locate the `matrix.include` list in the
`build-artifacts` job (around line 161). Add after the `aarch64-unknown-linux-gnu` entry:

```yaml
          - target: x86_64-unknown-linux-musl
            runner: ubuntu-latest
            cross: true
```

The full matrix `include` list should now read:

```yaml
        include:
          - target: x86_64-unknown-linux-gnu
            runner: ubuntu-latest
            cross: false
          - target: aarch64-unknown-linux-gnu
            runner: ubuntu-latest
            cross: true
          - target: x86_64-unknown-linux-musl
            runner: ubuntu-latest
            cross: true
          - target: x86_64-apple-darwin
            runner: macos-13
            cross: false
          - target: aarch64-apple-darwin
            runner: macos-latest
            cross: false
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/release-plz.yml
git commit -m "ci(matrix): add x86_64-unknown-linux-musl build target"
```

---

### Task 4: Versioned archives, checksums, and per-package version extraction

**Files:**

- Modify: `.github/workflows/release-plz.yml`

**Background:** The current workflow produces bare binary files named
`{name}-{target}` (no version, not an archive). binstall requires versioned `.tar.gz`
archives. release-plz produces independent per-package releases with independent versions;
both the tag AND the version per package must be read from the `release-plz.outputs.releases`
JSON to construct correct filenames and upload targets.

binstall does NOT verify sidecar `.sha256` files — those are for human/script use only.
binstall integrity relies on HTTPS transport.

The `uptrakit-controller-standalone` artifact uses the `uptrakit-controller` package tag
(same cargo package, different feature flags). Its archive is named
`uptrakit-controller-standalone-{version}-{target}.tar.gz` and the binary inside is named
`uptrakit-controller`.

For `uptrakit-cli`: the binary inside the archive is named `uptrakit` (matches the actual
binary name and the binstall `bin-dir` set in Task 1).

- [ ] **Step 1: Replace "Upload release assets" step with versioned archive packaging**

In `.github/workflows/release-plz.yml`, find the entire step starting with:

```yaml
      # --- upload to respective releases ---
      - name: Upload release assets
```

Replace that step (through the end of the step's `run:` block) with:

```yaml
      # --- package versioned archives, generate checksums, upload ---
      - name: Package and upload release assets
        env:
          GH_TOKEN: ${{ github.token }}
          TARGET: ${{ matrix.target }}
        run: |
          package_and_upload() {
            local pkg="$1"        # cargo package name (for RELEASES lookup)
            local src="$2"        # temp binary file on disk
            local arc_prefix="$3" # archive name prefix (e.g. uptrakit-agent)
            local inner_name="$4" # binary name inside the archive
            local tag version archive

            tag=$(echo "$RELEASES" | jq -r --arg p "$pkg" \
              '(.[] | select(.package_name==$p) | .tag) // empty')
            version=$(echo "$RELEASES" | jq -r --arg p "$pkg" \
              '(.[] | select(.package_name==$p) | .version) // empty')

            if [ -z "$tag" ] || [ ! -f "$src" ]; then
              return 0
            fi

            archive="${arc_prefix}-${version}-${TARGET}.tar.gz"
            cp "$src" "$inner_name"
            tar czf "$archive" "$inner_name"
            rm -f "$inner_name"

            if command -v sha256sum >/dev/null 2>&1; then
              sha256sum "$archive" > "${archive}.sha256"
            else
              shasum -a 256 "$archive" > "${archive}.sha256"
            fi

            gh release upload "$tag" "$archive" "${archive}.sha256"
          }

          package_and_upload "uptrakit-controller" \
            "uptrakit-controller-${TARGET}" \
            "uptrakit-controller" \
            "uptrakit-controller"

          package_and_upload "uptrakit-controller" \
            "uptrakit-controller-standalone-${TARGET}" \
            "uptrakit-controller-standalone" \
            "uptrakit-controller"

          package_and_upload "uptrakit-agent" \
            "uptrakit-agent-${TARGET}" \
            "uptrakit-agent" \
            "uptrakit-agent"

          package_and_upload "uptrakit-agent-ssh" \
            "uptrakit-agent-ssh-${TARGET}" \
            "uptrakit-agent-ssh" \
            "uptrakit-agent-ssh"

          package_and_upload "uptrakit-mqtt" \
            "uptrakit-mqtt-${TARGET}" \
            "uptrakit-mqtt" \
            "uptrakit-mqtt"

          package_and_upload "uptrakit-scheduler" \
            "uptrakit-scheduler-${TARGET}" \
            "uptrakit-scheduler" \
            "uptrakit-scheduler"

          package_and_upload "uptrakit-cli" \
            "uptrakit-cli-${TARGET}" \
            "uptrakit-cli" \
            "uptrakit"
```

- [ ] **Step 2: Update the "Attest build provenance" glob**

Find:

```yaml
      - name: Attest build provenance
        uses: actions/attest@v4
        with:
          subject-path: "uptrakit-*-${{ matrix.target }}"
```

Replace with:

```yaml
      - name: Attest build provenance
        uses: actions/attest@v4
        with:
          subject-path: "uptrakit-*-${{ matrix.target }}.tar.gz"
```

The glob now matches the versioned archives (e.g.
`uptrakit-agent-0.1.0-x86_64-unknown-linux-gnu.tar.gz`) rather than the bare temp binaries.

- [ ] **Step 3: Verify YAML syntax**

```bash
python3 -c "import yaml, sys; yaml.safe_load(open('.github/workflows/release-plz.yml'))" && \
  echo "YAML OK"
```

Expected: `YAML OK`

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release-plz.yml
git commit -m "ci(artifacts): versioned tar.gz archives, SHA-256 checksums, per-package version extraction

Breaking change: asset filenames now include version and .tar.gz suffix.
Scripts downloading assets by exact name must be updated."
```
