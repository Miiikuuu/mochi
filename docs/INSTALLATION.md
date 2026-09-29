# Installed Mochi (0.9.1 staging)

Original code is Apache-2.0. This is an installation foundation, not a public
binary release approval. Review ASSET_LICENSES.md and the release checklist,
before publishing artifacts. Default packages exclude Spine; private full builds
explicitly use `--features spine` and `build-package.py --spine`.

Build on Ubuntu 24.04 with the dependencies listed in CI, including `dpkg-dev`
for automatic ELF dependency version constraints:

```sh
cargo build --release -p mochi-desktop
/usr/bin/python3 scripts/build-package.py target/stage \
  --archive target/mochi-0.9.1-staging.tar.gz --deb
sudo apt install ./target/mochi-0.9.1-staging.deb
mochi-manager
```

The tar archive is a reproducible staging tree rooted at `usr/`; it includes
`/usr/share/doc/mochi/manifest.json` with hashes, a filtered original demo character, default bubble,
application metadata, locales and licenses. Existing imported assets remain in
the source tree and user data; they are not copied into a public-facing package.
A temporary `mochi-manager-legacy` command retains GTK3 for one transition release.

| Location | Purpose |
| --- | --- |
| `/usr/bin/mochi-manager` | Default GTK4 entry |
| `/usr/libexec/mochi/platform/linux/` | Installed helpers and Sessiond |
| `/usr/libexec/mochi/bin/mochi-desktop` | Renderer |
| `/usr/share/mochi/` | Bundled resources and plugin code |
| `/usr/share/applications/` | Desktop entry |
| `/usr/share/metainfo/` | AppStream metadata |
| `/usr/share/icons/` | Distribution-safe application icon |
| `/usr/share/locale/` | gettext catalogs |
| `/usr/lib/systemd/user/` | User socket/service units |

The user socket is enabled by the packaged sockets.target dependency. It is
available after login; the first request starts Sessiond. Existing desktop
sessions can run `systemctl --user daemon-reload` followed by
`systemctl --user start mochi-sessiond.socket`. Installed clients prefer socket
activation. On systems without a user systemd bus, the compatibility launcher
uses a private start lock and detached daemon. That fallback is not a Journal
service; desktop installations should use systemd.

```sh
systemctl --user status mochi-sessiond.socket mochi-sessiond.service
journalctl --user -u mochi-sessiond.service
```

Connections Diagnostics includes sanitized service state, generation, build and
protocol. No full binding key, token or message body is displayed. A graceful
upgrade first publishes Uncertain for outstanding replies. The replacement
service has a new generation and requires reconnecting the ChatGPT card; it
never silently resends a pending reply. Reopen Manager after replacing the
service to regain its write capability. Package upgrades also request a reload of an active Mochi
MCP gateway worker without rotating its URL. Following an unexpected daemon
crash, reload the MCP gateway/stdio server before creating a new card. Renderer/BubbleHost reacquires authority
through its fixed trusted entrypoint. Killing the daemon with SIGKILL cannot
publish a final event; subscribers report Disconnected.

`KillMode=process` deliberately keeps trusted GUI children alive when replacing
the service; stopping Sessiond is not the same as stopping the desktop Theme.
Close/stop the Theme explicitly when uninstalling a running installation.

Uninstall with `sudo apt remove mochi`. Upgrade, reinstall, remove and dpkg purge
preserve all user Themes, Resources and settings. To permanently delete a user's
data, stop Mochi first and run `mochi-purge-data --purge` as that user before
uninstalling. The source equivalent is `python3 scripts/purge-user-data.py --purge`. Without `--purge` this script removes nothing. Do not run it via sudo.
Release signing identities are separate from application data: purge preserves
`$XDG_CONFIG_HOME/mochi-release-signing` and the legacy
`$XDG_CONFIG_HOME/mochi/release-signing` directory (default XDG_CONFIG_HOME is
`~/.config`). See [release signing](RELEASE_SIGNING.md) for explicit key migration.

For a trusted MCP bridge use the installed `http.mjs` or `mcp.mjs` entrypoints;
these enter the daemon launch broker and receive an inherited FD. The existing reloadable gateway also launches its authenticated worker through
the broker, preserving its public address and reload protocol. Standalone HTTP
requires a separate MOCHI_MCP_TOKEN access token; this is not a Session Capability.
Third-party plugins cannot opt into these roles or ChatGPT scope. They run with
PID/network isolation and private runtime access; if Bubblewrap is unavailable,
third-party execution is disabled rather than exposing the account's session brokers.

CI tests the installed package after hiding the source checkout inside a private
mount namespace. It also exercises LISTEN_FDS activation, graceful replacement,
archive reproducibility and preserved user data. Local checks without mount
privileges explicitly report `source_hidden: false` rather than claiming that
part of acceptance passed.
