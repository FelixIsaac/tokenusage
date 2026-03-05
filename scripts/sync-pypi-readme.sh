#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${ROOT_DIR}/pypi/tu/README.md"
RAW_BASE="https://raw.githubusercontent.com/hanbu97/tokenusage/main"

cp "${ROOT_DIR}/README.md" "${DEST}"

# PyPI does not render relative image paths — rewrite them to absolute
# GitHub raw URLs.  Handle both src="..." and href="..." for docs/images.
if [[ "$(uname)" == "Darwin" ]]; then
  sed -i '' \
    -e "s|src=\"docs/|src=\"${RAW_BASE}/docs/|g" \
    -e "s|href=\"docs/|href=\"${RAW_BASE}/docs/|g" \
    "${DEST}"
else
  sed -i \
    -e "s|src=\"docs/|src=\"${RAW_BASE}/docs/|g" \
    -e "s|href=\"docs/|href=\"${RAW_BASE}/docs/|g" \
    "${DEST}"
fi

echo "Synced pypi/tu/README.md from root README.md (with absolute image URLs)"
