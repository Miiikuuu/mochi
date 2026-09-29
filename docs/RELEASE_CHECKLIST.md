# Release checklist

- [ ] Original-code Apache-2.0 notice and third-party license inventory reviewed.
- [ ] Public binary has no Spine dependency. A private full build is never a
      public release substitute; distribution rights require a separate audit.
- [ ] No unknown-rights characters, DIY art, font files or private brand assets.
- [ ] Audit Cargo/npm dependency licenses and distribute required notices.
- [ ] Public Source clone passes `sh scripts/public-ci.sh`, including workspace
      fmt/clippy/tests and a real source-mode GTK4 screenshot.
- [ ] Private monorepo regression evidence retained separately: GTK3 and GTK4.
- [ ] Session 15, Native 14, security and installation checks pass.
- [ ] Six light/dark 420/720/960 screenshots and generated summaries retained.
- [ ] Staging archives from identical inputs have identical SHA-256.
- [ ] Install on Ubuntu 24.04 with dependencies, test application-menu entry.
- [ ] Hide source checkout in a private mount namespace; installed GTK4 Manager,
      Renderer, BubbleHost and Sessiond still work.
- [ ] Verify actual systemd user socket activation on a desktop login; Journal
      contains no credentials, paths to private data or message bodies.
- [ ] Pending reply becomes uncertain before service replacement; old ACKs fail.
- [ ] Upgrade/reinstall preserves user data; remove/purge package preserves it.
      Destructive per-user deletion requires the separate explicit --purge tool.
- [ ] Test GTK3 Legacy entry once; no new functionality is added to Legacy.
- [ ] Only publish reviewed package artifacts, never an unfiltered checkout ZIP.

A check mark requires linked results from the actual environment. CI artifacts
are evidence; these unmarked release gates do not claim a release was approved.
