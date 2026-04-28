---
title: GitHub Actions Attestation Verification
weight: 140
description: Uptrakit can verify GitHub Actions Sigstore-based SLSA attestations for software releases to protect against supply-chain attacks.
---

# GitHub Actions Attestation Verification

## Overview

Uptrakit can verify [GitHub Actions attestations](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations/using-artifact-attestations-to-establish-provenance-for-builds)
for software releases tracked via the GitHub Releases plugin. Attestations provide
Sigstore-based SLSA provenance that proves a given artifact was produced by a specific
GitHub Actions workflow run, protecting against supply-chain attacks where artifacts
are replaced or tampered with after publication.

Verification is performed via the GitHub Attestations REST API:

```text
GET /repos/{owner}/{repo}/attestations/sha256:{digest}
```

No `gh` CLI or Sigstore tooling is required — all verification uses direct REST API calls.

## How It Works

Attestation verification is a **two-stage** process:

### Stage 1 — Controller (fetch time)

When `fetch_releases` runs for a GitHub-sourced software item, the controller:

1. Locates a checksums file in the release assets (any file whose name contains
   `sha256` or `checksum`, case-insensitive).
2. Downloads the checksums file and parses `<sha256hex>  <filename>` and
   `<sha256hex> *<filename>` lines.
3. Stores the `sha256_digest` for each matching release asset in
   `latest_release_metadata`.
4. Queries the GitHub Attestations API for the first asset with a known digest.
5. Stores the resulting [`AttestationStatus`](#attestationstatus) in
   `latest_release_metadata`.

The attestation status and SHA-256 digests are then propagated into
`ExecuteUpdatePayload.release_info` when an update is triggered.

### Stage 2 — Agent (pre-install)

Before executing a plugin update, the agent independently re-verifies the attestation:

1. Checks whether the release URL is a `https://github.com/{owner}/{repo}/…` URL.
   Non-GitHub updates are unaffected.
2. If the controller already set `attestation_status = Verified`, the agent
   trusts that verdict and proceeds immediately.
3. Otherwise, the agent independently re-queries the GitHub Attestations API using
   the first asset's `sha256_digest`.
4. Based on the result and the `require_attestation` policy, the agent either
   proceeds, warns, or aborts the update.

## AttestationStatus

| Value | Meaning |
| --- | --- |
| `Verified` | At least one attestation was found for the release asset digest. |
| `NotFound` | The GitHub Attestations API returned no attestations (404 or empty array). |
| `Unverified` | The check was skipped or inconclusive (no checksums file, network error, etc.). |

## Configuration

Both options are set in the GitHub Releases plugin configuration
(see [Plugin Configurations](../end-user/plugin-configs.md) for the full reference):

```json
{
  "verify_attestation": true,
  "require_attestation": false
}
```

### `verify_attestation` (default: `true`)

When `true`, the controller downloads the release checksums file, parses SHA-256
digests, and queries the GitHub Attestations API during `fetch_releases`. Results are
stored in `latest_release_metadata` and displayed in the UI.

Set to `false` to disable attestation checking entirely (not recommended for production
use of software from public repositories).

### `require_attestation` (default: `false`)

When `true`, the agent will **abort the update** if the independent re-verification
returns `NotFound`. The update fails with:

```text
attestation verification failed: no GitHub Actions attestation found for {owner}/{repo};
update blocked by require_attestation policy
```

When `false` (default), a warning is emitted in the update output but the update
proceeds. This is the recommended starting point when adopting attestation checks.

> [!CAUTION]
> Setting `require_attestation = true` provides the strongest
> supply-chain guarantee. However, if the upstream project does not use
> `actions/attest` in their release workflow, all updates will fail. Verify that
> attestations are consistently published before enabling this option.

## Example: Enabling Strict Attestation

```json
{
  "auth_token": "ghp_...",
  "verify_attestation": true,
  "require_attestation": true
}
```

With this configuration:

1. The controller verifies attestations at fetch time and displays the status in the UI.
2. The agent independently re-verifies before install.
3. Any release without a valid attestation is blocked.

## UI Indicators

The software item detail page shows attestation status in the **Latest Version** column:

- **Attested** (green badge) — `Verified`
- **Not attested** (red badge) — `NotFound`
- No badge — `Unverified` or attestation checking disabled

The update confirmation modal also displays:

- A green confirmation message when `Verified`.
- A yellow warning when `NotFound` (regardless of `require_attestation`).

## Security Considerations

- The independent agent re-verification uses a fresh `reqwest::Client` with
  `connect_timeout(10s)` and `timeout(30s)`.
- Only `https://github.com/` URLs are checked; GitHub Enterprise and other forges are
  not currently supported in the agent re-verification path.
- The agent does **not** perform Sigstore bundle cryptographic verification — it only
  checks whether the GitHub Attestations API returns a non-empty result for the
  expected SHA-256 digest. Full Sigstore verification may be added in a future version.
- Checksums files filtered out by `asset_patterns` are still used for attestation
  checking (the check operates on raw GitHub assets, not the filtered set).

## See Also

- [Security Architecture](security-architecture.md)
- [Plugin Configurations](../end-user/plugin-configs.md)
- [Wire Protocol](https://github.com/worried-networking/uptrakit/tree/main/docs/api/)
- [GitHub Actions: Using artifact attestations](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations/using-artifact-attestations-to-establish-provenance-for-builds)
