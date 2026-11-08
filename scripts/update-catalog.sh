#!/bin/bash
# Updates catalog/, the copy of Logix Catalog (github.com/openlogix/logix-catalog)
# that release builds embed, to a commit of that repo: a branch, tag or commit
# (default: main). Never edit catalog/ by hand: change boards in logix-catalog,
# then run this. catalog/SOURCE.json records the commit and its date (the
# catalog's version). For trying a local catalog clone without updating
# catalog/, build with LOGIX_ENGINE_DEV_CATALOG=<clone> instead.
#
#   scripts/update-catalog.sh [<ref>]

set -euo pipefail
cd "$(dirname "$0")/.."
ref="${1:-main}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

git clone --quiet https://github.com/openlogix/logix-catalog.git "$tmp/catalog"
git -C "$tmp/catalog" checkout --quiet "$ref"
commit="$(git -C "$tmp/catalog" rev-parse --short=7 HEAD)"
date="$(TZ=UTC git -C "$tmp/catalog" log -1 --date=format-local:%Y-%m-%d --format=%cd)"

rm -rf catalog
mkdir -p catalog/schemas
cp -R "$tmp/catalog/boards" catalog/boards
cp "$tmp/catalog/fpgas.json" catalog/
cp "$tmp/catalog/schemas/logix-board-v1.json" catalog/schemas/
cp "$tmp/catalog/LICENSE-MIT" "$tmp/catalog/LICENSE-APACHE" catalog/
printf '{\n  "repo": "https://github.com/openlogix/logix-catalog",\n  "commit": "%s",\n  "date": "%s"\n}\n' "$commit" "$date" > catalog/SOURCE.json
echo "catalog/ is now logix-catalog $commit ($date): $(ls catalog/boards | wc -l | tr -d ' ') boards."
