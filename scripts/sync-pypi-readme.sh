#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cp "${ROOT_DIR}/README.md" "${ROOT_DIR}/pypi/tu/README.md"
echo "Synced pypi/tu/README.md from root README.md"
