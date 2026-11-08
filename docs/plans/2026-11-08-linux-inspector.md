# Linux Host Inspector Plan (Milestone D1)

## Goal

A fake-covered `LinuxHostInspector` that validates the installed Linux host integration and reports actionable diagnostics, wired into `LinuxLiveBackend::diagnose` after the manifest gate: artifact files vs manifest hashes, nftables ruleset + active table, systemd units present/enabled/active, resolved dummy-link settings + `~own` conflicts, trust fingerprint + bundle, sidecar presence/version/digest, selected-address activity and interface coverage, and port conflicts.

## Constraints

- Mirror `macos::HostIntegrationInspector`: one `inspect(&Settings) -> Vec<Diagnostic>` entry point, small `inspect_*` helpers, injected command runner (`windows::setup::CommandRunner`, already platform-neutral and reused by `linux::setup`) so unit tests run everywhere without root.
- File reads go through a `system_root: Option<PathBuf>` override (`None` = real `/`; tests point at a temp tree), mirroring `LinuxHostInstaller::with_system_root`. Fixed paths otherwise: `/etc/own-domains/nftables.conf`, both unit files, `/usr/local/share/ca-certificates/own-domains.crt`.
- `LinuxRuntimeBackend::diagnose` (skeleton, `runtime.rs`) stays manifest-gate-only: `tests/linux_platform.rs` pins it on every host. Only `LinuxLiveBackend::diagnose` (`backend.rs`) runs the inspector after the gate passes.
- Reuse existing constructors and validators: `parse_linux_manifest`, `render_nftables`, `render_nftables_unit`, `render_resolver_unit`, `certificate_der`/`certificate_sha256`, `discover_covered_interfaces`, `SidecarManifest`. No new manifest schema, no new diagnostic transport.
- No CLI changes (`doctor` stays a stub; wiring it to live diagnose is D2). No firewall/Docker/UFW detection (deferred to doctor work, as in C2). No macOS/Windows output changes.

## Out of Scope

D2 (native `doctor` surfacing live diagnostics), Docker/UFW/firewalld policy detection, trust-bundle repair actions, `.deb`/AppImage packaging.

## Verification Commands

All from `/Users/kien/workspace/domains` with `export PATH=$HOME/.cargo/bin:$PATH`:

- test (host, per task): `cargo test -p own-domains-daemon --test linux_inspector_unit`
- full suite: `cargo test -p own-domains-daemon -p own-domains-cli`
- lint: `cargo clippy -p own-domains-daemon -p own-domains-cli --all-targets -- -D warnings`
- fmt: `cargo fmt --all -- --check`
- targets: `cargo check -p own-domains-daemon -p own-domains-cli --all-targets` for `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`
- VM: `cargo test -p own-domains-daemon --test linux_inspector_e2e` (or a `sudo` scenario run; per task)

### Task 1: Inspector skeleton with manifest + artifact file checks

Files:
- create `crates/own-domains-daemon/src/platform/linux/inspector.rs` (`LinuxHostInspector::with_parts`, `inspect` returning `Vec<Diagnostic>`)
- modify `crates/own-domains-daemon/src/platform/linux/mod.rs` (`pub mod inspector`)
- create `crates/own-domains-daemon/tests/linux_inspector_unit.rs` (fake runner + temp sysroot harness)

