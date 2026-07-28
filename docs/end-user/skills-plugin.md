---
title: Agent Skills Plugin
weight: 185
description: Distribute and manage packaged agent skills via the skills plugin.
---

# Agent Skills Plugin

The **Agent Skills** plugin discovers, tracks, and updates LLM-agent Skills installed globally
on a host via `npx skills@<version>` (default: `latest`).

## What gets discovered

The plugin reads `~/.agents/.skill-lock.json` on each managed host. Every Skill entry with
`sourceType == "github"` becomes a **Software Item** in Uptrakit. The Skill name (e.g.
`brainstorming`) is the display identifier; the installed version is the git tree SHA recorded
as `skillFolderHash`.

## GitHub Provider and rate limits

Release detection (finding whether a newer version of a Skill exists) calls the GitHub git-trees
API through the instance-wide **GitHub Provider** configured in **Settings → GitHub Provider**.

- **Without a token:** 60 unauthenticated requests per hour. Adequate for small deployments
  with skills concentrated in one or two repositories.
- **With a token:** 5,000 requests per hour. Recommended for larger or multi-repo deployments.

The plugin issues **one API call per source repository per refresh cycle** — not one per Skill —
so Skills from the same repo share a single request.

## Update semantics

Updates run `DISABLE_TELEMETRY=1 npx skills@<version> update -g <skill-name> -y` on the agent,
where `<version>` is the tenant-configured **Skills Package Version** (default: `latest`). The
`skills` CLI does not support version pinning; it always moves the Skill to the current HEAD tree
SHA. Uptrakit records the requested `to_version` for audit purposes but does not pass it to the
CLI. The detection cycle reconciles `installed_version` after the update lands.

## Skills Package Version

The plugin config exposes a **Skills Package Version** field (default: `latest`). Set it to a
specific npm dist-tag or semver version (e.g. `1.2.3`, `next`) if you want reproducible update
behaviour across hosts. Configure it per-tenant in the plugin config UI.

## GitHub-only source restriction

Only Skills with `sourceType == "github"` are tracked. Skills from other sources (GitLab, local
paths) are logged as warnings and skipped. This is a known v1 limitation.

## Standalone scheduler

If you run the standalone scheduler (without the embedded controller), the release-fetch path is
unavailable — the scheduler has no access to the GitHub Provider. Release fetch calls return an
error logged at `warn`. Discovery and version detection continue to work.

## Known limitations

- **Force-push false positives:** A force-push to the source repo that rewrites history without
  changing file content still changes the git tree SHA, producing a perpetual "update available"
  signal. Running the update is idempotent — `npx skills update` re-installs HEAD and records
  the new SHA.
- **Skill folder renamed upstream:** If a Skill folder moves in the source repo, release fetch
  returns zero releases. The stored identifier becomes stale; reinstall via the `skills` CLI
  to reconcile.
