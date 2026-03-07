#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_README="${ROOT_DIR}/README.md"
DST_README="${ROOT_DIR}/npm/tu/README.md"

REPO_BLOB_BASE="https://github.com/hanbu97/tokenusage/blob/main"
REPO_RAW_BASE="https://raw.githubusercontent.com/hanbu97/tokenusage/main"

tmp_file="$(mktemp)"
trap 'rm -f "${tmp_file}"' EXIT

cp "${SRC_README}" "${tmp_file}"

# npmjs renders best with absolute links.
# Convert local docs/assets references while keeping the document structure aligned
# with the root README.
sed -E \
  -e "s|href=\"docs/|href=\"${REPO_BLOB_BASE}/docs/|g" \
  -e "s|src=\"docs/|src=\"${REPO_RAW_BASE}/docs/|g" \
  -e "s|src=\"assets/|src=\"${REPO_RAW_BASE}/assets/|g" \
  -e "s|href=\"assets/|href=\"${REPO_BLOB_BASE}/assets/|g" \
  -e "s|\\]\\(docs/|](${REPO_BLOB_BASE}/docs/|g" \
  -e "s|\\]\\(\\./|](${REPO_BLOB_BASE}/|g" \
  "${tmp_file}" > "${DST_README}"

echo "Synced npm README from root README:"
echo "  ${SRC_README}"
echo "  -> ${DST_README}"
