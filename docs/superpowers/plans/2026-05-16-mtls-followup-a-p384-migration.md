# mTLS Follow-ups — Plan A: P-384 Key Algorithm Migration

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace every production TLS key generation site in the codebase from P-256
(`rcgen::PKCS_ECDSA_P256_SHA256`) to P-384 (`rcgen::PKCS_ECDSA_P384_SHA384`), and update the OCSP signing
algorithm constant to match.

**Architecture:** Mechanical constant substitution across seven production call sites and ~40 test-module call
sites. No new dependencies, no DB schema changes, no wire format changes. The substitution is
`PKCS_ECDSA_P256_SHA256` → `PKCS_ECDSA_P384_SHA384` (rcgen keygen) and `ECDSA_P256_SHA256_ASN1_SIGNING` →
`ECDSA_P384_SHA384_ASN1_SIGNING` (aws-lc-rs OCSP). One function — `generate_p256_keypair_for_ecies` in
`identity.rs` — is intentionally excluded: ECIES sealed-box encryption depends on P-256 uncompressed key layout.

**Tech Stack:** Rust (edition 2024), rcgen (certificate generation), aws-lc-rs (cryptographic operations)

---

## File Map

**Production sites — modified:**

| File                                          | Lines    | Role                                         |
| --------------------------------------------- | -------- | -------------------------------------------- |
| `crates/core/controller-runtime/src/pki.rs`   | 476, 697 | CA bootstrap keygen, server cert keygen      |
| `crates/ui/web-api/src/routes/server_cert.rs` | 171      | HTTP-triggered server cert renewal           |
| `crates/shared/service-sdk/src/identity.rs`   | 325, 614 | `ensure_keypair`, `generate_keypair_and_csr` |
| `crates/ui/cli/src/commands/auth.rs`          | 1228     | CLI CA trust keypair                         |
| `crates/ui/web-api/src/ocsp.rs`               | 410      | OCSP signing algorithm constant              |

**Test-only sites — mechanical sweep:**

| File                                                              | Lines                                  | Note                     |
| ----------------------------------------------------------------- | -------------------------------------- | ------------------------ |
| `crates/core/controller-runtime/src/pki.rs`                       | 1486, 1495, 1500                       | test module              |
| `crates/core/controller-runtime/src/cert_signer.rs`               | 273, 309, 323, 574                     | test module              |
| `crates/core/controller-runtime/src/crl_manager.rs`               | 655                                    | test module              |
| `crates/core/controller-runtime/src/scheduler/mod.rs`             | 295                                    | test module              |
| `crates/ui/web-api/src/routes/server_cert.rs`                     | 335, 349                               | test module in same file |
| `crates/ui/web-api/src/pki_utils.rs`                              | 236, 247, 262, 272, 281, 297, 308, 316 | test helpers             |
| `crates/ui/web-api/src/extract.rs`                                | 480, 496, 513, 528, 537, 549           | test module              |
| `crates/ui/web-api/src/lib.rs`                                    | 153                                    | test module              |
| `crates/ui/web-api/src/middleware/require_auth.rs`                | 421                                    | test module              |
| `crates/ui/web-api/src/middleware/resolve_ip.rs`                  | 190                                    | test module              |
| `crates/ui/web-api/src/routes/auth.rs`                            | 964                                    | test module              |
| `crates/ui/web-api/src/routes/me_2fa.rs`                          | 936                                    | test module              |
| `crates/ui/web-api/src/routes/mfa.rs`                             | 769                                    | test module              |
| `crates/ui/web-api/src/routes/services.rs`                        | 1466                                   | test module              |
| `crates/ui/web-api/src/routes/settings_nats.rs`                   | 453                                    | test module              |
| `crates/ui/web-api/src/routes/surfaces.rs`                        | 1038                                   | test module              |
| `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`     | 2014, 2073                             | test module              |
| `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`          | 3509                                   | test module              |
| `crates/ui/web-api/src/test_harness/mod.rs`                       | 153                                    | test harness             |
| `crates/shared/service-sdk/src/ca.rs`                             | 377, 400, 411, 435                     | test module              |
| `crates/shared/service-sdk/src/cert_handler.rs`                   | 548                                    | test module              |
| `crates/shared/service-sdk/src/cert_resolver.rs`                  | 127                                    | test module              |
| `crates/shared/service-sdk/src/event_loop.rs`                     | 612                                    | test module              |
| `crates/shared/service-sdk/src/identity.rs`                       | 890, 1022, 1112, 1199                  | test module              |
| `crates/shared/service-sdk/src/tls.rs`                            | 380, 394                               | test module              |
| `crates/core/agent-ssh-runtime/src/ssh_key.rs`                    | 411                                    | test module              |
| `crates/ui/cli/tests/command_execution.rs`                        | 214                                    | test file                |
| `crates/core/integration-tests/tests/database_helpers/harness.rs` | 136                                    | test infrastructure      |
| `crates/core/integration-tests/tests/reverse_proxy/pki.rs`        | 48, 71, 95, 133, 163                   | test infrastructure      |
| `crates/core/integration-tests/tests/reverse_proxy/server.rs`     | 145                                    | test infrastructure      |

