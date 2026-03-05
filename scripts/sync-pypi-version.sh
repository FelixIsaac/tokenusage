#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TOML="${ROOT_DIR}/Cargo.toml"
INIT_PY="${ROOT_DIR}/pypi/tu/tokenusage/__init__.py"

cargo_version="$(sed -nE 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "${CARGO_TOML}" | head -n1)"
if [[ -z "${cargo_version}" ]]; then
  echo "Failed to read version from ${CARGO_TOML}" >&2
  exit 1
fi

sed -i '' "s/__version__ = \".*\"/__version__ = \"${cargo_version}\"/" "${INIT_PY}" 2>/dev/null \
  || sed -i "s/__version__ = \".*\"/__version__ = \"${cargo_version}\"/" "${INIT_PY}"

echo "Synced PyPI package version from Cargo.toml:"
echo "  ${INIT_PY} -> ${cargo_version}"
