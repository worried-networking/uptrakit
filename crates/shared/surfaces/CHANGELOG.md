# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.1](https://github.com/worried-networking/uptrakit/releases/tag/uptrakit-surfaces-v0.0.1) - 2026-04-27

### Added

- *(surfaces)* add entity-link cell type, SurfaceEntityType, SurfaceEntityRef
- *(surfaces)* add #[non_exhaustive] to public surface enums + wildcard arms
- *(proxmox)* replace degraded boundary surface with full proxmox_hosts_surface
- *(surfaces)* add ContextSelector capability and SurfaceContextSelectorDescriptor
- port service providers and cli to surface runtime
- port representative plugins to native surfaces
- add shared surface contract crate

### Fixed

- *(proxmox,surfaces)* cap per_page at 200; omit empty required_for_interactions
- *(ci)* resolve all backend-lint, frontend, semantic-boundary, markdown, and edition CI failures
- unblock task 8 verification bundle
- validate surface tab identifiers
- enforce surface capability usage checks
- enforce surface registration invariants
- tighten shared surface contracts

### Other

- *(shared-surfaces)* split validation and harden contracts
- require labels for surface interactions
- Add software item tab slot to software detail route
- *(surfaces)* remove extension-era runtime leftovers
- Port proxmox host-detail surface and harden hydration behavior
