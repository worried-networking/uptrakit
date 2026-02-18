# Code Review: uptrakit-provider-registry

Extensibility-focused review of the provider registry crate.

## Role in the Architecture

This crate is the centralized dispatch layer for creating providers, validating configurations,
and managing secret masking/restoration. It is the only crate that depends on all concrete
provider implementations.

## Findings

### ~~Critical: hardcoded match arms for every operation~~ (FIXED)

**Resolution:** Replaced all four match blocks with a `register_providers!` declarative macro.
Adding a new provider now requires a single entry in the macro invocation. All provider
`new()` constructors now consistently return `Result<Self>` and validate their config.

### Significant: no dynamic provider registration mechanism

All providers are statically linked. There is no way for an external crate to register a provider
without modifying this crate's source code and `Cargo.toml`.

### Recommendation: implement a registration pattern

Several approaches, in order of increasing complexity:

1. **Macro-based registration** -- a declarative macro that generates all match arms from a list
   of `(ProviderType, ConfigType, ProviderType)` tuples, reducing the 4 match blocks to a single
   declaration.

2. **`ProviderFactory` trait with a registry map** -- each provider crate exports a factory, and
   the registry collects them into a `HashMap<ProviderType, Box<dyn ProviderFactory>>` at startup:

   ```rust
   pub trait ProviderFactory: Send + Sync {
       fn create(&self, config: &Value, executor: Arc<dyn CommandExecutor>) -> Result<Box<dyn Provider>>;
       fn validate(&self, config: &Value) -> Result<()>;
       fn mask_secrets(&self, config: &Value) -> Value;
       fn restore_secrets(&self, incoming: &mut Value, existing: &Value);
   }
   ```

3. **Inventory crate pattern** -- use the `inventory` crate (or `linkme`) for compile-time
   self-registration so provider crates register themselves without modifying the registry.

## Positive Observations

- **Generic `mask_secrets_for<T>` and `restore_secrets_for<T>` helpers** eliminate boilerplate
  within each match arm.
- **Comprehensive test coverage** -- every provider type has creation, validation, and secret
  masking/restoration tests.
- **`validate_config_str` convenience method** accepts string provider types, useful for HTTP API
  handlers that receive provider types as strings.
- Clean error types (`RegistryError`) with descriptive variants.
