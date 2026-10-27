# Windows Sidecars Plan (release slice 3)

## Goal

Windows sidecars pinned, fetched, verified, and staged exactly like macOS:
`sidecars/manifest.json` gains Windows releases, `fetch-sidecars` handles
zips and a `--platform` selector, `release:preflight` verifies Windows
hashes, the daemon facade resolves the bundled layout, and Tauri resources
include the Windows builds.

## Constraints

- Darwin flow byte-identical: existing manifest entries, validation, and
  tests keep passing unchanged; Windows arrives as additive entries.
- No new production dependencies: zip extraction uses `node:zlib`
  (`inflateRawSync`) plus a minimal zip reader with zip-slip rejection
  and the existing size caps.
- Version pins stay: CoreDNS 1.14.7 (x64 only — no ARM64 upstream asset),
  Caddy 2.11.7 (x64 + ARM64). Executable digests computed host-side during
  implementation and verified against the published checksums files.
- Layout mirrors darwin: `sidecars/build/<target>/coredns-<target>[.exe]`,
  `caddy-<target>[.exe]`, `manifest.json` (runtime manifest, schema 1 —
  the Rust `SidecarManifest` already parses it).

## Out of Scope

Facade selection (done, slice 1), setup/uninstall (done, slice 2), Tauri
bundle target (slice 4), version bump/release check (slice 5).

## Verification Commands

- test: `pnpm exec vitest run tests/scripts/fetch-sidecars.test.ts`
- test: `cargo test -p own-domains-daemon --test windows_paths` (host)
- lint: `pnpm run lint` (scripts) and both cargo clippies with `-- -D warnings`
- build: `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc`
- release: `pnpm run sidecars:fetch -- --verify-only` (host, after staging)

### Task 1: Manifest schema for Windows releases

Files:
- modify `sidecars/manifest.json` (types only — no entries yet)
- modify `scripts/fetch-sidecars.ts`
- modify `tests/scripts/fetch-sidecars.test.ts`

Steps:
1. Write test: a manifest with `windowsReleases` entries validates;
   unknown arch, unknown Windows target (`x86_64-pc-windows-msvc` and
   `aarch64-pc-windows-msvc` are the only two), duplicate arch per
   sidecar, bad digest, and non-GitHub URL each fail; a manifest without
   `windowsReleases` still validates exactly as before.
2. Run the vitest file; confirm the new cases fail.
3. Implement: `WindowsRelease` type (`architecture`, `target`, `url`,
   `archiveMember`, `checksum`, `executableSha256`), `windowsReleases?`
   on each sidecar definition, validation mirroring the darwin rules.
4. Run the vitest file; confirm green. Run `pnpm run lint`.
5. Run `pnpm test` (full vitest suite) to confirm nothing regressed.

Done when: schema accepts sparse Windows matrices and still enforces
every digest/URL rule, with darwin behavior untouched.

### Task 2: Zip member extraction without new dependencies

Files:
- modify `scripts/fetch-sidecars.ts`
- modify `tests/scripts/fetch-sidecars.test.ts`

Steps:
1. Write test: `extractZipMember` returns the exact bytes of a stored and
   a deflated member built synthetically in-test; rejects `../` and
   absolute member paths; rejects truncated archives; respects the
   existing byte caps.
2. Run; confirm it fails (no such function).
3. Implement with `node:zlib` `inflateRawSync`, central-directory parsing,
   and the same traversal/size guards as the tar path.
4. Run the vitest file; confirm green.

Done when: zip extraction is covered like the tar path, no new packages.

### Task 3: Real Windows manifest entries with verified digests

Files:
- modify `sidecars/manifest.json`

Steps:
1. Download host-side: CoreDNS `coredns_1.14.7_windows_amd64.zip`,
   Caddy `caddy_2.11.7_windows_{amd64,arm64}.zip`; verify each against
   its published checksums file (already known: CoreDNS zip SHA-256
   `6b718a5e…`; Caddy SHA-512s in `caddy_2.11.7_checksums.txt`).
