# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.1](https://github.com/worried-networking/uptrakit/releases/tag/uptrakit-mqtt-runtime-v0.0.1) - 2026-04-21

### Added

- port service providers and cli to surface runtime
- unify mqtt runtime for standalone and embedded hosting

### Fixed

- *(ci)* resolve all backend-lint, frontend, semantic-boundary, markdown, and edition CI failures
- revert try_publish to async publish to fix channel overflow
- *(mqtt-runtime)* prevent double-publish channel overflow on initial connect
- *(mqtt)* replace blocking publish/subscribe with non-blocking try_* variants
- *(mqtt-runtime)* complete surface config mutations asynchronously
- *(services)* make surface registration best effort
- address task 8 follow-up verification blockers
- release embedded mqtt claims on yield

### Other

- require labels for surface interactions
- *(surfaces)* remove extension-era runtime leftovers
- rename surface runtime capability internals
- isolate plugin boundaries in track a
