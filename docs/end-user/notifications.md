# Notifications

The notification subsystem lets you receive alerts when important events occur on your
Uptrakit controller. You configure **channels** (where messages go) and **rules** (which events
trigger messages). A delivery **log** tracks the history of every notification sent.

## Concepts

### Channels

A channel is a delivery endpoint. Each channel has a type and a type-specific configuration.
Uptrakit currently supports three channel types:

| Type | Description |
| --- | --- |
| `webhook` | Sends a JSON POST request to the configured URL. Optionally signs the payload with HMAC-SHA256. |
| `telegram` | Sends a message to a Telegram chat via the Bot API. Supports inline keyboard buttons for actionable notifications. |
| `email` | Sends email notifications via SMTP. Global SMTP settings are shared across all email channels; each channel configures only recipient addresses. |

Channels can be enabled or disabled independently. A disabled channel suppresses all deliveries
without deleting the channel or its rules.

### Rules

A rule binds an **event type** to a **channel**. When an event fires and a matching enabled rule
exists, the controller delivers a notification through the associated channel. Rules can optionally
include **scope filters** to narrow the set of events they match (see [Scoping Rules](#scoping-rules)).

### Event types

| Event type | Fires when |
| --- | --- |
| `update_available` | A new upstream version is detected for a software item. |
| `update_completed` | An update finishes successfully. |
| `update_failed` | An update fails. |
| `new_software_discovered` | Autodiscovery finds a new software package. |
| `new_service_enrolled` | A new service (agent, MQTT, SSH agent) registers with the controller. |
| `ca_rotated` | The controller's internal CA is rotated. |

### Actionable notifications

Some event types produce **actionable** notifications. For `update_available` events, the
notification can include an action button. On Telegram, this appears as an inline keyboard button
labelled with the available version (for example, "Install 2.1.0"). Pressing the button triggers
the update on the controller without requiring you to open the CLI or web UI.

For webhook channels, action metadata (label, callback URL, and token) is included in the JSON
payload so your automation tooling can act on it.

## Web UI

The Settings page in the web UI provides built-in tabs for managing notification channels,
rules, and viewing the delivery log. Channel management tabs appear automatically based on
which notification plugins are enabled.

### Notification Channels tab

All enabled notification plugins share a single **Notification Channels** tab on the Settings page,
with each channel type rendered as a section:

- **Webhook Channels** — create, edit, test, and delete webhook endpoints
- **Telegram Channels** — manage Telegram bot channels
- **Email Channels** — manage email recipient lists and configure per-tenant SMTP overrides

These sections use the extension framework and render dynamically. Channel creation and editing
open form modals with fields appropriate to the channel type. Sensitive fields (bot tokens,
webhook secrets, SMTP passwords) are masked with `***` and preserved on edit unless explicitly
changed.

### SMTP configuration

SMTP settings use a two-layer architecture:

- **Global SMTP Defaults** — configured on the **Global Settings** page in the "SMTP Defaults"
  section. These are server-wide defaults shared across all tenants.
- **Per-tenant SMTP overrides** — the Email Channels section includes a **Configure SMTP** button
  that opens a form pre-populated with the current per-tenant SMTP settings. Non-empty fields
  override the global defaults. Changes are saved with patch semantics: only fields you modify
  are updated, and empty fields inherit from global defaults.

The Global SMTP Defaults form includes an optional **EHLO Hostname** field. The SMTP `EHLO`
command requires a valid fully-qualified domain name. If left empty, the domain part of the
**From Address** is used automatically (e.g. `from_address = noreply@example.com` → EHLO
hostname `example.com`). Set this field when using a relay server whose required EHLO hostname
differs from your sender domain — for example when Gmail or another provider rejects with
`555 5.5.2 Syntax error`. This is a global setting and cannot be overridden per-tenant.

**Send Test Email** sends a test message to the **calling user's own profile email address**. No
recipient address input is needed.

### Notification Rules tab

The **Notification Rules** tab lets you create rules that bind event types to channels.
Each rule specifies a channel and an event type, with optional scope filters for host ID,
software item ID, or plugin type. Rules can be enabled or disabled independently.

### Notification Log tab

The **Notification Log** tab displays a read-only table of all delivery attempts with their
status (delivered, failed, or pending), timestamps, and error messages for failed deliveries.

### Permissions

The notification tabs are visible only to users with the `view_notifications` permission.
Creating, editing, and deleting channels and rules requires `manage_notifications`.

## Setting Up a Webhook Channel

Webhook channels POST a JSON payload to a URL you control. The payload includes the event title,
body, event metadata, and any action references. When a `secret` is configured, the request body is
signed with HMAC-SHA256 and the signature is sent in the `X-Uptrakit-Signature` header as
`sha256=<hex>`.

### Step 1: Create the channel

```sh
uptrakit notifications channels create --name "My Webhook" --type webhook \
  --config '{"url":"https://example.com/hooks/uptrakit","secret":"my-secret"}'
```

Webhook configuration fields:

| Field | Required | Description |
| --- | --- | --- |
| `url` | Yes | The HTTP or HTTPS endpoint to POST notifications to. |
| `secret` | No | HMAC-SHA256 signing key. When present, every request includes an `X-Uptrakit-Signature` header. |
| `headers` | No | Object of custom HTTP headers to include with every request (e.g. `{"Authorization":"Bearer ..."}`). |

### Step 2: Create a rule

```sh
uptrakit notifications rules create --channel-id <CHANNEL_ID> --event-type update_available
```

### Step 3: Test the channel

```sh
uptrakit notifications channels test <CHANNEL_ID>
```

The test command sends a synthetic notification through the channel and reports success or failure.

### Step 4: Verify delivery

```sh
uptrakit notifications log
```

## Setting Up a Telegram Channel

Telegram channels send messages to a chat or group via the Telegram Bot API. Actionable events
render inline keyboard buttons directly in the chat.

### Step 1: Create a Telegram bot

1. Open a conversation with [@BotFather](https://t.me/BotFather) on Telegram.
2. Send `/newbot` and follow the prompts to create a bot.
3. Copy the **bot token** (e.g. `123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11`).

### Step 2: Get the chat ID

Add the bot to the target group or channel. Send a message in the group, then call the
Telegram `getUpdates` API to find the `chat.id`:

```sh
curl https://api.telegram.org/bot<BOT_TOKEN>/getUpdates
```

The chat ID for groups is a negative number (e.g. `-100123456789`).

### Step 3: Configure the Telegram webhook

Set up a Telegram webhook so that button presses are forwarded to your controller. The callback
endpoint is:

```text
POST /api/v1/notifications/callback/telegram/{channel_id}
```

This is a **public** endpoint. It does not require JWT authentication. Instead, Telegram includes
the `X-Telegram-Bot-Api-Secret-Token` header, which the controller verifies against the channel's
`webhook_secret` config field.

Register the webhook with Telegram after creating the channel (you need the channel ID first):

```sh
curl -X POST "https://api.telegram.org/bot<BOT_TOKEN>/setWebhook" \
  -d "url=https://your-controller.example.com/api/v1/notifications/callback/telegram/<CHANNEL_ID>" \
  -d "secret_token=my-secret"
```

### Step 4: Create the channel

```sh
uptrakit notifications channels create --name "Ops Telegram" --type telegram \
  --config '{"bot_token":"123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11","chat_id":"-100123456789","webhook_secret":"my-secret"}'
```

Telegram configuration fields:

| Field | Required | Description |
| --- | --- | --- |
| `bot_token` | Yes | The bot token from BotFather. Stored encrypted at rest. |
| `chat_id` | Yes | The Telegram chat, group, or channel ID to send messages to. |
| `webhook_secret` | No | Secret token used to verify incoming Telegram callbacks. Must match the `secret_token` parameter passed to `setWebhook`. |

### Step 5: Create a rule and test

```sh
uptrakit notifications rules create --channel-id <CHANNEL_ID> --event-type update_available
uptrakit notifications channels test <CHANNEL_ID>
```

## Scoping Rules

By default, a rule with no scope filters matches **all events** of its event type across the
entire tenant. You can narrow the scope using one or more filters when creating or updating a rule.

| Filter | Effect |
| --- | --- |
| `--host-id <HOST_ID>` | Only match events for that specific host. |
| `--software-item-id <ITEM_ID>` | Only match events for that specific software item. |
| `--plugin-type <TYPE>` | Only match events from that plugin type (e.g. `releases_github`, `package_manager_apt`). |

Filters can be combined for narrow targeting. For example, to receive Telegram alerts only when
updates are available for a specific software item on a specific host:

```sh
uptrakit notifications rules create \
  --channel-id <CHANNEL_ID> \
  --event-type update_available \
  --host-id <HOST_ID> \
  --software-item-id <ITEM_ID>
```

A rule with no scope filters and a rule with filters can coexist on the same channel. Both fire
when their conditions are met, so you can set up a broad "catch-all" rule alongside targeted rules
on different channels.

## Managing Channels and Rules

### Channel commands

```sh
# List all notification channels (paginated)
uptrakit notifications channels list
uptrakit notifications channels list --page 2 --per-page 10

# Show channel details
uptrakit notifications channels get <CHANNEL_ID>

# Update a channel (name, config, or enabled state)
uptrakit notifications channels update <CHANNEL_ID> --name "New Name" --enabled false

# Delete a channel
uptrakit notifications channels delete <CHANNEL_ID>

# Send a test notification through a channel
uptrakit notifications channels test <CHANNEL_ID>
```

### Rule commands

```sh
# List all notification rules (paginated, with optional filters)
uptrakit notifications rules list
uptrakit notifications rules list --channel-id <CHANNEL_ID> --event-type update_available

# Show rule details
uptrakit notifications rules get <RULE_ID>

# Update a rule (event type, enabled state)
uptrakit notifications rules update <RULE_ID> --enabled false

# Delete a rule
uptrakit notifications rules delete <RULE_ID>
```

## Actionable Notifications

When a notification carries actions (currently `update_available` events), the behavior depends on
the channel type:

**Telegram:** The message includes an inline keyboard button labelled "Install \<version\>".
Pressing the button sends a callback to the controller's Telegram callback endpoint, which
triggers the update. Each button is single-use; pressing it again has no effect.

**Webhook:** The JSON payload includes an `actions` array with each action's `label`,
`callback_url`, and `token`. Your receiving service can POST to the callback URL with the token
to trigger the action.

## Viewing Delivery History

The notification log records every delivery attempt with its status (`pending`, `delivered`, or
`failed`), the event type, and timestamps.

```sh
# List recent delivery log entries (paginated)
uptrakit notifications log
uptrakit notifications log --page 1 --per-page 20
```

Failed deliveries include an `error_message` field with the failure reason. Use the JSON output
format for programmatic inspection:

```sh
uptrakit notifications log -o json | jq '.items[] | select(.status == "failed")'
```

## Permissions

Notification operations require the following permissions:

| Permission | Grants access to |
| --- | --- |
| `view_notifications` | List and view channels, rules, and the delivery log. |
| `manage_notifications` | Create, update, delete, and test channels and rules. |

Users without `view_notifications` cannot see any notification resources. Users with
`view_notifications` but without `manage_notifications` can inspect the configuration and
delivery log but cannot make changes.

## Related Documentation

- [Notifications API](../api/notifications.md) -- REST endpoint reference for all notification
  operations.
- [CLI Usage Guide](cli-usage.md) -- full CLI command reference including notification subcommands.
- [Auth and Authorization](../security/auth-and-authorization.md) -- permissions model, roles, and
  access control.