**Do NOT migrate (intentionally P-256):**

| File                                        | Line | Reason                                                                       |
| ------------------------------------------- | ---- | ---------------------------------------------------------------------------- |
| `crates/shared/service-sdk/src/identity.rs` | 670  | `generate_p256_keypair_for_ecies` — ECIES uses P-256 uncompressed key format |
| `crates/core/mqtt-runtime/src/handler.rs`   | 289  | ECIES identity for MQTT embedded handler test                                |
| `crates/core/mqtt-runtime/src/lib.rs`       | 1188 | ECIES sealed-box decrypt test                                                |
| `crates/shared/crypto/src/ecies.rs`         | 240  | ECIES module test, intentionally P-256                                       |

**Documentation modified:**

- `docs/security/pki-certificates.md` — algorithm column → P-384
- `docs/development/coding-standards.md` — update any P-256 mention

---

## Task 1: Migrate production sites in controller-runtime and server_cert.rs

**Files:**

- Modify: `crates/core/controller-runtime/src/pki.rs:476, 697`
- Modify: `crates/ui/web-api/src/routes/server_cert.rs:171`

- [ ] **Step 1: Open `crates/core/controller-runtime/src/pki.rs`. Find lines 476 and 697. Both read:**

```rust
KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).context_to::<PkiError>()?;
```

Change both to:

```rust
KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).context_to::<PkiError>()?;
```

- [ ] **Step 2: Open `crates/ui/web-api/src/routes/server_cert.rs`. Find line 171:**

```rust
let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
```

Change to:

```rust
let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384)
```

- [ ] **Step 3: Verify no accidental ECIES site was changed. Run:**

```bash
grep -n "generate_p256_keypair_for_ecies" crates/shared/service-sdk/src/identity.rs
```

Expected: line 667 — function signature `pub fn generate_p256_keypair_for_ecies(...)` is present (identity.rs
not touched in this task). The `PKCS_ECDSA_P256_SHA256` constant inside the function body (line 670) is
verified in Task 2 Step 3 and Task 5 Step 3.

- [ ] **Step 4: Check compilation of modified crates:**

```bash
cargo check -p uptrakit-controller-runtime --all-features 2>&1 | grep -E "^error" | head -20
cargo check -p uptrakit-web-api --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 5: Commit:**

```bash
git add crates/core/controller-runtime/src/pki.rs \
        crates/ui/web-api/src/routes/server_cert.rs
