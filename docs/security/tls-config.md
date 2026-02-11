# CLI TLS options

The CLI client (`crates/ui/cli/src/client.rs`) uses the system trust store by default for TLS verification. The previous
hardcoded `tls_danger_accept_invalid_certs(true)` has been removed. An explicit `--insecure` flag is now required to
skip TLS certificate verification:

| Flag | Default | Description |
| --- | --- | --- |
| `--insecure` | `false` | Skip TLS certificate verification (self-signed certs). Use only for development or initial setup. |
