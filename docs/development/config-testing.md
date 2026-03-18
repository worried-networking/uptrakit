# Config Testing Architecture

Config testing lets users validate a plugin configuration before deploying it to production.
A test executes real operations (version detection commands, syntax checks, HTTP requests) against
real targets but does not create database records or trigger actual updates. This document covers
the end-to-end architecture, supported test kinds, and how to extend the system.

For a quick reference on declaring config test support in a plugin, see the
[Config Test Capability](plugin-guidelines.md#config-test-capability) section in the plugin
guidelines.

## Overview

The config test flow starts at a REST API endpoint and ends with a structured result containing
success/failure status, output, optional detected version, and timing information. The flow
differs depending on whether the plugin runs on the controller or on an agent.

**Controller-side plugins** (those with `ControllerSideFetchReleases` capability) are validated
in-process on the controller. The endpoint returns immediately after config validation.

**Agent-side plugins** (all others) require the controller to proxy the test request to a
connected agent over the WebSocket wire protocol. The agent executes the test locally and sends
the result back.

## Architecture

### Request flow

```text
POST /api/v1/plugin-configs/test
         |
         v
  [Route handler: test_plugin_config]
         |
         |-- Validate plugin type is known
         |-- Merge with saved config (if plugin_config_id provided)
         |-- Validate merged config
         |-- Reject dangerous command patterns
         |
         +-- Controller-side?
         |     |
         |     Yes --> Return success (config validation is sufficient)
         |
         No (agent-side)
         |
         |-- Require host_id
         |-- Resolve host -> service (via service_host join)
         |-- Determine ConfigTestKind from request or default
         |-- Build TestPluginConfigPayload
         |
         v
  [ConfigTestProxy::invoke]
         |
         |-- Create oneshot channel
         |-- Send ControllerMessage::TestPluginConfig to agent
         |-- Await response (30 s timeout)
         |
         v
  [Agent: run_config_test]
         |
         |-- Match on ConfigTestKind
         |-- Create plugin instance via PluginRegistry
         |-- Execute test-kind-specific logic
         |-- Return TestPluginConfigResultPayload
         |
         v
  [ConfigTestProxy::complete]
         |
         v
  TestPluginConfigResponse -> HTTP 200
```

### Key components

| Component | Crate | Purpose |
| :--- | :--- | :--- |
| `test_plugin_config` route | `web-api` (`routes/plugin_configs.rs`) | REST handler; validates, merges config, dispatches |
| `ConfigTestProxy` | `web-api` (`config_test_proxy.rs`) | Request/response correlation over WebSocket |
| `ConfigTestOps` | `plugin-infrastructure-core` (`descriptor.rs`) | Per-plugin metadata: supported kinds and default kind |
| `ConfigTestKind` | `wire` (`payloads.rs`) | Enum of test kinds sent on the wire |
| `TestPluginConfigPayload` | `wire` (`payloads.rs`) | Controller-to-agent wire payload |
| `TestPluginConfigResultPayload` | `wire` (`payloads.rs`) | Agent-to-controller wire result |
| `run_config_test` | `agent-core` (`config_test.rs`) | Agent-side dispatch and execution |
| `TestPluginConfigRequest` | `web-api-types` (`plugin_config_test.rs`) | HTTP request body type |
| `TestPluginConfigResponse` | `web-api-types` (`plugin_config_test.rs`) | HTTP response body type |

### ConfigTestProxy pattern

`ConfigTestProxy` follows the same correlation pattern as `ExtensionProxy`:

1. The REST handler generates a UUID v7 `request_id` and creates a `oneshot::Sender` in a
   `parking_lot::Mutex<HashMap<String, Sender>>` pending map.
2. The proxy sends `ControllerMessage::TestPluginConfig` to the agent's WebSocket channel.
3. The REST handler awaits the `oneshot::Receiver` with a 30-second timeout.
4. When the agent responds with `ServiceMessage::TestPluginConfigResult`, the WebSocket handler
   calls `ConfigTestProxy::complete(request_id, result)` which resolves the oneshot.

If the agent disconnects before responding, the oneshot sender is dropped and the receiver
returns an error. If the timeout fires first, the pending entry is cleaned up.

## Supported test kinds

| `ConfigTestKind` | Role used | What it tests | Implemented |
| :--- | :--- | :--- | :--- |
| `VersionDetection` | `VersionDetector` | Creates a plugin instance, calls `detect_installed_version()` on the specified package identifier, and returns the detected version string. | Yes |
| `UpdateCommandValidation` | None (direct) | Extracts `update_command` from the config JSON and runs `sh -n -c "<command>"` to syntax-check it without executing. | Yes |
| `Connectivity` | `ReleaseFetcher` | Tests upstream API connectivity by performing a lightweight `fetch_releases()` call. Used for controller-side plugins. | Planned |
| `PreUpdateHook` | `LifecycleHook` | Executes the pre-update hook with a mock `UpdateLifecycleContext`. | Planned |
| `PostUpdateHook` | `LifecycleHook` | Executes the post-update hook with a mock `UpdateLifecycleContext`. | Planned |

The `ConfigTestKind` enum is `#[non_exhaustive]` and uses `#[serde(rename_all = "snake_case")]`
for wire serialization. The agent-side handler falls through to a wildcard arm for unrecognized
kinds, logging a warning and returning an error result without panicking.

### VersionDetection

The agent creates a plugin instance via `PluginRegistry::create_plugin`, extracts the
`VersionDetector` role with `plugin.as_version_detector()`, and calls
`detect_installed_version(package_identifier)`. Three outcomes:

- **Version detected** -- `success: true`, `output: "detected version: 1.24.0"`,
  `detected_version: "1.24.0"`.
- **No version found** -- `success: true`, `output: "no version detected (package may not be
  installed)"`, `detected_version: null`.
- **Error** -- `success: false`, `error: "version detection failed: ..."`.

### UpdateCommandValidation

Extracts `update_command` from the config JSON and validates syntax:

```sh
sh -n -c "<update_command>"
```

The `-n` flag causes the shell to parse but not execute the command. This catches syntax errors
(unmatched quotes, invalid redirections) without side effects. Returns an error if the config
has no `update_command` field or if the field is empty.

## How plugins declare support

Plugins declare config test support in the `declare_plugin!` macro:

```rust
declare_plugin! {
    id: PluginTypeId::from_static("generic_shell"),
    name: "Generic Shell",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    config: ShellConfig,
    plugin: ShellPlugin,
    host_requirements: HostRequirements::POSIX,
    config_test: [VersionDetection, UpdateCommandValidation],
}
```

The `config_test:` field accepts a list of `ConfigTestKind` variants:

- The **first** kind in the list becomes the `default_kind` (used when the API caller does not
  specify `test_kind`).
- The macro generates a `ConfigTestOps` struct on the plugin's `PluginDescriptor` with
  `supported_kinds` and `default_kind` fields.
- The macro automatically adds `PluginCapability::ConfigTest` to the plugin's capability list.

The generated `ConfigTestOps` is stored in `PluginDescriptor.config_test` as
`Option<&'static ConfigTestOps>`. Plugins that omit the `config_test:` field have `None`.

## API reference

### Request

```text
POST /api/v1/plugin-configs/test
```

**Permission:** `test_plugin_configs`

**Request body (`TestPluginConfigRequest`):**

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `plugin_type` | `string` | Yes | Plugin type identifier (e.g. `"generic_shell"`). |
| `config` | `object` | Yes | Plugin configuration JSON to test. |
| `plugin_config_id` | `uuid` | No | Saved config ID; incoming `config` is shallow-merged on top. |
| `host_id` | `uuid` | No | Target host for agent-side tests (required for non-controller-side plugins). |
| `test_kind` | `string` | No | One of: `version_detection`, `update_command_validation`, `connectivity`, `pre_update_hook`, `post_update_hook`. Defaults to `version_detection` for agent-side plugins. |
| `package_identifier` | `string` | No | Package identifier for testing (e.g. `"nginx"`, `"owner/repo"`). |

### Response

**Success (`TestPluginConfigResponse`):**

| Field | Type | Description |
| :--- | :--- | :--- |
| `success` | `bool` | Whether the test passed. |
| `test_kind` | `string` | The kind of test that was executed. |
| `output` | `string?` | Command output or status message. |
| `error` | `string?` | Error message if the test failed. |
| `detected_version` | `string?` | Detected version (for `version_detection` tests). |
| `duration_ms` | `u64` | Test duration in milliseconds. |

**Error responses:**

| Status | Condition |
| :--- | :--- |
| 400 | Unknown plugin type, invalid config, missing `host_id`, unknown `test_kind`, dangerous command pattern |
| 404 | Host or plugin config not found |
| 502 | Agent disconnected during test or send failure |
| 504 | Agent did not respond within 30 seconds |

## Adding a new test kind

1. **Add the variant to `ConfigTestKind`** in `crates/shared/wire/src/payloads.rs`. The enum is
   `#[non_exhaustive]` so this is a non-breaking wire change. Use `snake_case` naming to match
   the serde rename convention.

2. **Add handling in the agent** in `crates/shared/agent-core/src/config_test.rs`. Add a new
   match arm in `handle_config_test` for the new kind. Follow the pattern of existing handlers:
   measure elapsed time, capture all errors into the result payload, never panic.

3. **Add the kind to the route handler's match** in
   `crates/ui/web-api/src/routes/plugin_configs.rs`. Map the `test_kind` string to the new
   `ConfigTestKind` variant in the `test_plugin_config` function.

4. **Declare support in relevant plugins** by adding the new kind to `config_test: [...]` in
   each plugin's `declare_plugin!` invocation.

5. **Add tests** in `config_test.rs` for the new handler, covering success, failure, and edge
   cases. Follow the existing test patterns using `NoopCommandExecutor`.

## Security considerations

Config tests execute real operations against real targets:

- **VersionDetection** runs shell commands on the agent host via the configured command executor.
- **UpdateCommandValidation** invokes `sh -n` on the agent, which parses the command without
  executing it.
- **Connectivity** (when implemented) will make HTTP requests to external APIs.

Mitigations in place:

- **Authorization** -- the endpoint requires the `test_plugin_configs` permission.
- **Dangerous command rejection** -- the route handler checks for dangerous patterns in
  command fields before dispatching (controlled by `reject_dangerous_commands` config flag).
- **Config validation** -- the merged config is validated against the plugin's schema before
  the test is dispatched.
- **Timeout** -- agent-side tests have a 30-second timeout. If the agent does not respond, the
  pending request is cleaned up and a 504 is returned.
- **Session-targeted delivery** -- `TestPluginConfig` messages are never published to NATS.
  They are sent exclusively over the authenticated WebSocket connection to the target agent.
- **No side effects** -- tests do not create database records, trigger updates, or modify
  host state. `UpdateCommandValidation` explicitly uses `sh -n` (no-execute mode).

## Related documentation

- [Plugin Development Guidelines](plugin-guidelines.md) -- macro syntax, capabilities, roles
- [Coding Standards](coding-standards.md) -- `#[non_exhaustive]`, error handling patterns
- [Error Handling](error-handling.md) -- `rootcause::Report` conventions
