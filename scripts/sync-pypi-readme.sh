#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${ROOT_DIR}/pypi/tu/README.md"
RAW_BASE="https://raw.githubusercontent.com/FelixIsaac/tokenusage/main"

cp "${ROOT_DIR}/README.md" "${DEST}"

# PyPI does not render relative image paths — rewrite them to absolute
# GitHub raw URLs.  Handle both src="..." and href="..." for docs/images.
if [[ "$(uname)" == "Darwin" ]]; then
  sed -i '' \
    -e "s|src=\"docs/|src=\"${RAW_BASE}/docs/|g" \
    -e "s|href=\"docs/|href=\"${RAW_BASE}/docs/|g" \
    -e "s|src=\"assets/|src=\"${RAW_BASE}/assets/|g" \
    -e "s|href=\"assets/|href=\"${RAW_BASE}/assets/|g" \
    -e "s|href=\"\\./LICENSE\"|href=\"${RAW_BASE}/LICENSE\"|g" \
    -e "s|href=\"\\./README\\.zh-cn\\.md\"|href=\"${RAW_BASE}/README.zh-cn.md\"|g" \
    "${DEST}"
else
  sed -i \
    -e "s|src=\"docs/|src=\"${RAW_BASE}/docs/|g" \
    -e "s|href=\"docs/|href=\"${RAW_BASE}/docs/|g" \
    -e "s|src=\"assets/|src=\"${RAW_BASE}/assets/|g" \
    -e "s|href=\"assets/|href=\"${RAW_BASE}/assets/|g" \
    -e "s|href=\"\\./LICENSE\"|href=\"${RAW_BASE}/LICENSE\"|g" \
    -e "s|href=\"\\./README\\.zh-cn\\.md\"|href=\"${RAW_BASE}/README.zh-cn.md\"|g" \
    "${DEST}"
fi

echo "Synced pypi/tu/README.md from root README.md (with absolute image URLs)"
