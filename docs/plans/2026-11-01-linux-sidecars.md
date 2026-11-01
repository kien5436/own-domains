# Linux Sidecars Plan (Milestone 2, Phase B)

## Goal

Pin, fetch, verify, and stage Linux sidecars exactly like darwin and
Windows: `linuxReleases` entries in `sidecars/manifest.json`,
`--platform linux` fetch/verify support, `release:preflight` coverage,
and Tauri resource entries for both Linux targets. No new production
dependencies.

## Constraints

- Manifest shape follows the design doc, not the Windows precedent:
  `linuxReleases` is a per-architecture *map* keyed like `releases`,
  because both sidecars ship both Linux archs (CoreDNS has no such gap
  on Linux). Validation rules mirror darwin exactly (URL allowlist,
  digest formats, member-name rule, target allowlist).
- Linux assets are `.tgz`/`.tar.gz` — the existing tar extraction path
  is reused; no zip work needed. Executable names have no `.exe` suffix:
  `coredns-<target>` / `caddy-<target>`, matching the darwin convention
  the daemon facade already expects.
- Darwin and Windows flows stay byte-identical: new code paths activate
  only under `--platform linux`.
- Digests are computed host-side from real downloads and verified against
  the published checksums files (CoreDNS `.sha256`, Caddy `checksums.txt`
  with its sigstore signature); nothing is invented.
- Facade bundled-layout resolution is Phase C, not here: this phase
  produces staged files plus the manifest entries that describe them.

## Out of Scope

Unix extraction (done, Phase A), Linux backend + CLI wiring (Phase C),
setup/uninstall transactions (Phase D), `.deb` packaging (Phase E).

## Verification Commands

- test: `pnpm exec vitest run tests/scripts/fetch-sidecars.test.ts` (repo root)
- lint: `pnpm run lint` (repo root)
- check: `pnpm run sidecars:fetch -- --platform linux` then `--verify-only` (repo root, host)
- release: `pnpm run release:preflight` (repo root, includes the new target)
- rust: `cargo test --workspace --locked`, both clippies, three target checks, fmt (unchanged code, but run to prove it)

### Task 1: `linuxReleases` manifest schema and validation

Files:
- modify `scripts/fetch-sidecars.ts`
- modify `tests/scripts/fetch-sidecars.test.ts`

Steps:
1. Write test: a `describe` block mirroring the windows one — valid map
   entries for both archs validate; unknown arch key, wrong target for
   the arch, Darwin target in a Linux entry, bad digest, non-GitHub URL,
   non-plain member, and duplicate handling (maps dedupe naturally —
   assert the parsed shape instead) each fail; a manifest without
   `linuxReleases` still validates.
2. Run; confirm the new cases fail (no `linuxReleases` support).
3. Implement: `LinuxReleases` map type, `LINUX_TARGETS` allowlist
   (`amd64` → `x86_64-unknown-linux-gnu`,
   `arm64` → `aarch64-unknown-linux-gnu`), validation mirroring the
   darwin rules with `linux/` in the error labels.
4. Run vitest + lint; confirm green.

Done when: the map validates exactly like `releases` and darwin is
untouched.

### Task 2: Real Linux manifest entries with verified digests

Files:
- modify `sidecars/manifest.json`

Steps:
1. Download host-side: CoreDNS `1.14.7` `linux_amd64`/`linux_arm64`
   `.tgz`, Caddy `2.11.7` `linux_amd64`/`linux_arm64` `.tar.gz`;
   verify each against its published checksum (CoreDNS `.sha256`
   files, Caddy shared `checksums.txt` entry).
2. Extract each binary, record SHA-256 as `executableSha256`.
3. Add the four entries with `archiveMember` `coredns` / `caddy`.
4. Validate the real manifest through the new schema path.
5. Confirm `git diff --stat` on the manifest is additive only.

Done when: the manifest pins four verified Linux builds.

### Task 3: Fetch and verify flow for `--platform linux`

Files:
- modify `scripts/fetch-sidecars.ts`
- modify `tests/scripts/fetch-sidecars.test.ts`

Steps:
1. Write test: with a stub fetcher, fetching `linux/amd64` downloads
   the tgz, extracts the member, writes
   `coredns-x86_64-unknown-linux-gnu` (no `.exe`) plus the runtime
   manifest, and records the right paths; checksum mismatch and
   executable-digest mismatch each fail with nothing written;
   darwin fetch output is byte-identical to before.
2. Run; confirm it fails (platform defaults to darwin; no linux path).
3. Implement: `--platform darwin|windows|linux` (default stays
   `darwin`); per-platform target map and release lookup
   (`releases` for darwin, `windowsReleases` array for windows,
   `linuxReleases` map for linux); tar extraction by `.tgz`/`.tar.gz`
   suffix; skip `runVersion` for non-host targets (extend the existing
   host/target comparison to the Linux map); default arch set for
   linux is both archs.
4. Run vitest + lint; confirm green.

Done when: Linux fetch round-trips through stubs and darwin/windows
output is unchanged.

### Task 4: Tauri resources for the Linux builds

Files:
- modify `crates/own-domains-desktop/tauri.conf.json`

Steps:
1. Add `sidecars/build/x86_64-unknown-linux-gnu` and
   `sidecars/build/aarch64-unknown-linux-gnu` directory mappings
   following the Windows entry shape.
2. Validate JSON parses; `pnpm run lint` passes.
3. Confirm the Windows entries are untouched.

Done when: the bundle config references the staged Linux layout.

### Task 5: Stage, preflight, and full verification

Steps:
1. Host: `pnpm run sidecars:fetch -- --platform linux` into
   `sidecars/build` (ignored output, not committed).
2. Host: `pnpm run sidecars:fetch -- --verify-only --platform linux`
   (hmm — verify the flag composes; adjust the command if the CLI
   rejects the combination, documenting the working form).
3. Host: `pnpm run release:preflight`; confirm all five targets verify.
4. Run the full Rust + vitest + lint + target-check verification.
5. Commit `feat:` + push `feat/linux-host-rendering`.

Done when: real Linux sidecars are staged and verified with host +
target evidence, and the branch is pushed.
