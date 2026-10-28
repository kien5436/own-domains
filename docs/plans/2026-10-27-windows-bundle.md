# Windows Bundle Plan (release slice 4)

## Goal

`pnpm run desktop:build:windows` on a Windows machine produces an
installable unsigned NSIS bundle: app + daemon and CLI sidecar binaries
+ staged Windows sidecars, with a Windows icon and no macOS-only
assumptions in the staging path.

## Constraints

- No new production dependencies. Icons come from `tauri icon` on the
  existing `icon.png`.
- macOS flows byte-identical: `desktop:build`, `:arm64`, `:x86_64`,
  and `stage-desktop` defaults unchanged.
- Beta scope is x64 only: ARM64 Windows runs the x64 bundle emulated
  (proven pattern from the VM milestone). No ARM64 bundle target yet.
- Unsigned: no signing config; SmartScreen path stays documented, not
  suppressed.
- WebView2: Tauri default (`downloadBootstrapper`) — no config needed.

## Out of Scope

Facade (done), setup/uninstall (done), sidecars (done), version bump and
release checks (slice 5), the actual bundle build (needs x64 hardware —
queued for slice 5 with the machine).

## Verification Commands

- test: `pnpm test` (full vitest suite)
- test: `cargo test --workspace --locked` (host)
- lint: `pnpm run lint`
- build: `pnpm exec tauri build --help` parses (config loads); JSON parses

### Task 1: Windows application icon

Files:
- add `crates/own-domains-desktop/icons/icon.ico` (generated)

Steps:
1. Run `pnpm exec tauri icon crates/own-domains-desktop/icons/icon.png`
   from the repo root; confirm `icon.ico` appears.
2. Confirm `git status` shows only icon additions (plus any icon set
   refresh the tool performs — review the diff, keep it).
3. Run `pnpm test`; confirm green.

Done when: the bundle has a Windows icon from the existing artwork.

### Task 2: Stage the daemon with `.exe` on Windows targets

Files:
- modify `scripts/stage-desktop.ts`
- create `tests/scripts/stage-desktop.test.ts` (or extend lifecycle)

Steps:
1. Write test: the staged file name mapping yields
   `own-domains-daemon-x86_64-pc-windows-msvc.exe` for the Windows
   triple and `own-domains-daemon-aarch64-apple-darwin` (no extension)
   for darwin; the executable-bit assertion is skipped on `win32`.
2. Run; confirm it fails.
3. Implement: extract a pure `stagedDaemonName(target)` helper, `.exe`
   suffix when the target contains `windows`, gate the mode check on
   `process.platform !== 'win32'`.
4. Run vitest + lint; confirm green.

Done when: staging is correct per target with the macOS path unchanged.

### Task 3: `desktop:build:windows` script (x64, NSIS)

Files:
- modify `package.json`

Steps:
1. Add `desktop:build:windows`: release daemon build, stage with
   `--target x86_64-pc-windows-msvc`, then
   `tauri build --target x86_64-pc-windows-msvc --bundles nsis`
   (no `--no-bundle`).
2. Validate: `pnpm exec tauri build --help` works; JSON config parses;
   script fails fast with a clear message when run on macOS
   (tauri itself errors on the foreign target — confirm the error names
   the target).
3. Run `pnpm run lint`; confirm green.

Done when: the release bundle has a one-command entry point.

### Task 4: Full verification and commit

Steps:
1. `pnpm test` green, `pnpm run lint` clean.
2. `cargo test --workspace --locked` green on host.
3. Commit `feat:` + push `feat/windows-release`.

Done when: slice 4 is pushed; the bundle build itself runs on the x64
machine in slice 5.
