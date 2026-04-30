#!/usr/bin/env bash
# Wrapper around real cargo. release-plz's git_only mode runs
# `cargo package --allow-dirty --workspace` per analyzed package in a /tmp
# worktree to extract metadata from the resulting .crate tarball. The
# default verification step recompiles the entire workspace per worktree
# (~10 GB each) and 13 worktrees coexist on disk, blowing the runner.
# Inject --no-verify so cargo only emits the .crate tarball without
# building. release-plz only reads metadata from the tarball, never the
# build artifacts (release_plz_core::next_ver::get_cargo_package).
set -euo pipefail

self_dir=$(cd "$(dirname "$0")" && pwd)
cleaned_path=$(echo "$PATH" | tr ':' '\n' | grep -vF "$self_dir" | paste -sd: -)
real_cargo=$(PATH="$cleaned_path" command -v cargo)

# Recursion guard: cargo sets $CARGO=argv[0] when spawning build scripts /
# proc-macros, so child invocations could re-enter the wrapper through
# $CARGO. If we are already inside a wrapped exec, forward to real cargo
# without re-parsing.
if [ -n "${UPTRAKIT_CARGO_WRAPPER_ACTIVE:-}" ]; then
  unset CARGO
  exec "$real_cargo" "$@"
fi
export UPTRAKIT_CARGO_WRAPPER_ACTIVE=1
unset CARGO

saw_package=false
saw_workspace=false
already_no_verify=false
for arg in "$@"; do
  case "$arg" in
    package)      saw_package=true ;;
    --workspace)  saw_workspace=true ;;
    --no-verify)  already_no_verify=true ;;
  esac
done

if $saw_package && $saw_workspace && ! $already_no_verify; then
  exec "$real_cargo" "$@" --no-verify
else
  exec "$real_cargo" "$@"
fi
