#!/usr/bin/env bash
# Regenerate the AsyncAPI spec from the Rust wire types.
set -euo pipefail
cd "$(dirname "$0")/.."
UPDATE_ASYNCAPI=1 cargo test -p uptrakit-wire --all-features asyncapi_
echo "Regenerated crates/shared/wire/asyncapi.yaml"
echo "NOTE: if the message/schema set changed, re-validate the document with an"
echo "AsyncAPI validator (https://studio.asyncapi.com or @asyncapi/parser) — spec §Verification."
