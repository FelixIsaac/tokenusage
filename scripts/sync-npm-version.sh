#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TOML="${ROOT_DIR}/Cargo.toml"
NPM_PACKAGE_JSON="${ROOT_DIR}/npm/tu/package.json"

cargo_version="$(sed -nE 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "${CARGO_TOML}" | head -n1)"
if [[ -z "${cargo_version}" ]]; then
  echo "Failed to read version from ${CARGO_TOML}" >&2
  exit 1
fi

node - "${NPM_PACKAGE_JSON}" "${cargo_version}" <<'NODE'
const fs = require("fs");
const pkgPath = process.argv[2];
const version = process.argv[3];
const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
pkg.version = version;
fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
NODE

echo "Synced npm package version from Cargo.toml:"
echo "  ${NPM_PACKAGE_JSON} -> ${cargo_version}"
