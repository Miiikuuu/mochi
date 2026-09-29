# Public source candidate and build lanes

The private monorepo remains private. No workflow in this phase creates a GitHub
Release or changes repository visibility. `packaging/public-source.json` is the
source allow-list. `scripts/public-source.py` builds an independent tree and tar
without `.git`, history, vendor runtimes, imported characters, DIY artwork,
portraits, fonts, user logo or reference media. The product PNG is the original
procedurally generated Mochi Dot. Public tests also carry explicitly reviewed
1×1/8×8 diagnostic sprites and codec vectors. A hash scan detects renamed private media.

The public tree contains original Mochi code, Default Bubble, release icon,
selected documentation and dependency lockfiles. The optional Spine dependency
is replaced by an inert stub; its explicit runtime feature rejects compilation. Public source can compile and
package without access to the private checkout; enabling Spine there fails
explicitly. The complete default workspace, including the stub and exported
tests, passes fmt, clippy and tests. Installed Manager also blocks Apply for existing Spine assets when
the installed binary lacks that feature; it offers Welcome without replacing
the saved Theme. CI builds the extracted source and checks its Cargo dependency tree.

Default `cargo build -p mochi-desktop` uses the Sprite lane. Private full builds
explicitly pass `--features spine`; the existing private run-desktop script does
so. Tests for original Spine assets run only in that lane. GPU triangle data is
defined in Core, avoiding an indirect Spine dependency in the Sprite renderer.
`--build-info` reports the actual compiled version and feature, and the package
builder rejects a mismatched binary. `build-package.py --spine` is a private
staging option and includes the separate Spine license; it is not redistribution
permission. No public lane contains Spine code or makes a Spine license claim.

Original product code is Apache-2.0. The entire binary is **not** described as
Apache-only: Cargo and npm dependencies keep their upstream license expressions,
notices and attribution. Each package contains SPDX 2.3, Cargo/npm inventories,
license texts, build-tool versions, source-tree/lockfile/binary SHA-256 provenance
and a manifest under `/usr/share/doc/mochi`. System libraries remain distribution
dependencies, not embedded copies. Review the actual SBOM before release.

CI signs the SHA-256 manifest with an ephemeral Ed25519 staging key. The public
key and detached signature allow integrity verification for that run; this is
not a long-term publisher identity. A public release must use a separately
managed release signing key or trusted CI identity after mirror audit. No private
signing key is included in either artifact.

Two independently compiled binaries and package trees must yield identical tar
and deb bytes. Installed acceptance uses the system Node and validates nonempty
GTK4 screenshots, original Dot rendering, migration fixtures and MCP delivery.

The committed ChatGPT connection card is also a distribution artifact. Its
esbuild metafile closes the browser dependency graph separately from production
Node dependencies. `npm ci --prefix plugins/mochi-chatgpt` followed by
`node plugins/mochi-chatgpt/build-ui.mjs --check` must reproduce the committed
HTML, metafile, license inventory and license texts exactly. Every non-Mochi
input must have a package/version/license record; missing records or notices
fail the build. Source and binary carry the same `bundled-js-licenses.json`.
The binary SPDX document describes the bundle and its CONTAINS relationships,
including `@modelcontextprotocol/ext-apps` 2.0.0, and preserves LICENSE/NOTICE
under `dependency-licenses/bundle/`.

`PUBLIC_SOURCE_DISTRIBUTION.json` selects Welcome, Mochi Dot and Default Bubble
in uninstalled source mode. `PUBLIC_SOURCE_FILES.json` records the audited file
set and hashes. `scripts/public-source-check.py` verifies that set before the
first temporary Git commit, clones without local object sharing, checks every
file, and runs `scripts/public-ci.sh` from the clone. No private history or
formal public remote is created. The generated GitHub workflow runs that same
CI entrypoint. A real Xvfb source-mode GTK4 screenshot is retained. Diagnostic
codec and sprite fixtures use explicit reviewed hashes; they are not user art.