2. Extract each `.exe`, record SHA-256 of the bytes as `executableSha256`.
3. Add the three entries (CoreDNS amd64 only; Caddy both archs) with
   `archiveMember` `coredns.exe` / `caddy.exe`.
4. Run `pnpm exec vitest run tests/scripts/fetch-sidecars.test.ts`
   manifest-validation cases against the real file; confirm green.

Done when: the manifest pins three Windows builds with published +
computed digests, all validation passing.

### Task 4: Fetch and verify flow for `--platform windows`

Files:
- modify `scripts/fetch-sidecars.ts`
- modify `tests/scripts/fetch-sidecars.test.ts`

Steps:
1. Write test: with a stub fetcher, fetching `windows/amd64` downloads
   the zip, extracts the member, writes `coredns-x86_64-pc-windows-msvc(.exe)`
   plus runtime `manifest.json`, and records the right paths; checksum
   mismatch and executable-digest mismatch each fail without writing;
   requesting CoreDNS `arm64/windows` fails (no such release);
   darwin fetch output is byte-identical to before.
2. Run; confirm it fails.
3. Implement: `--platform darwin|windows` (default `darwin`), per-entry
   arch filtering from the manifest arrays, zip vs tar dispatch by
   extension, `.exe` suffix and no chmod reliance on Windows outputs,
   skip `runVersion` for non-host targets (mirror cross-arch darwin).
4. Run vitest file + `pnpm run lint`; confirm green.

Done when: Windows fetch round-trips through stubs and darwin is
untouched.

### Task 5: Daemon facade resolves the bundled layout

Files:
- modify `crates/own-domains-daemon/src/platform/windows/factory.rs`
- modify `crates/own-domains-daemon/tests/windows_paths.rs`

Steps:
1. Write test: `resolve_paths` gains bundled-layout resolution — given
   a resources dir containing
   `sidecars/x86_64-pc-windows-msvc/{coredns-…,caddy-…,manifest.json}`,
   the factory picks those exes; an explicit `--sidecars` dir still
   wins; missing bundled exes fall back to the exe directory.
2. Run; confirm it fails.
3. Implement the lookup (mirror the macOS resources-dir pattern, no
   manifest parsing in the facade — existence check only).
4. Run host unit tests; confirm green. Run both clippies.

Done when: bundled vs override resolution is pinned without touching
the network.

### Task 6: Tauri resources for the Windows builds

Files:
- modify `crates/own-domains-desktop/tauri.conf.json`

Steps:
1. Add `resources` entries for
   `sidecars/build/x86_64-pc-windows-msvc/*` and
   `sidecars/build/aarch64-pc-windows-msvc/*` (caddy only for ARM64),
   following the existing darwin entry shape.
2. Validate JSON parses and `pnpm run lint` passes.
3. Confirm `sidecars:fetch -- --verify-only` stays green on host once
   staged (staging itself is Task 7's host step).

Done when: the bundle config references exactly the staged layout.

### Task 7: Stage, preflight, and VM check

Steps:
1. Host: run the fetch for both Windows targets into `sidecars/build`
   (ignored output, not committed); run
   `pnpm run sidecars:fetch -- --verify-only`; confirm green.
2. VM: `cargo test -p own-domains-daemon --test windows_paths`
   passes (bundled-layout cases run on the real target).
3. VM: point `--sidecars` at a directory staged in the Task-2 layout
   and re-run the facade smoke (`status` shows stopped runtime).

Done when: hashes verify on host and the layout resolves on the guest.

### Task 8: Full verification and commit

Steps:
1. `cargo test --workspace --locked` green on host.
2. Both clippies, `pnpm run lint`, `pnpm test`, three target checks, fmt.
3. Commit `feat:` + push `feat/windows-release`.

Done when: slice 3 is pushed with host + guest evidence.
