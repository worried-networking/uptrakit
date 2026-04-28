# CI/binstall Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable `cargo binstall` for all 7 binary crates and fix 5 gaps in the
`build-artifacts` CI workflow: versioned archives, SHA-256 checksums, pinned `cross`,
`x86_64-unknown-linux-musl` build target, and per-package version extraction.

**Architecture:** Two independent workstreams that can land in either order.
(A) Add `[package.metadata.binstall]` stanzas to 7 `Cargo.toml` files — no CI changes needed,
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
- Modify: `crates/core/controller-standalone/Cargo.toml` — add `[package.metadata.binstall]`
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
- Modify: `docs/development/releases.md` — update stale asset naming, binary table,
  target table, tool name references

---

### Task 1: Add binstall metadata to 6 binary crates

**Files:**

- Modify: `crates/core/controller/Cargo.toml`
- Modify: `crates/core/controller-standalone/Cargo.toml`
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

`uptrakit-controller-standalone` is a distinct cargo package with its own release tag
(`uptrakit-controller-standalone-v{version}`) and its own binstall stanza.

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

- [ ] **Step 2: Add binstall metadata to `crates/core/controller-standalone/Cargo.toml`**

Append at the end of the file:

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-controller-standalone-v{ version }/uptrakit-controller-standalone-{ version }-{ target }.tar.gz"
bin-dir = "{ bin }{ binary-ext }"
pkg-fmt = "tgz"
```

- [ ] **Step 3: Add binstall metadata to `crates/core/agent/Cargo.toml`**

Append at the end of the file:

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-agent-v{ version }/uptrakit-agent-{ version }-{ target }.tar.gz"
bin-dir = "{ bin }{ binary-ext }"
pkg-fmt = "tgz"
```

- [ ] **Step 4: Add binstall metadata to `crates/core/agent-ssh/Cargo.toml`**

Append at the end of the file:

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-agent-ssh-v{ version }/uptrakit-agent-ssh-{ version }-{ target }.tar.gz"
bin-dir = "{ bin }{ binary-ext }"
pkg-fmt = "tgz"
```

- [ ] **Step 5: Add binstall metadata to `crates/core/mqtt/Cargo.toml`**

Append at the end of the file:

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-mqtt-v{ version }/uptrakit-mqtt-{ version }-{ target }.tar.gz"
bin-dir = "{ bin }{ binary-ext }"
pkg-fmt = "tgz"
```

- [ ] **Step 6: Add binstall metadata to `crates/core/scheduler/Cargo.toml`**

Append at the end of the file:

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-scheduler-v{ version }/uptrakit-scheduler-{ version }-{ target }.tar.gz"
bin-dir = "{ bin }{ binary-ext }"
pkg-fmt = "tgz"
```

- [ ] **Step 7: Add binstall metadata to `crates/ui/cli/Cargo.toml`**

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

- [ ] **Step 8: Verify metadata is parseable**

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
  "uptrakit-controller-standalone",
  "uptrakit-mqtt",
  "uptrakit-scheduler"
]
```

- [ ] **Step 9: Commit**

```bash
git add \
  crates/core/controller/Cargo.toml \
  crates/core/controller-standalone/Cargo.toml \
  crates/core/agent/Cargo.toml \
  crates/core/agent-ssh/Cargo.toml \
  crates/core/mqtt/Cargo.toml \
  crates/core/scheduler/Cargo.toml \
  crates/ui/cli/Cargo.toml
git commit -m "feat(binstall): add cargo-binstall metadata to 7 binary crates"
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
          key: cross-0.2.5-${{ runner.os }}-${{ runner.arch }}

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

- Modify: `Cross.toml`
- Modify: `.github/workflows/release-plz.yml`

**Background:** GNU-linked Linux binaries dynamically link against glibc. Systems running
Ubuntu 20.04, RHEL 8, or similar with glibc < 2.31 cannot run these binaries. musl-linked
binaries are statically linked and run on any Linux. `cross` handles musl cross-compilation
via Docker. The `cross` Docker image for `x86_64-unknown-linux-musl` does not include cmake
or clang by default, but `aws-lc-sys` (a transitive dep via rustls) requires them. `Cross.toml`
must declare a `pre-build` step for the musl target, identical to the existing aarch64 entry.

- [ ] **Step 1: Add musl pre-build entry to `Cross.toml`**

`Cross.toml` currently contains only `[target.aarch64-unknown-linux-gnu]`. Append:

```toml
[target.x86_64-unknown-linux-musl]
pre-build = ["apt-get update && apt-get install -y cmake clang pkg-config"]
```

- [ ] **Step 2: Add musl entry to the build matrix in `.github/workflows/release-plz.yml`**

Locate the `matrix.include` list in the
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
            runner: macos-15-intel
            cross: false
          - target: aarch64-apple-darwin
            runner: macos-latest
            cross: false
```

- [ ] **Step 3: Commit**

