# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.1](https://github.com/worried-networking/uptrakit/releases/tag/uptrakit-shared-macros-v0.0.1) - 2026-04-21

### Added

- *(macros)* add wire_safe_enum! declarative macro
- Implement TOP3 least-effort fixes from codereview

### Fixed

- *(ci)* resolve all backend-lint, frontend, semantic-boundary, markdown, and edition CI failures
- frontend accessibility, security, and UX improvements with expanded tests
- resolve top 5 code review findings across 6 crates

### Other

- *(cargo)* add workspace lints and consolidate inline dependencies
- *(codereview)* resolve all remaining open code review findings
- *(codereview)* resolve top 5 open code review findings
- add code review results for shared crates
- remove CODEREVIEW.md files
- add extensibility-focused code review for all crates
- add code review reports for 7 shared crates
- add impl_report_conversion! macro and replace verbose ReportConversion impls
