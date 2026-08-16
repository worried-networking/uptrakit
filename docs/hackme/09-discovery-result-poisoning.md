# ATK-09: Discovery Result Poisoning

| Field          | Value                                                                            |
| -------------- | -------------------------------------------------------------------------------- |
| Severity       | Low                                                                              |
| Attack surface | Discovery subsystem                                                              |
| Prerequisites  | Ability to modify `/usr/bin/update` or discovery script output on a managed host |
| STRIDE         | Tampering                                                                        |

## Attack description

This is an attempted attack: the source allowlist described under Current mitigations defeats
it before any attacker-controlled repository is ever reached.

1. The attacker modifies the `/usr/bin/update` script on a managed host (requires
   local access or prior compromise of the host).
2. The Proxmox Helper Scripts (PHS) discovery plugin reads this file via
   `cat -- /usr/bin/update` and parses it for script references.
3. The attacker injects a line pointing at an attacker-controlled repository, hoping
   discovery will fetch and trust it -- for example:
   `https://raw.githubusercontent.com/attacker-org/malicious/main/ct/backdoor.sh`
4. This does not work: `parse_phs_scripts` only extracts a slug when the line already
   matches one of the four compile-time-fixed source prefixes in `SOURCES`
   (`community-scripts/ProxmoxVE`, `community-scripts/ProxmoxVED`, `tteck/Proxmox`,
   `worried-networking/uptrakit`). `attacker-org/malicious` matches none of them, so
   the line yields no `DiscoveryTarget` -- at most a diagnostic warning naming the
   (credential-redacted) URL if it happens to be the first unrecognised token on the
   line.
5. The only line an attacker can get parsed at all is one that already matches a real
   source prefix, carrying an attacker-chosen but character-restricted slug
   (`[a-z0-9-]+`; path traversal and other characters rejected). Even then, the plugin
   never fetches the attacker-supplied URL text: it reconstructs the fetch URL from
   the validated slug and the matched source's own canonical template, so the script
   it fetches -- and any `owner`/`repo` values extracted from it -- always come from
   one of the four real upstream repositories, never from attacker-controlled content.
6. Consequently, this path cannot make the controller auto-create software items or
   plugin configs that point at an attacker's repository. The impact below describes
   what this design prevents, not an outcome that occurs today.

## Worst-case impact

The scenario below is what the mitigations under Current mitigations exist to prevent -- it does
not describe an outcome reachable through the `/usr/bin/update` injection path today.

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
  script. A slug is only extracted when the line already matches one of the four
  compile-time-fixed source prefixes in the `SOURCES` table
  (`community-scripts/ProxmoxVE`, `community-scripts/ProxmoxVED`, `tteck/Proxmox`,
  `worried-networking/uptrakit`); the plugin then reconstructs the fetch URL from the
  validated slug and the matched source, never from attacker-supplied URL text. This
  fetch allowlist is source-complete: a poisoned `/usr/bin/update` cannot direct
  fetches to arbitrary hosts, only to slugs under one of the four known repositories.
- **Owner/repo component validation.** Extracted `owner` and `repo` values pass
  through `is_valid_gh_component()` which rejects `/` and `..`. This prevents path
  traversal in API URLs.
- **Credential redaction in diagnostic logs.** The "no secrets in logs" invariant
  (AGENTS.md) applies to the diagnostic WARN emitted when no source prefix matches an
  attacker-supplied URL (step 4 above): `redact_url_userinfo_for_logging()` strips any
  `user:token@` component from that URL before it is logged, so a credential smuggled
  into `/usr/bin/update` cannot leak into the discovery log even in the case where
  nothing else in this attack succeeds.
- **Discovery results are tracked immediately.** Discovered software items are created
  with `enabled: true`. The `featured` flag controls visibility (featured items appear
  individually; non-featured items appear in aggregated host summaries).
- **Discovery allowlist/ignorelist.** Operators can configure discovery allowlists and
  ignorelists to filter which items are discovered and tracked.
- **Plugin config validation.** Auto-created plugin configs pass through the target
  plugin's `validate()` method, which enforces `api_base_url` restrictions (HTTPS
  only, no private hosts for GitHub/GitLab/Forgejo).

## Residual risk

- ~~`is_valid_gh_component()` is minimal.~~ **Fixed.** The validation now enforces a
  strict character allowlist (`[a-zA-Z0-9._-]`), a 100-character length limit, rejects
  empty strings and `..`, matching GitHub's actual naming rules.
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

- ~~Strengthen `is_valid_gh_component()` to enforce a strict character allowlist~~ —
  **Done.** Now uses `[a-zA-Z0-9._-]+` with length and `..` checks.
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
