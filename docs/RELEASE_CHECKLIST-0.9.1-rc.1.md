# 0.9.1-rc.1 release checklist (unpublished)

Candidate identifier: **0.9.1-rc.1**. Product/package version: **0.9.1 staging**.
This document is a review gate, not an announcement or distribution approval.
The private monorepo remains private; no formal public mirror has been pushed.

- [ ] Link the final commit and successful private-monorepo CI run.
- [ ] Bundle rebuild from npm lockfile matches committed HTML and all license
      artifacts; ext-apps 2.0.0 appears in source/binary inventory and SPDX.
- [ ] Public Source manifest, forbidden hashes, media and Git object audit pass.
- [ ] One-root-commit public clone passes its complete exported CI, including
      workspace fmt/clippy/tests and a real uninstalled GTK4 screenshot.
- [ ] Default renderer dependency graph excludes Spine; explicit public Spine
      is rejected while its inert workspace stub builds.
- [ ] Installed no-Spine package opens Welcome/Mochi Dot with checkout hidden;
      old Themes stay visible and are not silently rewritten.
- [ ] Private Full personal-install result is retained separately, without
      private assets; never include the private-full .deb in public artifacts.
- [ ] GTK3, GTK4, session/security and lifecycle regression evidence retained.
- [ ] Missing Resources has a separate keyboard-accessible Repair Banner.
- [ ] lintian reports zero errors/warnings; AppStream, Desktop Entry and systemd
      metadata validate; installed commands have Man Pages.
- [ ] Two independent builds yield byte-identical tar and .deb artifacts.
- [ ] Review SPDX, Cargo/npm/Bundle notices and actual included licenses.
- [ ] Verify SHA256SUMS with the pinned persistent release public key.
- [ ] Maintainer has securely backed up the signing key and independently
      verified its fingerprint. This cannot be inferred from a CI success.
- [ ] Approve a dedicated public mirror and exact artifacts before any release.

Evidence paths are generated outputs: CI artifacts; `target/public-clone/target/`
for exported-source checks; `docs/private-full-install/results.json` is a
private-monorepo report and intentionally absent from Public Source. Check off
gates only after reviewing the corresponding final results. No release is
published by generating this checklist or signing a staging artifact.
