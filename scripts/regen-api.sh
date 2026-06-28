#!/usr/bin/env bash
# Regenerate the OpenAPI spec (Rust) and the frontend client (TS) in one step.
set -euo pipefail
cd "$(dirname "$0")/.."
UPDATE_OPENAPI=1 cargo test -p uptrakit-web-api --all-features openapi_
( cd frontend && npm run gen:api )
echo "Regenerated crates/ui/web-api/openapi.json + frontend/src/lib/api/generated/"
