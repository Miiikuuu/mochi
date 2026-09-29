# Existing-user installation migration

The Cargo workspace package version is the single product version source.
Installation metadata, Debian control, AppStream and Sessiond use that value;
the package check also verifies the current Changelog heading.

Theme schema validation is independent of package availability. Missing
characters, bubbles and plugins retain their original IDs, action settings,
Theme name and selection. Apply validates availability before touching Runtime.
Cast supports replacement even when all four slots are occupied; missing plugins
can be disabled and missing bubbles replaced. Locate opens the resource importer.

If installed resources cannot run any saved Theme, an additional Welcome Theme
is offered in memory. Merely opening Manager never rewrites `themes.json`.
An explicit save persists the resulting list. No old Theme is replaced by Dot.

Catalog paths and exact old generated Desktop/Autostart entries identify former
checkouts. Library offers **Adopt into Library** for surviving bundled packages.
That explicit action copies validated bytes to `$XDG_DATA_HOME/mochi`, commits
through the resource transaction journal and leaves the original directory and
Theme bytes intact. Adopted plugins become local and unreviewed.
Customized launch entries are preserved byte-for-byte; exact generated desktop
entries are removed so the system entry wins, and exact autostart entries are
updated to the installed launcher.

The shipped Node bridges support Node **18.19 or newer**. The App SDK requiring
Node 20 is a build-only dependency: the generated browser bundle is shipped,
and `npm ci --omit=dev` excludes that SDK from installed server dependencies.
Installed CI resets PATH and exercises `/usr/bin/node`, including actual MCP
create, attach, poll and acknowledge operations.

CI validates installed metadata with lintian, appstreamcli,
desktop-file-validate and systemd-analyze. Independent Cargo target directories
produce two binaries, two staging archives and two Debian packages, compared
byte-for-byte. Installed screenshots wait for a Viewable window and nonempty
first frame. Tests preserve old Lime Squad fixture bytes before any explicit
save, with and without the old catalog, including a hidden source checkout.
