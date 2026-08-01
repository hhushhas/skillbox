# Skillbox release session log — 2026-08-01

- 20:45 PKT — Started the 0.4.0 release work. Mapped the GitHub tag workflow, Homebrew tap, Cargo manifest, Wrangler site target, and npm registry state. The unscoped `skillbox` npm name belongs to an unrelated package, while `@hhushhas/skillbox` is available but npm authentication is not configured locally.
- 21:08 PKT — The requested high-reasoning commit review could not initialize its app-server client (`Operation not permitted`), so it was recorded as a failed review rather than coverage.
- 21:10 PKT — M4 verification passed on `4656a2d`; the first GitHub release run exposed a CI-only Claude setup fixture failure because the test did not create a settings file when no `claude` executable was present.
- 21:12 PKT — Fixed the fixture in `0c5b46c`, reran local and M4 verification successfully, pushed `main`, and moved the newly-created unreleased `v0.4.0` tag with an explicit force-with-lease.
- 21:16 PKT — GitHub release run `30707616265` passed all five target builds and published the five binaries plus `SHA256SUMS` at v0.4.0.
- 21:18 PKT — Updated Homebrew tap commit `2d13b17`; formula audit passed, `brew reinstall hhushhas/tap/skillbox` installed 0.4.0, and `brew test` plus harness-status verification passed.
- 21:19 PKT — Deployed the marketing site with Wrangler to `skillbox.hasanshoaib.com`; live HTTP verification returned 200 and contained the audit, setup, and npm messaging. Deployment version: `320ad7e7-4ee7-4664-85a2-4c516ff68c06`.
- 21:20 PKT — Cargo publication remains blocked by `cargo publish --locked`: `no token found`; npm publication remains blocked by `npm whoami`: `401 Unauthorized`. The scoped npm launcher itself passed a real v0.4.0 download, checksum, extraction, forwarding, and cleanup test.
- 22:12 PKT — Published `@hhushhas/skillbox@0.4.0` successfully. The shell had split `--registry=https://` from `registry.npmjs.org/`, so npm warned about the malformed option and zsh tried to execute the second line; a direct registry check now returns 0.4.0 with `latest` set correctly.
- 22:16 PKT — The first immediate npx test was held by the local npm `min-release-age=3` policy; setting `npm_config_min_release_age=0` for the test made npx download, verify, and run the published package successfully.
- 22:18 PKT — Committed npm-first installation docs and marketing copy in `1192acb`, removed Cargo installation messaging from the marketing site, pushed `main`, and redeployed the site. Wrangler deployment version: `396c9f2b-b6eb-413b-a75b-070877079666`.
- 22:19 PKT — Verified the published package through isolated global npm installation and npx execution; both reported `skillbox 0.4.0`.
