# Contributing to Mochi

Original contributions use Apache-2.0 unless a documented upstream license
applies. Do not contribute artwork without redistribution evidence. Preserve
existing uncommitted work and never commit credentials, private URLs or chats.

The Public Source artifact carries `PUBLIC_SOURCE_DISTRIBUTION.json`. It starts
with original Mochi Dot, Default Bubble and Welcome; private Spine resources
are not required. Build with `cargo build --release -p mochi-desktop`, then run
`/usr/bin/python3 platform/linux/manager_v2.py` directly from the extracted tree.
On Ubuntu, install the dependencies listed in `.github/workflows/ci.yml`.

Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`
and `cargo test --workspace`. In the Public Source distribution,
`sh scripts/public-ci.sh` runs the full exported CI, including the real GTK4
source-mode screenshot and browser bundle license verification. Explicit Spine
is unavailable in that distribution; private full source retains its own CI.

Manager state and controllers remain GTK-independent. Views must not access
stores, runtime services or the filesystem. Saving a Theme does not Apply it.
Keep revision conflicts, scoped Undo and resource leases intact. GTK3 Legacy is
frozen: fix regressions and security issues, without adding product pages.

After changing the browser card, run `npm ci --prefix plugins/mochi-chatgpt`
and `node plugins/mochi-chatgpt/build-ui.mjs`. Commit the generated HTML,
metafile, inventory and license texts together. `--check` verifies reproducibility.

See `docs/RELEASE_CHECKLIST.md` for release gates and `ASSET_LICENSES.md` for
licensing boundaries. Report actual test failures; never edit results to pass.
