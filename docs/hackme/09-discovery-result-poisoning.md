# ATK-09: Discovery Result Poisoning

| Field | Value |
| --- | --- |
| Severity | Low |
| Attack surface | Discovery subsystem |
| Prerequisites | Ability to modify `/usr/bin/update` or discovery script output on a managed host |
| STRIDE | Tampering |

## Attack description

1. The attacker modifies the `/usr/bin/update` script on a managed host (requires
   local access or prior compromise of the host).
2. The Proxmox Helper Scripts (PHS) discovery plugin reads this file via
   `cat -- /usr/bin/update` and parses it for script references.
3. The attacker injects lines containing GitHub URLs pointing to attacker-controlled
   repositories. For example:
   `https://raw.githubusercontent.com/attacker-org/malicious/main/ct/backdoor.sh`
4. The PHS parser extracts the slug (`backdoor`) and fetches the script from the
   canonical GitHub URL. If the script contains version extraction logic and
   `owner/repo` references, the discovery plugin creates `DiscoveryTarget` entries
   with attacker-controlled plugin config values.
5. The controller auto-creates software items and plugin configs from the discovery
   results, potentially linking them to the attacker's GitHub repository.
6. On subsequent version checks and updates, the controller fetches releases from
   the attacker's repository and may execute update commands derived from the
   attacker's plugin config.

## Worst-case impact

- **Attacker-controlled update source.** Software items created from poisoned
  discovery results point to the attacker's GitHub repository. Version checks fetch
  release data from this repository, and updates install whatever the attacker
  publishes.
- **Supply chain compromise.** If the auto-created software item is assigned to
  multiple hosts, the attacker's malicious updates propagate across the
  infrastructure.
- **Plugin config injection.** The `DiscoveryTarget.plugin_config` JSON is
  attacker-influenced. While the config is validated by the plugin's `validate()`
  method, the attacker controls the `owner` and `repo` fields used in API URL
  construction.

## Current mitigations

- **Slug validation.** The PHS plugin validates extracted slugs via `is_valid_slug()`,
  which restricts slugs to `[a-z0-9-]+`. Path traversal (`..`) and special characters
  are rejected.
- **Canonical URL reconstruction.** The plugin does not use the raw URL from the
  script. Instead, it reconstructs the fetch URL from the validated slug:
  `https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/{slug}.sh`.
  This effectively allowlists the fetch target to a single GitHub organization.
- **Owner/repo component validation.** Extracted `owner` and `repo` values pass
  through `is_valid_gh_component()` which rejects `/` and `..`. This prevents path
  traversal in API URLs.
- **Discovery results require approval.** Discovered software items are created with
  `discovery_state = "pending"` (or `"approved"` for auto-discovery with existing
  configs). Items in `pending` state are visible but inactive until an admin reviews
  and approves them.
- **Discovery allowlist/ignorelist.** Operators can configure discovery allowlists and
  ignorelists to filter which discovered items are presented for approval.
- **Plugin config validation.** Auto-created plugin configs pass through the target
  plugin's `validate()` method, which enforces `api_base_url` restrictions (HTTPS
  only, no private hosts for GitHub/GitLab/Forgejo).

## Residual risk

- **`is_valid_gh_component()` is minimal.** The validation only checks for `/` and
  `..`. An owner/repo containing characters like `@`, `%`, `#`, or spaces could
  manipulate the URL path when interpolated into
  `{base}/repos/{owner}/{repo}/releases`.
- **Local host compromise is prerequisite.** If the attacker can modify
  `/usr/bin/update`, they likely already have significant access to the host,
  reducing the incremental value of this attack. However, the attack's value lies
  in **propagation** — the compromised host's discovery results affect the
  controller's inventory for all hosts.
- **Auto-discovery may skip approval.** When auto-discovery runs with an existing
  plugin config for the same plugin type, new items may be auto-approved without
  operator review.
- **Version extraction from install scripts.** The PHS plugin fetches and parses
  install scripts to extract version information. While no code from these scripts
  is executed, complex parsing logic could be tricked into extracting incorrect
  versions.

## Recommended improvements

- Strengthen `is_valid_gh_component()` to enforce a strict character allowlist (e.g.,
  `[a-zA-Z0-9._-]+`) matching GitHub's actual username and repository naming rules.
- Add a warning indicator in the UI for software items created by auto-discovery,
  especially those pointing to repositories outside the expected organizations.
- Consider requiring explicit admin approval for all auto-created plugin configs, even
  when a matching plugin config already exists.
- Log the full discovery chain (source host, parsed script, extracted targets) for
  audit purposes, making it easier to trace the origin of suspicious software items.

## References

- [Autodiscovery API](../api/autodiscovery.md)
- [Discovery Allowlist](../api/discovery-allowlist.md)
- [Plugin Guidelines](../development/plugin-guidelines.md)
- `crates/plugins/discovery/proxmox-helper-scripts/src/discovery.rs` — PHS parser
- `crates/plugins/discovery/proxmox-helper-scripts/src/plugin.rs` — PHS plugin
- `crates/plugins/discovery/proxmox-helper-scripts/src/phs_version.sh` — version
  helper script
