# Release signing identity

`packaging/release-signing/public.pem` pins Mochi's persistent Ed25519 release
identity. Its private key is owned by the maintainer, outside this repository,
at `$XDG_CONFIG_HOME/mochi-release-signing/release-ed25519.pem` (or
`~/.config/mochi-release-signing/release-ed25519.pem` when XDG_CONFIG_HOME is
unset), with mode 0600. This directory is separate from Mochi's application
configuration, Themes, Resources and plugin state. Initialization refuses to
overwrite an existing key or replace a pinned identity. If the public identity
is already pinned but its private key is missing, restore the existing key;
`--init` will not generate a replacement. Public artifacts contain only the public key.

The former `$XDG_CONFIG_HOME/mochi/release-signing/release-ed25519.pem` location
remains supported. When only that key exists, signing uses it and prints a
legacy-path notice. It does not silently move or rotate the key. To explicitly
migrate it to the independent directory:

```sh
/usr/bin/python3 scripts/sign-release.py --migrate-key
```

Migration validates the key against `packaging/release-signing/public.pem`,
durably copies it without overwriting an existing destination, then removes the
old private-key file. The private-key bytes and public fingerprint stay the same;
the pinned public file is never rewritten. If both default paths hold different
identities, normal signing, initialization and migration refuse to proceed.
Resolve that conflict explicitly; do not delete keys to bypass the check.
`--migrate-key` cannot be combined with `--key` or `--init`.

`mochi-purge-data --purge` deletes Themes, Resources, preferences and plugin
state, but preserves the independent signing directory and the **entire legacy
release-signing directory**, including any public identity or backup stored
there. Purge does not migrate keys. The pinned public key in the source tree is
also outside application data. For a custom `--key` or `--public`, choose a
location outside the Mochi user-data roots; arbitrary custom paths inside those
roots are not protected. See `scripts/sign-release.py --help` for all options.

Sign reviewed artifacts locally:

```sh
/usr/bin/python3 scripts/sign-release.py --output target/release-signatures \
  target/mochi-public-source.tar.gz target/mochi-staging.deb
openssl pkeyutl -verify -rawin -pubin \
  -inkey packaging/release-signing/public.pem \
  -in target/release-signatures/SHA256SUMS \
  -sigfile target/release-signatures/SHA256SUMS.sig
```

After verifying the signature, run `sha256sum -c SHA256SUMS` in the directory
containing the named artifacts. Obtain the public key fingerprint through an
independently trusted channel before relying on a first release. The signature
proves integrity under this identity, not approval to redistribute artwork.

Keep an encrypted offline backup of the private key. Do not commit it, copy it
into a package, or put it in ordinary CI environment variables. Rotation must
be explicit and accompanied by a notice authenticated using the previous key
where possible. Lost-key recovery requires independently announcing the new
fingerprint; initialization never silently rotates the old key.

CI's ephemeral staging signature remains separately labelled. It is not the
persistent release identity. No workflow automatically publishes a Release.