git commit -m "feat(pki): migrate CA bootstrap, server cert, and HTTP renewal keygen to P-384"
```

---

## Task 2: Migrate production sites in service-sdk and CLI; update OCSP constant

**Files:**

- Modify: `crates/shared/service-sdk/src/identity.rs:325, 614`
- Modify: `crates/ui/cli/src/commands/auth.rs:1228`
- Modify: `crates/ui/web-api/src/ocsp.rs:410`

- [ ] **Step 1: Open `crates/shared/service-sdk/src/identity.rs`. Find line 325 (inside `ensure_keypair`):**

```rust
rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| {
```

Change to:

```rust
rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).map_err(|e| {
```

- [ ] **Step 2: In the same file, find line 614 (inside `generate_keypair_and_csr`):**

```rust
let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| {
```

Change to:

```rust
let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).map_err(|e| {
```

- [ ] **Step 3: Verify line 670 (`generate_p256_keypair_for_ecies`) is UNCHANGED. It should still read:**

```rust
let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| {
```

Do not touch this line.

- [ ] **Step 4: Open `crates/ui/cli/src/commands/auth.rs`. Find line 1228:**

```rust
rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("key pair");
```

Change to:

```rust
rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("key pair");
```

- [ ] **Step 5: Open `crates/ui/web-api/src/ocsp.rs`. Find line 410:**

```rust
&aws_lc_rs::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
```

Change to:

```rust
&aws_lc_rs::signature::ECDSA_P384_SHA384_ASN1_SIGNING,
```

- [ ] **Step 6: Verify the ECIES sites in mqtt-runtime and crypto/ecies.rs are unchanged:**

```bash
grep -n "PKCS_ECDSA_P256_SHA256" \
  crates/core/mqtt-runtime/src/handler.rs \
  crates/core/mqtt-runtime/src/lib.rs \
  crates/shared/crypto/src/ecies.rs
```

Expected: these lines still read `P256` — do not change them.

- [ ] **Step 7: Check compilation:**

```bash
cargo check -p uptrakit-service-sdk --all-features 2>&1 | grep -E "^error" | head -20
cargo check -p uptrakit-web-api --all-features 2>&1 | grep -E "^error" | head -20
cargo check -p uptrakit-cli --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 8: Commit:**

```bash
git add crates/shared/service-sdk/src/identity.rs \
        crates/ui/cli/src/commands/auth.rs \
        crates/ui/web-api/src/ocsp.rs
git commit -m "feat(pki): migrate service CSR keygen, CLI CA trust keygen, and OCSP signing to P-384"
```

---

## Task 3: Test-sweep — controller-runtime test sites

**Files:**

- Modify: `crates/core/controller-runtime/src/pki.rs:1486, 1495, 1500`
- Modify: `crates/core/controller-runtime/src/cert_signer.rs:273, 309, 323, 574`
- Modify: `crates/core/controller-runtime/src/crl_manager.rs:655`
- Modify: `crates/core/controller-runtime/src/scheduler/mod.rs:295`

All substitutions are identical: replace `PKCS_ECDSA_P256_SHA256` with `PKCS_ECDSA_P384_SHA384`.

- [ ] **Step 1: Run a sed substitution across these files:**

```bash
for f in \
  crates/core/controller-runtime/src/pki.rs \
  crates/core/controller-runtime/src/cert_signer.rs \
  crates/core/controller-runtime/src/crl_manager.rs \
  crates/core/controller-runtime/src/scheduler/mod.rs
do
  sed -i '' 's/PKCS_ECDSA_P256_SHA256/PKCS_ECDSA_P384_SHA384/g' "$f"
done
```

- [ ] **Step 2: Verify the production sites in pki.rs were already changed in Task 1 (no regression):**

```bash
grep -n "PKCS_ECDSA_P256_SHA256\|PKCS_ECDSA_P384_SHA384" crates/core/controller-runtime/src/pki.rs
```

Expected: zero `P256` lines, five `P384` lines (two production + three test).

- [ ] **Step 3: Verify cert_signer.rs, crl_manager.rs, scheduler/mod.rs have no remaining P256:**

```bash
grep -n "PKCS_ECDSA_P256_SHA256" \
  crates/core/controller-runtime/src/cert_signer.rs \
  crates/core/controller-runtime/src/crl_manager.rs \
  crates/core/controller-runtime/src/scheduler/mod.rs
```

Expected: no output.

- [ ] **Step 4: Check compilation and run unit tests:**

```bash
cargo check -p uptrakit-controller-runtime --all-features 2>&1 | grep -E "^error" | head -20
cargo test -p uptrakit-controller-runtime --all-features 2>&1 | tail -20
```

Expected: no errors, tests pass.

- [ ] **Step 5: Commit:**

```bash
git add crates/core/controller-runtime/src/pki.rs \
        crates/core/controller-runtime/src/cert_signer.rs \
        crates/core/controller-runtime/src/crl_manager.rs \
        crates/core/controller-runtime/src/scheduler/mod.rs
git commit -m "test(pki): migrate controller-runtime test keygen helpers to P-384"
```

---

## Task 4: Test-sweep — web-api test sites

**Files:**

- Modify: `crates/ui/web-api/src/routes/server_cert.rs:335, 349`
- Modify: `crates/ui/web-api/src/pki_utils.rs` (8 sites)
- Modify: `crates/ui/web-api/src/extract.rs` (6 sites)
- Modify: `crates/ui/web-api/src/lib.rs:153`
- Modify: `crates/ui/web-api/src/middleware/require_auth.rs:421`
- Modify: `crates/ui/web-api/src/middleware/resolve_ip.rs:190`
- Modify: `crates/ui/web-api/src/routes/auth.rs:964`
- Modify: `crates/ui/web-api/src/routes/me_2fa.rs:936`
- Modify: `crates/ui/web-api/src/routes/mfa.rs:769`
- Modify: `crates/ui/web-api/src/routes/services.rs:1466`
- Modify: `crates/ui/web-api/src/routes/settings_nats.rs:453`
- Modify: `crates/ui/web-api/src/routes/surfaces.rs:1038`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/messages.rs:2014, 2073`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs:3509`
- Modify: `crates/ui/web-api/src/test_harness/mod.rs:153`
- Modify: `crates/ui/web-api/src/ocsp.rs:479, 491, 531, 641, 661` (test module sites)

- [ ] **Step 1: Run sed substitution across all web-api source files:**

```bash
find crates/ui/web-api/src -name "*.rs" -exec \
  sed -i '' 's/PKCS_ECDSA_P256_SHA256/PKCS_ECDSA_P384_SHA384/g' {} \;
```

- [ ] **Step 2: Verify the production ocsp.rs site was already changed in Task 2 (no regression):**

```bash
grep -n "ECDSA_P256_SHA256_ASN1_SIGNING\|ECDSA_P384_SHA384_ASN1_SIGNING" crates/ui/web-api/src/ocsp.rs
```

Expected: zero `P256_ASN1_SIGNING` lines, one `P384_ASN1_SIGNING` line (production site at line 410).

- [ ] **Step 3: Verify no P256 remains in web-api src:**

```bash
grep -rn "PKCS_ECDSA_P256_SHA256" crates/ui/web-api/src/
```

Expected: no output.

- [ ] **Step 4: Check compilation and run unit tests:**

```bash
cargo check -p uptrakit-web-api --all-features 2>&1 | grep -E "^error" | head -20
cargo test -p uptrakit-web-api --all-features 2>&1 | tail -20
```

Expected: no errors, tests pass.

- [ ] **Step 5: Commit:**

```bash
git add crates/ui/web-api/src/
git commit -m "test(pki): migrate web-api test keygen helpers to P-384"
```

---

## Task 5: Test-sweep — service-sdk, cli, and integration-test sites

**Files:**

- Modify: `crates/shared/service-sdk/src/ca.rs:377, 400, 411, 435`
- Modify: `crates/shared/service-sdk/src/cert_handler.rs:548`
- Modify: `crates/shared/service-sdk/src/cert_resolver.rs:127`
- Modify: `crates/shared/service-sdk/src/event_loop.rs:612`
- Modify: `crates/shared/service-sdk/src/identity.rs:890, 1022, 1112, 1199`
- Modify: `crates/shared/service-sdk/src/tls.rs:380, 394`
- Modify: `crates/core/agent-ssh-runtime/src/ssh_key.rs:411`
- Modify: `crates/ui/cli/tests/command_execution.rs:214`
- Modify: `crates/core/integration-tests/tests/database_helpers/harness.rs:136`
- Modify: `crates/core/integration-tests/tests/reverse_proxy/pki.rs:48, 71, 95, 133, 163`
- Modify: `crates/core/integration-tests/tests/reverse_proxy/server.rs:145`

- [ ] **Step 1: Run sed substitution. `identity.rs` MUST be excluded from the `find` glob — it still contains
      `PKCS_ECDSA_P256_SHA256` at line 670 (`generate_p256_keypair_for_ecies`) and a file-level sed would
      silently migrate it, breaking ECIES. Handle `identity.rs` test sites (lines 890, 1022, 1112, 1199) with
      line-targeted sed:**

```bash
# service-sdk test sites — exclude identity.rs (ECIES line still P256 there)
find crates/shared/service-sdk/src -name "*.rs" ! -name "identity.rs" -exec \
  sed -i '' 's/PKCS_ECDSA_P256_SHA256/PKCS_ECDSA_P384_SHA384/g' {} \;

# identity.rs test sites only — line-targeted to avoid touching line 670
for line in 890 1022 1112 1199; do
  sed -i '' "${line}s/PKCS_ECDSA_P256_SHA256/PKCS_ECDSA_P384_SHA384/" \
    crates/shared/service-sdk/src/identity.rs
done

# agent-ssh-runtime
sed -i '' 's/PKCS_ECDSA_P256_SHA256/PKCS_ECDSA_P384_SHA384/g' \
  crates/core/agent-ssh-runtime/src/ssh_key.rs

# cli test
sed -i '' 's/PKCS_ECDSA_P256_SHA256/PKCS_ECDSA_P384_SHA384/g' \
  crates/ui/cli/tests/command_execution.rs

# integration-tests
sed -i '' 's/PKCS_ECDSA_P256_SHA256/PKCS_ECDSA_P384_SHA384/g' \
  crates/core/integration-tests/tests/database_helpers/harness.rs \
  crates/core/integration-tests/tests/reverse_proxy/pki.rs \
  crates/core/integration-tests/tests/reverse_proxy/server.rs
```

- [ ] **Step 2: CRITICAL — verify `generate_p256_keypair_for_ecies` and ECIES tests are NOT migrated:**

```bash
grep -n "PKCS_ECDSA_P256_SHA256" \
  crates/shared/service-sdk/src/identity.rs \
  crates/core/mqtt-runtime/src/handler.rs \
  crates/core/mqtt-runtime/src/lib.rs \
  crates/shared/crypto/src/ecies.rs
```

Expected: exactly one hit — `identity.rs:670` (`generate_p256_keypair_for_ecies`). mqtt-runtime and ecies.rs should still read P256.

- [ ] **Step 3: Confirm the sed did NOT touch `generate_p256_keypair_for_ecies` line:**

```bash
sed -n '665,675p' crates/shared/service-sdk/src/identity.rs
```

Expected: line 670 still reads `PKCS_ECDSA_P256_SHA256`.

- [ ] **Step 4: Verify no remaining P256 in the swept files:**

```bash
grep -rn "PKCS_ECDSA_P256_SHA256" \
  crates/shared/service-sdk/src/ \
  crates/core/agent-ssh-runtime/src/ \
  crates/ui/cli/tests/ \
  crates/core/integration-tests/tests/database_helpers/ \
  crates/core/integration-tests/tests/reverse_proxy/
```

Expected: no output.

- [ ] **Step 5: Check compilation:**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 6: Run full test suite:**

```bash
cargo test --all-features 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 7: Commit:**

```bash
git add crates/shared/service-sdk/src/ \
        crates/core/agent-ssh-runtime/src/ssh_key.rs \
        crates/ui/cli/tests/command_execution.rs \
        crates/core/integration-tests/tests/database_helpers/harness.rs \
        crates/core/integration-tests/tests/reverse_proxy/
git commit -m "test(pki): migrate service-sdk, cli, and integration-test keygen helpers to P-384"
```

---

## Task 6: Documentation updates

**Files:**

- Modify: `docs/security/pki-certificates.md`
- Check: `docs/development/coding-standards.md`

- [ ] **Step 1: Open `docs/security/pki-certificates.md`. Find the asset lifetimes table or key algorithms section.
      Update the algorithm column for CA, server cert, and service client certs from P-256 / `PKCS_ECDSA_P256_SHA256` to
      P-384 / `PKCS_ECDSA_P384_SHA384`. Add a note:**

```markdown
> **Key algorithm:** All newly generated keys use P-384 (ECDSA, SHA-384). Existing keys continue
> to use P-256 until their normal renewal cycle. The semi-production deployment should trigger
> `POST /api/v1/settings/rotate-ca` after deploying this change to accelerate CA renewal to P-384.
```

- [ ] **Step 2: Search `docs/development/coding-standards.md` for any P-256 reference:**

```bash
grep -n "P-256\|P256\|PKCS_ECDSA_P256" docs/development/coding-standards.md
```

If any hit appears, update the reference to P-384. If no hit, no change needed.

- [ ] **Step 3: Lint docs:**

```bash
npx markdownlint --config .markdownlint.json docs/security/pki-certificates.md docs/development/coding-standards.md
```

Expected: no errors.

- [ ] **Step 4: Commit:**

```bash
git add docs/security/pki-certificates.md docs/development/coding-standards.md
git commit -m "docs(pki): update key algorithm documentation to P-384"
```

---

## Task 7: Full quality gates

- [ ] **Step 1: Format all Rust files:**

```bash
cargo fmt --all
```

Expected: no changes (all edits in Tasks 1–5 are constant substitutions with no formatting impact, but run
anyway to satisfy the pre-commit gate).

- [ ] **Step 2: Full cargo check (both feature sets):**

```bash
cargo check --no-default-features --features db-sqlite 2>&1 | grep -E "^error" | head -20
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 3: Full clippy:**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep -E "^error" | head -20
cargo clippy --all-targets --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 4: Full test suite:**

```bash
cargo test --all-features 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 5: cargo deny:**

```bash
cargo deny check
```

Expected: no violations.

- [ ] **Step 6: Markdown lint:**

```bash
npx markdownlint --config .markdownlint.json '**/*.md'
```

Expected: no errors.

- [ ] **Step 7: Docker integration gates (mandatory — P-384 migration affects cert generation):**

```bash
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
cargo test -p uptrakit-integration-tests -- --ignored --nocapture 2>&1 | tail -40
cargo test -p uptrakit-integration-tests --test reverse_proxy -- --ignored --nocapture 2>&1 | tail -40
```

Expected: all tests pass, including `reverse_proxy` tests that exercise mTLS with the new P-384 keys.
