#!/bin/bash
# Regenerates schemas/tools.json, the published copy of the tool schemas,
# and schemas/logix-boards-v1.json, the boards schema,
# from the code. Review the diff: once released, changes may only add tools
# or optional fields.

set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/_rust-env.sh

UPDATE_SCHEMAS=1 cargo test --quiet --test schemas
git --no-pager diff --stat -- schemas/
echo "If schemas/logix-boards-v1.json changed, copy it to the website: docs/schemas/ in openlogix.github.io."