```bash
git add Cross.toml .github/workflows/release-plz.yml
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

`uptrakit-controller-standalone` is a distinct cargo package with its own release tag
(`uptrakit-controller-standalone-v{version}`). Its archive is named
`uptrakit-controller-standalone-{version}-{target}.tar.gz` and the binary inside is named
`uptrakit-controller-standalone`.

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
          set -euo pipefail
          # RELEASES is injected from the job-level env: block (line 176 of the workflow).
          # Do not remove that job-level declaration — this step inherits it from there.

          # Args: pkg src arc_prefix inner_name
          #   pkg        - cargo package name (for RELEASES JSON lookup)
          #   src        - temp binary file on disk (e.g. uptrakit-agent-x86_64-...)
          #   arc_prefix - archive name prefix   (e.g. uptrakit-agent)
          #   inner_name - binary name inside archive (usually = arc_prefix, except cli=uptrakit)
          package_and_upload() {
            local pkg="$1" src="$2" arc_prefix="$3" inner_name="$4"
            local tag version archive tmpdir

            tag=$(echo "$RELEASES" | jq -r --arg p "$pkg" \
              '(.[] | select(.package_name==$p) | .tag) // empty')
            version=$(echo "$RELEASES" | jq -r --arg p "$pkg" \
              '(.[] | select(.package_name==$p) | .version) // empty')

            if [ -z "$tag" ]; then
              return 0  # package not in this release — silent skip is correct
            fi
            if [ ! -f "$src" ]; then
              echo "ERROR: $pkg released (tag=$tag) but binary $src not found" >&2
              return 1
            fi

            archive="${arc_prefix}-${version}-${TARGET}.tar.gz"
            tmpdir=$(mktemp -d)
            cp "$src" "${tmpdir}/${inner_name}"
            tar czf "$archive" -C "$tmpdir" "$inner_name"
            rm -rf "$tmpdir"

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

          package_and_upload "uptrakit-controller-standalone" \
            "uptrakit-controller-standalone-${TARGET}" \
            "uptrakit-controller-standalone" \
            "uptrakit-controller-standalone"

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

          # cli: package name is uptrakit-cli but binary inside archive is "uptrakit"
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

- [ ] **Step 3: Update `docs/development/releases.md`**

The doc is stale in several ways. Make these changes:

1. Replace all references to "release-please" with "release-plz" (different project).
   The first line uses a hyperlink — update BOTH the label AND the URL:

   Old: `[release-please](https://github.com/googleapis/release-please)`

   New: `[release-plz](https://github.com/release-plz/release-plz)`

   Change every other occurrence of "release-please" text to "release-plz" throughout
   the file.

2. Update step 4 of the release flow (currently says "4 targets (28 total)"):

   ```text
   4. The `release-plz.yml` workflow builds 7 binaries for 5 targets (35 total), ...
   ```

3. Replace the binary artifacts table. The current table has wrong entries
   (`uptrakit-controller-swagger` — no such artifact; the actual second artifact is
   `uptrakit-controller-standalone`). Replace with:

   | Artifact name | Package | Features |
   | --- | --- | --- |
   | `uptrakit-controller` | uptrakit-controller | embedded-frontend,db-sqlite,oidc,zeroconf,notifications-all (no embedded services) |
   | `uptrakit-controller-standalone` | uptrakit-controller-standalone | embedded-frontend,db-sqlite,oidc,zeroconf,notifications-all,embedded-scheduler,embedded-mqtt,embedded-agent,embedded-ssh-agent |
   | `uptrakit-agent` | uptrakit-agent | (default) |
   | `uptrakit-agent-ssh` | uptrakit-agent-ssh | (default) |
   | `uptrakit-mqtt` | uptrakit-mqtt | (default) |
   | `uptrakit-scheduler` | uptrakit-scheduler | db-all,oidc (no-default-features) |
   | `uptrakit-cli` | uptrakit-cli | (default) |

4. Update the asset naming line:

   Old: `Asset naming: {artifact}-v{version}-{target} (raw binaries, no tarballs).`

   New: `Asset naming: {artifact}-{version}-{target}.tar.gz (archived; single binary at archive root).`

5. Update the target table to add the musl row:

   | Target | Runner | Method |
   | --- | --- | --- |
   | `x86_64-unknown-linux-gnu` | `ubuntu-latest` | native |
   | `x86_64-unknown-linux-musl` | `ubuntu-latest` | `cross` |
   | `aarch64-unknown-linux-gnu` | `ubuntu-latest` | `cross` |
   | `x86_64-apple-darwin` | `macos-13` | native |
   | `aarch64-apple-darwin` | `macos-latest` | native |

6. Update the ARM64 section heading to cover both cross-compiled targets:

   Old heading: `### ARM64 Linux cross-compilation`

   New heading: `### cross-compiled Linux targets`

   Update body: note that both `aarch64-unknown-linux-gnu` and `x86_64-unknown-linux-musl`
   use cross with the `Cross.toml` pre-build hook.

7. Update the attestation example to use the new archive filename format:

   Old: `gh attestation verify uptrakit-controller-v0.0.2-x86_64-unknown-linux-gnu`

   New: `gh attestation verify uptrakit-controller-0.0.2-x86_64-unknown-linux-gnu.tar.gz`

8. Replace the entire configuration files table. The old table references three files that
   do not exist (`.github/release-please-config.json`, `.github/.release-please-manifest.json`,
   `.github/workflows/release-please.yml`). Replace with:

   | File | Purpose |
   | --- | --- |
   | `release-plz.toml` | release-plz package configuration (bump rules, changelog) |
   | `.github/workflows/release-plz.yml` | Release workflow (version bump + artifact builds) |
   | `.github/workflows/docker.yml` | Docker image builds (triggered by `v*` tags) |
   | `Cross.toml` | Cross-compilation settings for cross-compiled Linux targets |

   Note: `release-plz.toml` is at the workspace root (not under `.github/`).

- [ ] **Step 4: Verify YAML syntax**

```bash
python3 -c "import yaml, sys; yaml.safe_load(open('.github/workflows/release-plz.yml'))" && \
  echo "YAML OK"
```

Expected: `YAML OK`

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/release-plz.yml docs/development/releases.md
git commit -m "ci(artifacts): versioned tar.gz archives, SHA-256 checksums, per-package version extraction

Breaking change: asset filenames now include version and .tar.gz suffix.
Scripts downloading assets by exact name must be updated."
```
