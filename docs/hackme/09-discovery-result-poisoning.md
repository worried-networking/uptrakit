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
   the line yields no `DiscoveryTarget`. If -- and only if -- the whole file yields no
   scripts at all, `parse_phs_scripts` emits one diagnostic WARN naming the first
   whitespace-delimited token in the entire file that contains `://`, with its
   userinfo redacted. That token is not necessarily the attacker's line, and not
   necessarily an unrecognised URL: it is simply the first URL-like token in the file,
   which may well be a recognised-prefix URL that was rejected for an invalid slug.
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
- **Userinfo redaction in diagnostic logs.** The "no secrets in logs" invariant
  (AGENTS.md) applies to the diagnostic WARN emitted when no source prefix matches any
  URL in the update file (step 4 above). `redact_url_userinfo_for_logging()` replaces
  the URL's **userinfo** component -- and only that component -- with `***`: the `@`
  must occur after `://` and before the first following `/`, `?`, or `#`, so
  `https://user:token@host/x` is redacted but an `@` later in the path is left alone.
  When the authority holds more than one `@` -- a raw `@` inside the password, as in
  `https://user:p@ss@host/x` -- the **last** one in the authority is the delimiter, so
  no fragment of the secret survives. Nothing else in the URL is touched. A secret carried elsewhere in the same URL -- in
  the query string (`https://host/x?token=SECRET`) or in the fragment -- is logged
  verbatim. Treat the WARN's `first_url` field as untrusted, potentially
  secret-bearing content read off the host, not as sanitised output.
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
  reducing the incremental value of this attack. The source allowlist removes the
  propagation angle: every script body the plugin fetches comes from one of the four
  fixed repositories, so a poisoned host cannot put an attacker-controlled repository
  into the controller's inventory — neither for itself nor for any other host.
- **Slug selection remains attacker-chosen.** What a poisoned `/usr/bin/update` still
  controls is _which_ allowlisted slugs get fetched and analysed. By listing CT-script
  URLs for slugs the container does not actually run, an attacker can make discovery
  fetch those upstream scripts and — for any whose version file or package happens to
  be present on the host — create software items and auto-create the matching
  release-source or package-manager plugin configs, which the controller then polls on
  every version check. The result is inventory noise and unwanted outbound polling of
  legitimate repositories, not an attacker-controlled update source.
- **Auto-discovery may skip approval.** When auto-discovery runs with an existing
  plugin config for the same plugin type, new items may be auto-approved without
  operator review.
- **Script analysis stays heuristic.** The `owner`/`repo` pair, npm package, and APT
  package written into auto-created targets are pattern-matched out of the fetched CT
  script — or, for containers whose CT script names no upstream source at all, out of
  the install script fetched from the same allowlisted source. The attacker cannot
  craft that input, only choose which published script gets parsed; a mis-parse
  therefore yields a wrong package name or a wrong (but real) upstream repository for
  an otherwise legitimate-looking item. No code from these scripts is executed.
- **Reported versions are host-supplied.** The installed version attached to each
  discovered item comes from commands run on the managed host (the PHS version file
  under `/root/`, `dpkg-query`, `npm list -g`), so a compromised host can report any
  version it likes and thereby fabricate or suppress an "update available" state for
  its own items.

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
