# Provider Development Guidelines

When adding or changing a provider, document the full lifecycle:

- How the agent detects the installed version.
- How the controller resolves the latest upstream version.
- Version comparison rules (semver, tag prefixes, build metadata handling).
- Update execution steps, required privileges, and failure modes.
- Required configuration fields with examples.
- Any assumptions about the agent environment or custom scripts.

Providers should keep parsing and comparison logic in pure functions so they are easy to test.

The provider registry crate (`uptrakit-provider-registry`) centralizes config validation, mask/restore workflows, and creates provider instances based
on `ProviderType`. Document provider behavior so the registry can continue to validate configs and mask secrets correctly.
