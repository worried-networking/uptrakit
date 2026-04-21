# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.1](https://github.com/worried-networking/uptrakit/releases/tag/uptrakit-agent-ssh-runtime-v0.0.1) - 2026-04-21

### Added

- *(audit)* emit semantic mutation audit events
- port service providers and cli to surface runtime
- *(updates)* make restart recovery owner-aware

### Fixed

- *(ci)* resolve all backend-lint, frontend, semantic-boundary, markdown, and edition CI failures
- *(audit)* type runtime audit emitter actions
- *(audit)* align action taxonomy with the semantic audit contract
- *(agent-ssh-runtime)* restore UpdateHooks dropped in surface rename
- *(agent-ssh-runtime)* restore UpdateHooks capability dropped during extraction
- preserve embedded ssh yield parity

### Other

- rename surface runtime capability internals
- isolate plugin boundaries in track a
- extract ssh agent runtime
