# Windows Release Plan (release slice 5 — needs the x64 machine)

## Goal

Cut `0.1.0-beta.3`: bump versions, build the signed-off NSIS bundle on
x86_64 Windows, run the full acceptance matrix, and record the release.

## Constraints

- Version bump is atomic across all eight manifests (six Cargo crates,
  `package.json`, `tauri.conf.json`); no partial bumps.
- The bundle build and x64 acceptance run on the x86_64 machine only.
  Nothing here re-verifies ARM64 behavior (already green).
- Unsigned stays unsigned: `release:check` (macOS-only full check) is
  not a gate for Windows; preflight + bundle + matrix are.
- Any x64 failure stops the release: fix forward on `feat/windows-release`,
  re-run the matrix, then bump.

## Out of Scope

Everything else (slices 1–4 done). No ARM64-native bundle, no signing,
no auto-updater.

## Verification Commands

- build: `pnpm run desktop:build:windows` (on the x64 machine)
- test: `cargo test --workspace --locked` (x64 machine + ARM64 VM)
- release: `pnpm run release:preflight` (either machine)

### Task 1: Version bump to 0.1.0-beta.3

Files:
- `package.json`, `crates/own-domains-desktop/tauri.conf.json`,
  `crates/{own-domains-core,own-domains-protocol,own-domains-client,own-domains-cli,own-domains-daemon,own-domains-desktop}/Cargo.toml`

Steps:
1. Replace `0.1.0-beta.2` with `0.1.0-beta.3` in exactly those eight
   files (verify with a repo-wide grep that no stragglers remain).
2. Run `cargo check --workspace` and `pnpm exec tauri build --help`
   on the host; confirm nothing references the old version.
3. Commit `chore:` + push.

Done when: the version is uniform and the tree still checks clean.

### Task 2: Build the NSIS bundle on x64 hardware

Steps:
1. Pull `feat/windows-release` on the x86_64 machine; install Rust 1.90+,
   Node + pnpm, WebView2 (present by default on Win10/11 — confirm in
   Settings > Apps if the build complains).
2. `pnpm install`, then `pnpm run desktop:build:windows`.
3. Confirm the `.nsis` installer exists under
   `crates/own-domains-desktop/target/release/bundle/nsis`.
4. Install it on the same machine (accept the SmartScreen warning —
   expected unsigned); confirm the app launches.

Done when: an installer built from the branch installs and launches.

### Task 3: x64 acceptance matrix

Steps:
1. `cargo test --workspace --locked` on the x64 machine; all green.
2. Elevated setup runbook (same as the ARM64 pass): preview, apply,
   NRPT rule present, firewall rule present, cert in per-user Root,
   `start` serves traffic, `stop`, uninstall leaves nothing behind.
3. Reboot persistence spot-check: after reboot, `.own` still resolves
   with the daemon running (note: autostart is out of scope — start the
   daemon manually after reboot).
4. Gate D retest: run the Job Object behavioral check on x64 hardware
   (the ARM64 `ERROR_BAD_LENGTH` block may be emulation-only).

Done when: native CoreDNS behavior, setup reversibility, and the Gate D
verdict are all recorded from real hardware.

### Task 4: Release notes and acceptance record

Files:
- modify `docs/windows-acceptance.md` (on `docs/cross-platform-adapters`)
- add release notes wherever the repo keeps them (check root for
  CHANGELOG/releases notes first; create `docs/release-0.1.0-beta.3.md`
  if none exists)

Steps:
1. Record: x64 matrix results, unsigned/SmartScreen expectation,
   WebView2 prerequisite, emulated-CoreDNS note for ARM64, icon
   placeholder status.
2. Commit + push the docs branch.

Done when: a reader can reproduce the release verdict from the docs.

### Task 5: Tag 0.1.0-beta.3 and merge

Steps:
1. Final host verification: `pnpm test`, `pnpm run lint`,
   `cargo test --workspace --locked`, both clippies, fmt.
2. Merge `feat/windows-release` to `main` with `--no-ff`, delete the
   branch local + remote.
3. Tag `v0.1.0-beta.3` on the merge commit, push the tag.

Done when: the release is tagged on `main` and the branch is gone.