Steps:
1. Write test: a sysroot containing the four artifact files with manifest-matching bytes plus a valid `install-manifest.json` in a temp state dir yields zero diagnostics; a manifest whose artifact sha256 does not match the file bytes yields an `artifactMismatch` diagnostic naming the path; a missing artifact file yields `artifactMissing`.
2. Run `cargo test -p own-domains-daemon --test linux_inspector_unit`; confirm it fails (`linux::inspector` does not exist).
3. Implement: parse + validate the manifest (`parse_linux_manifest`; invalid → single `setupManifestInvalid` diagnostic and stop); for each manifest artifact read the file under `system_root`, compare bytes/hash, report `artifactMissing` / `artifactMismatch` (mode/owner checks Unix-only via metadata, codes `artifactMode` / `artifactOwner`).
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-daemon --test linux_platform`; confirm green.

Done when: manifest + file checks are fake-covered and the existing suite is green.

### Task 2: nftables ruleset + active table checks

Files:
- modify `crates/own-domains-daemon/src/platform/linux/inspector.rs`
- modify `crates/own-domains-daemon/tests/linux_inspector_unit.rs`

Steps:
1. Write test: ruleset bytes differing from `render_nftables(&manifest.interfaces, manifest.user_id)` yield `nftablesMismatch`; `nft list table ip own_domains` output missing the `prerouting` chain yields `nftablesTableMissing`; a failing `nft` invocation yields `nftablesUnreadable` (not a panic).
2. Run; confirm it fails (no nftables checks).
3. Implement `inspect_nftables`: byte-compare file vs re-render; run `nft list table ip own_domains`, require both chain names (`prerouting` and the forward/output chain names used by the renderer) in stdout.
4. Run the test; confirm it passes.

Done when: ruleset drift and a missing/inaccessible table are fake-covered.

### Task 3: systemd unit checks

Files:
- modify `crates/own-domains-daemon/src/platform/linux/inspector.rs`
- modify `crates/own-domains-daemon/tests/linux_inspector_unit.rs`

Steps:
1. Write test: unit file bytes differing from the corresponding `render_*_unit()` yield `unitMismatch` naming the unit; `systemctl is-enabled <unit>` reporting anything but `enabled` yields `unitDisabled`; `systemctl is-active <unit>` reporting anything but `active` yields `unitInactive`.
2. Run; confirm it fails.
3. Implement `inspect_units` for both unit names imported from `systemd.rs` (file compare + the two `systemctl` probes; treat runner errors as `unitUnknown` with the command output truncated).
4. Run the test; confirm it passes.

Done when: both units are content- and state-checked with fakes.

### Task 4: resolved dummy-link + `~own` conflict checks

Files:
- modify `crates/own-domains-daemon/src/platform/linux/inspector.rs`
- modify `crates/own-domains-daemon/tests/linux_inspector_unit.rs`

Steps:
1. Write test with canned `resolvectl` outputs: `resolvectl dns own-domains0` without `127.0.0.1:5354` yields `resolverDnsMismatch`; `domain` without `~own` yields `resolverDomainMismatch`; `default-route` reporting `yes` yields `resolverDefaultRoute`; `dnssec`/`dnsovertls` other than `no` yield `resolverHardeningMismatch`; a second link whose `domain` output contains `~own` yields `ownDomainConflict` naming that link.
2. Run; confirm it fails.
3. Implement `inspect_resolved`: run the six fixed `resolvectl` queries for `own-domains0` (import `RESOLVER_LINK` from `manifest.rs`); parse `ip -j link`-style? No — discover other links via `resolvectl --no-pager link` list + per-link `domain` query, capped at the manifest interface count + 8 to bound subprocess calls.
4. Run the test; confirm it passes.

Done when: link settings and cross-link `~own` conflicts are fake-covered.

### Task 5: trust checks

Files:
- modify `crates/own-domains-daemon/src/platform/linux/inspector.rs`
- modify `crates/own-domains-daemon/tests/linux_inspector_unit.rs`

Steps:
1. Write test: installed cert whose DER SHA-256 differs from `manifest.ca_fingerprint_sha256` yields `caFingerprintMismatch` (file kept, no removal — inspect only); `openssl x509 -in <cert> -noout -fingerprint -sha256` output not containing the manifest fingerprint (case-insensitive, colon-tolerant) yields `caUntrusted`.
2. Run; confirm it fails.
3. Implement `inspect_trust` with `certificate_der`/`certificate_sha256` and the single fixed `openssl` invocation.
4. Run the test; confirm it passes.

Done when: fingerprint and bundle trust are fake-covered without mutating anything.

### Task 6: sidecar checks

Files:
- modify `crates/own-domains-daemon/src/platform/linux/inspector.rs`
- modify `crates/own-domains-daemon/tests/linux_inspector_unit.rs`

Steps:
1. Write test: missing `coredns` binary yields `sidecarMissing`; `--version` output not containing the manifest version yields `sidecarVersionMismatch`; bytes whose SHA-256 differs from the manifest digest yield `sidecarDigestMismatch`. Same triple for `caddy` (`version` subcommand).
2. Run; confirm it fails.
3. Implement `inspect_sidecars` taking the two exe paths (constructor parameters `coredns_exe`, `caddy_exe`: the daemon factory passes `LinuxPaths` values; tests pass temp files).
4. Run the test; confirm it passes.

Done when: presence, version, and digest are fake-covered for both sidecars.

### Task 7: address activity, coverage, and port conflicts

Files:
- modify `crates/own-domains-daemon/src/platform/linux/inspector.rs`
- modify `crates/own-domains-daemon/tests/linux_inspector_unit.rs`

Steps:
1. Write test with canned `ip -j addr` output: settings LAN not present on any interface yields `lanAddressInactive`; LAN present on an interface absent from `manifest.interfaces` yields `interfaceCoverageMissing`; canned `ss -H -ltn` output showing a listener on `0.0.0.0:8080` yields `portConflict` naming the port; the expected resolved stub listener on `127.0.0.1:53` is ignored.
2. Run; confirm it fails.
3. Implement `inspect_network`: parse `ip -j addr` via `discover_covered_interfaces`-adjacent logic (reuse `LinuxInterface.primary_ipv4`; an eligible interface need not be covered to be active); parse `ss -H -ltn` lines for `LISTEN` entries on ports 53/5354/8080/8443/4781, ignoring `127.0.0.1:53` (resolved stub) and listeners owned by the current runtime (PID match is best-effort; unknown PIDs still conflict).
4. Run the test; confirm it passes.

Done when: activity, coverage, and port conflicts are fake-covered.

### Task 8: Wire the inspector into live-backend `diagnose`

Files:
- modify `crates/own-domains-daemon/src/platform/linux/backend.rs` (`LinuxLiveBackend` holds a `LinuxHostInspector`; `diagnose` runs the manifest gate, then the inspector with `settings`)
- modify `crates/own-domains-daemon/tests/linux_backend_unit.rs` (factory tests unchanged; add healthy-manifest live-backend case)

Steps:
1. Write test: a live backend built over a temp state dir with a valid manifest but an empty sysroot reports diagnostics (e.g. `artifactMissing`), proving the inspector runs post-gate; the existing no-manifest case still reports only `setupRequired`.
2. Run; confirm it fails (live `diagnose` ignores settings).
3. Implement: factory builds the inspector from `resolve_paths` (sidecar exes) + the same `system_root` derivation (bundle-aware? The installer root override is test-only; the backend factory has no root knob — add `LinuxLiveBackend::with_inspector` for tests and default the factory to real `/`); skeleton `runtime.rs` untouched.
4. Run `cargo test -p own-domains-daemon --test linux_backend_unit --test linux_platform --test linux_inspector_unit`; confirm green.

Done when: live `diagnose` = gate + inspector, skeleton and existing tests untouched.

### Task 9: VM verification (privileged)

Files: none (VM execution; fix fallout in `inspector.rs` or tests as found).

Steps:
1. On the Ubuntu VM, pull `main`, build, run a real `sudo -E setup --apply`, then exercise `diagnose` over the live backend (via a small `cfg(linux)` scenario test or the existing e2e binary with a diagnose assertion — add `tests/linux_inspector_e2e.rs` only if a scenario file proves cleaner than extending `linux_backend_e2e.rs`; prefer extending the existing e2e with a post-apply `diagnose().is_empty()` assertion plus a stopped-runtime negative case).
2. Confirm: healthy system → empty diagnostics; stop a unit (`systemctl stop`) → `unitInactive`; break nothing else.
3. Paste all output.

Done when: the inspector passes against the real host integration with evidence.

### Task 10: Full verification and review

Files: none (verification only).

Steps:
1. Host: `cargo test -p own-domains-daemon -p own-domains-cli`; report results.
2. `cargo clippy -p own-domains-daemon -p own-domains-cli --all-targets -- -D warnings`, three target checks, `cargo fmt --all -- --check`, `pnpm test`; report results.
3. `git diff --stat`; present for review. Do not merge, do not push; merging is the user's decision.

Done when: all checks reported with real output and the diff is up for review.
"
