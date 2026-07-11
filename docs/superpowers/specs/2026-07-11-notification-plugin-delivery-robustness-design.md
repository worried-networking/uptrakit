# Notification Plugin Delivery Robustness — Design

**Date:** 2026-07-11
**Status:** Draft
**Source:** `.superpowers/audit-2026-07-11.md` — HIGH "Telegram bot token leaks into logs and
notification_log.error_message on delivery failure" + MEDIUM "Email delivery aborts on first failed recipient —
partial delivery reported as total failure, retries duplicate" + MEDIUM "Webhook/Telegram/PHS hand-roll reqwest
clients bypassing the shared builder; Telegram has already drifted". Three notification/plugin HTTP+delivery
robustness gaps — one spec.

## Problem

1. **Telegram bot-token leak (HIGH, secret-in-logs).** The Telegram `sendMessage` URL embeds the bot token
   (`https://api.telegram.org/bot{bot_token}/sendMessage`, telegram/plugin.rs:134). Transport failures (DNS,
   connect-refused, timeout) are stringified via
   `map_err(|e| report!(NotificationPluginError::HttpRequest(e.to_string())))` (:142) — and reqwest 0.13.4's
   `Display for Error` appends `for url ({url})` whenever the error carries a URL. That string is persisted to
   `notification_log.error_message` (controller-core notification.rs:878), returned by the notifications API,
   and `tracing::warn!`'d — so **any network hiccup writes the plaintext bot token to the DB and logs**,
   readable by anyone with `view_notifications`. The webhook plugin (plugin.rs:153) has the same pattern (lower
   impact — the URL is the user's own, but may carry basic-auth userinfo).
2. **Email partial-delivery = total-failure (MEDIUM).** `EmailPlugin::deliver` loops `to_addresses` and does
   `send_email(&cfg, email).await?` per recipient. If recipient 2 of 5 fails, recipients 3-5 are never
   attempted and the whole channel delivery is marked failed even though recipient 1 got the mail. Any re-send
   (test action, future retry) **duplicates** delivery to the already-succeeded recipients. Each recipient also
   opens a fresh SMTP connection (connect+auth per call).
3. **Hand-rolled reqwest clients bypass the shared builder (MEDIUM, drift).**
   `plugin-infrastructure-core::build_plugin_http_client` exists so timeout/TLS/SSRF/**redirect** requirements
   "cannot drift per-plugin". Three plugins still hand-build: webhook (plugin.rs:76-83), telegram
   (plugin.rs:41-47), proxmox-helper-scripts (plugin.rs:114-125). All three set connect/timeout/SSRF, but
   telegram **omits `.redirect(Policy::none())`** (reqwest defaults to following up to 10 redirects — an SSRF/
   token-exfil vector for a token-bearing URL) and omits a User-Agent — the exact drift the builder prevents.

## Approach

### 1. Strip the URL from stringified transport errors (Telegram + webhook)

At telegram/plugin.rs:142 and webhook/plugin.rs:153, convert the error via **`e.without_url().to_string()`**
(reqwest 0.13's `Error::without_url()`, verified in the pin, removes the URL the `Display` impl would otherwise
append). This is the minimal, complete fix — the token (and any basic-auth userinfo) never enters the string
that gets persisted/logged. Prefer this over a fixed message string so the error still names the failure kind
(timeout vs connect vs DNS) for operators. **Coupled with fix 3 (contrarian):** `without_url()` nulls only the
*outer* error's URL, not a URL nested in a redirect error's `source`. That's inert on the Display path today
(reqwest's `Display` never prints `source`), and for Telegram fix 3's `redirect(Policy::none())` removes the
redirect variant entirely — so land (1) and (3) **together**; the token-free guarantee for Telegram rests on
both. The regression test asserts on the **Debug** format too (`format!("{err:?}")`), not just Display, so a
future `{:?}`/source-traversal render can't silently resurface the token. **This is a new pattern** — no `without_url`/redaction helper for
reqwest errors exists in the codebase today; the prevailing idiom is `impl_report_conversion!(reqwest::Error =>
…, |e| …e.to_string())` which passes the raw string through at ~8 plugin sites (proxmox/docker/gitlab/etc.).
Those other sites are the **same latent class** but out of scope here (only Telegram embeds a secret in the URL
*path*; the others' URLs are user-owned) — worth a grep + follow-up, not this fix. Confirm during implementation
that `without_url()` covers the full `Display` (including any source-chain that could re-surface the URL); the
`notification_log.error_message` column itself is a plain `String` and stays so — a URL-stripped transport-kind
message is not a credential, so no `SecretString` column change is warranted, the strip *is* the compliance
path.

### 2. Email: attempt every recipient, aggregate, reuse the connection

**Headline (stated honestly, contrarian-corrected):** the real, in-scope win is **attempt-all-recipients** +
**connection reuse**. The "stops duplicating on retry" benefit is *not* delivered here and the claim is dropped —
see the contract note below.

Rework `EmailPlugin::deliver` to:

- Iterate all `to_addresses`, attempting each; collect `(recipient, error)` for failures instead of `?`-bailing
  on the first. Today a failure at recipient 2 of 5 skips 3-5; after this, all five are attempted.
- **Contract (must be explicit — the controller caller is binary):** `notification.rs` (843-893) maps
  `Ok(())` → `delivered`, `Err` → `failed`, with **no retry** and no third state. So choose **partial =
  failed-with-list**: return `Err(DeliveryFailed(failed_recipients))` on *any* recipient failure (never mark a
  partially-failed send as `delivered` — "partial = success" would silently drop the failed recipients forever,
  strictly worse than today). `NotificationPluginError::DeliveryFailed` is a single opaque `String`
  (`#[non_exhaustive]`, notifications/core/src/error.rs); extend it minimally with a `Vec<String>` of failed
  recipients. (`BatchActionResponse{succeeded, failed}` considered and rejected — UUID-keyed, poor fit for
  addresses.)
- **The failed list is write-only for now** (it renders into the `error_message` text column; no consumer
  parses it back into a retry filter — that consumer is explicitly out of scope). Do **not** claim this reduces
  retry duplication: with attempt-all, *more* recipients succeed on pass 1, so a manual re-send (which resends
  the full `to_addresses`) duplicates *more*, not less, until a consumer parses the list. The list is returned
  now so a future retry/UI *can* consume it; storing it structured (vs text) is deferred with that consumer.
- Reuse a single SMTP connection for the batch. The email plugin uses **`mail-send`/`mail-builder`** (Stalwart,
  not `lettre` — no `lettre`/`AsyncSmtpTransport` anywhere in the workspace), and `send_email`
  (email/plugin.rs:254) builds a fresh `SmtpClientBuilder` and `.connect()`s **per call** today; refactor to
  build+connect the `SmtpClient` **once** and call `.send()` N times against `mail-send`'s actual API — cuts
  connect/auth overhead and failure surface. If that refactor proves large, connection reuse is the
  lower-priority half; the correctness fix (attempt-all + aggregate) is the required half.
- **Idempotency note:** "retry only the failed recipients" requires the retry caller to *have* the failed list.
  If no retry machinery consumes it yet (the audit says "future retry logic"), the immediate win is still real:
  the test-resend action and any future retry stop duplicating. Don't build retry orchestration here (YAGNI) —
  just return the structured failure so it *can* be consumed.

### 3. Route the three plugins through `build_plugin_http_client`

Convert webhook, telegram, and proxmox-helper-scripts to
`build_plugin_http_client(PluginHttpClientConfig { … })` (exact type name — grep-verified) — webhook with
`SsrfMode::Permissive` when `allow_private_urls`, the others `Strict` — getting redirect-none, User-Agent,
timeouts, TLS, and SSRF from one place so they can't drift again.

**No relocation, no architecture decision (review corrected the original framing):** the "reachability" concern
was a false alarm. Telegram, webhook, and email **already depend on `uptrakit-plugin-infrastructure-core`**
today (`features = ["plugin-ops"]`, for the `NotificationTransport` trait); `build_plugin_http_client` just
needs the crate's existing `http-client` feature enabled
(`http-client = ["dep:reqwest", "uptrakit-shared-types/http-ssrf"]`) on that existing dependency edge — a
one-line feature addition in three manifests, **not** a new crate dependency. `ci/check_plugin_semantic_boundary.py`
passes clean with this edge (it polices deps *on concrete leaf `uptrakit-plugin-*` crates*, not on the shared
infra-core base). proxmox-helper-scripts reaches the builder trivially (same family). Nothing moves to
shared-types.
Note: **webhook already sets `.redirect(Policy::none())`** (plugin.rs:77) — for webhook this conversion is pure
DRY/future-proofing, not a fix; only telegram is missing redirect-none + UA. proxmox-helper-scripts already
sets UA + redirect-none + SSRF by hand — also a DRY conversion.
Impl note: once the hand-rolled client is deleted, **drop the now-unused direct `reqwest = { workspace = true }`
line** from telegram/webhook manifests (reqwest arrives transitively via infra-core's `http-client` feature) —
else the unused-dependency CI gate (`cargo machete`) flags it. Feature-enabling is inert on the controller
build: reqwest+`http-ssrf` is already compiled in via the plugin registry's `catalog` feature.

## Tests

Plugin unit tests (each notification plugin crate has a test module):

1. **Token-leak regression (the HIGH):** construct a Telegram transport error carrying the token-bearing URL,
   run it through the delivery error path, assert **both** `format!("{err}")` and `format!("{err:?}")` contain
   neither the bot token nor `api.telegram.org/bot…` (Debug too — guards the source-chain case). Same for
   webhook basic-auth userinfo.
2. **Email partial delivery:** stub the SMTP send so recipient 2 of 3 fails; assert recipients 1 and 3 are both
   attempted, the returned failure names only recipient 2, and recipient 1/3 are reported delivered (a re-send
   would target only recipient 2).
3. **Shared-builder conformance:** assert each converted client has redirect-none and a User-Agent (or, if the
   builder is hard to introspect post-build, a construction-time test that `build_plugin_http_client` is the
   call path — the redirect/UA guarantee then comes from the builder's own tests).
4. No time-API tests; no `start_paused` (these are HTTP/SMTP-stub tests, not timer tests — snapshot rule).

## Documentation deliverables

- No `build_plugin_http_client` relocation (see fix 3) → no path/doc move; the `crates/plugins/AGENTS.md`
  "clients must use `build_plugin_http_client`" rule already exists and this change makes three more plugins
  conform to it.
- `NotificationPluginError::DeliveryFailed` doc **and** the `NotificationTransport::deliver` rustdoc in
  `plugin-infrastructure-core/src/roles.rs` (partial-success semantics touch the shared trait's contract, which
  other notification plugins implement — document the failed-recipient shape there, not only on the error type).
- No API/wire/OpenAPI change (the notifications API already returns `error_message`; its *contents* stop leaking
  secrets — a fix, not a surface change). No new ADR.

## Out of scope / deferred

- Retry-orchestration machinery that consumes the per-recipient failure list (the fix makes the list available;
  building the retry loop is separate — "future retry logic" per the audit).
- Auditing non-notification plugins' reqwest error stringification for the same URL-leak class beyond the named
  sites (worth a grep; fixing others is a new finding).
- The proxmox `verify_ssl`/`verify_tls` key-drift MEDIUM (separate finding, different fix).
- Migrating SMTP away from per-call connect if `send_email`'s refactor proves large (connection reuse is the
  lower-priority half of fix 2).
