#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
test -f PUBLIC_SOURCE_DISTRIBUTION.json
mkdir -p target
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm ci --prefix plugins/mochi-chatgpt --ignore-scripts --no-audit --no-fund
node plugins/mochi-chatgpt/build-ui.mjs --check
/usr/bin/python3 scripts/bundle-license-check.py
/usr/bin/python3 scripts/npm-license-check.py
/usr/bin/python3 scripts/release-signing-check.py
/usr/bin/python3 scripts/public-manager-check.py
if cargo check -p mochi-desktop --features spine > target/spine-rejection.log 2>&1; then
  echo 'Public source unexpectedly enabled Spine' >&2
  exit 1
fi
grep -q 'Spine is unavailable in Public Source' target/spine-rejection.log
