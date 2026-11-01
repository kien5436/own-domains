# Linux Setup/Uninstall C2 Plan

## Goal

Privileged Linux setup/uninstall behind the shared `HostInstaller` contract: `LinuxHostInstaller` with receipt-based apply (`nftables.json`, `resolver-unit.json`, `nftables-unit.json`, `trust.json` + `install-manifest.json`), idempotent re-apply, reverse-order uninstall with fingerprint verification, rollback journal semantics, and CLI wiring (`setup`/`uninstall` gates + `host_installer` on Linux). Verified with fake-runner unit tests on host and a real `sudo` run on the Ubuntu VM.

## Constraints

- Mirror `WindowsHostInstaller` structure: injected `CommandRunner` + `RunningProcess` traits (reuse them directly — they are platform-neutral), `apply_receipt`/`remove_*_checked` helpers, `ensure_applied_dir`, `persist_lan_address`. No new production dependencies.
- Four host artifacts only (fixed allowlist): `/etc/own-domains/nftables.conf`, `/etc/systemd/system/own-domains-nftables.service`, `/etc/systemd/system/own-domains-resolver.service`, `/usr/local/share/ca-certificates/own-domains.crt`. Uninstall validates every path against this allowlist, never the manifest list alone.
- Receipts live under `<state>/applied/` (`nftables.json`, `units.json`, `trust.json`); the install manifest (`install-manifest.json` via `build_linux_manifest` + `render_linux_manifest`) is written last on apply and removed last on uninstall. Reuse `build_linux_manifest`/`render_linux_manifest`/`parse_linux_manifest` — no new manifest code.
- CA: reuse the Caddy CA in the target user's state dir. The CLI resolves the staged Caddy exactly like the daemon (`linux::backend::resolve_paths` + `bundled_resources_dir`); generation fails with an actionable message when no Caddy is staged. Trust apply runs `update-ca-certificates`; uninstall verifies the DER fingerprint before removing and reports `caFingerprintMismatch` on conflict.
- Interface handling per design: validate the requested interface is eligible and carries `lan_address` as primary IPv4; discover all eligible interfaces at setup and store their names; address changes stay unprivileged (persist `lan_address` in settings only).
- Elevation: `sudo` only. CLI gates `--apply` on euid 0; installer resolves the target user from `SUDO_USER`/`SUDO_UID` and refuses when unresolvable. SSH sessions are allowed for setup (unlike Windows per-user trust) — root writes system paths either way.
- No macOS/Windows output changes: new code activates only under `target_os = "linux"` CLI gates and the new installer.
- Preview is read-only: no state-dir creation, no CA generation, no writes, no unit/nft/trust changes.

## Out of Scope

Exhaustive preflight (Docker publications, UFW/firewalld policy detection, `nftables.service` flush-ruleset warning, global `~own` conflicts) — implement the essential gates (root, sudo user, interface eligibility, required binaries, `nft -c` ruleset check) and defer the rest to doctor work. `HostIntegrationInspector` diagnostics (`lanAddressInactive`, `interfaceCoverageMissing`, port conflicts) — separate inspector milestone. `.deb`/AppImage packaging.

## Verification Commands

- test (host, per task): `cargo test -p own-domains-daemon --test linux_setup_unit` and `cargo test -p own-domains-cli` from `/Users/kien/workspace/domains`
- lint: `cargo clippy -p own-domains-daemon -p own-domains-cli --all-targets -- -D warnings`
- fmt: `cargo fmt --all -- --check`
- targets: `cargo check -p own-domains-daemon -p own-domains-cli --all-targets` for `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`
- VM: `sudo -E own-domains setup` (preview), `sudo -E own-domains setup --apply`, re-apply, `sudo -E own-domains uninstall --apply`, real assertions per task

### Task 1: Installer skeleton with preview + receipt helpers, fake-tested

Files:
- create `crates/own-domains-daemon/src/platform/linux/setup.rs` (`LinuxHostInstaller` with injected `CommandRunner`, `preview_setup`, `preview_uninstall`, `apply_receipt`/`write_receipt`/`read_receipt`, `ensure_applied_dir`, `persist_lan_address` mirroring Windows)
- modify `crates/own-domains-daemon/src/platform/linux/mod.rs` (`pub mod setup`)
- create `crates/own-domains-daemon/tests/linux_setup_unit.rs` (fake runner mirroring `windows_setup_unit.rs`)

Steps:
1. Write test: preview lists create/update actions for the four artifact paths + manifest without invoking the runner; preview_uninstall with no receipts yields no actions.
2. Run `cargo test -p own-domains-daemon --test linux_setup_unit`; confirm it fails (no `linux::setup` module).
3. Implement the skeleton: `LinuxHostInstaller::with_parts(runner, caddy_exe)`, receipt dir helpers, preview actions with kinds `linux-nftables`, `linux-nftables-unit`, `linux-resolver-unit`, `linux-trust`, `linux-manifest`.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-daemon --test linux_platform`; confirm green.

Done when: preview is pinned by tests and the runner is never touched by preview.

### Task 2: nftables + systemd units apply/uninstall with rollback

Files:
- modify `crates/own-domains-daemon/src/platform/linux/setup.rs`
- modify `crates/own-domains-daemon/tests/linux_setup_unit.rs`

Steps:
1. Write test: apply writes nftables.conf content (rendered via `render_nftables` over the covered interfaces), both unit files (via `render_nftables_unit`/`render_resolver_unit`), runs `nft -c -f`, `systemctl daemon-reload`, enable+start both units; receipts record path + sha256; second apply is unchanged (no runner calls); uninstall stops/disables units, deletes table via `nft delete table ip own_domains`, removes files, daemon-reload, deletes receipts; missing artifact on uninstall is already-removed (no error).
2. Run; confirm it fails (no apply logic).
3. Implement: `apply_nftables` (write root-owned 0644 via temp+rename — atomic write helper; `nft -c -f` validate, enable/start unit), `apply_units`, rollback on failure in reverse order (disable/stop newly enabled units, delete newly loaded table, remove new files), `remove_*_checked` mirroring Windows.
4. Run the test; confirm it passes.
5. Run full host daemon suite; confirm green.

Done when: unit + nftables lifecycle is fake-covered including rollback and idempotent re-apply.

### Task 3: Trust apply/uninstall with fingerprint verification

Files:
- modify `crates/own-domains-daemon/src/platform/linux/setup.rs`
- modify `crates/own-domains-daemon/tests/linux_setup_unit.rs`

Steps:
1. Write test: apply copies the CA cert (from state `caddy-data/.../root.crt`, ensured via the Caddy-CA helper with staged Caddy) to `/usr/local/share/ca-certificates/own-domains.crt`, runs `update-ca-certificates`, receipt holds the lowercase DER SHA-256 (`certificate_sha256`); uninstall recomputes the fingerprint of the installed file and removes it only on match, then reruns `update-ca-certificates`; fingerprint mismatch leaves the file and fails with `caFingerprintMismatch`.
2. Run; confirm it fails (no trust logic).
3. Implement: `ensure_caddy_ca` adapted from Windows (staged Caddy `run --config` with PKI-only config, bounded wait for all four files, half-written CA dropped); `apply_trust`/`remove_trust` with `certificate_der`/`certificate_sha256` verification.
4. Run the test; confirm it passes.

Done when: trust install/remove is fake-covered with fingerprint gating.

### Task 4: Manifest write-last / remove-last + interface validation

Files:
- modify `crates/own-domains-daemon/src/platform/linux/setup.rs`
- modify `crates/own-domains-daemon/tests/linux_setup_unit.rs`

Steps:
1. Write test: apply validates requested interface eligible + carries lan_address (fake `ip -j` output through the runner), discovers covered set, writes `install-manifest.json` (mode 0600, target-user owned) only after all receipts succeed; manifest parse round-trips through `parse_linux_manifest`; uninstall removes the manifest only after all host artifacts are gone; corrupt manifest on uninstall refuses with fixed paths for manual inspection.
2. Run; confirm it fails.
3. Implement: preflight-essential checks (interface eligibility via `validate_covered_interfaces`, lan-on-interface via `discover_covered_interfaces` over runner `ip` output), manifest build with sidecar digests (read from staged `manifest.json` files), write-last/remove-last ordering.
4. Run the test; confirm it passes.

Done when: manifest lifecycle and interface validation are fake-covered.

### Task 5: Essential preflight gates

Files:
- modify `crates/own-domains-daemon/src/platform/linux/setup.rs`
- modify `crates/own-domains-daemon/tests/linux_setup_unit.rs`

Steps:
1. Write test: apply fails before any mutation when: not root (injectable uid probe), `SUDO_USER`/`SUDO_UID` unresolvable, state dir not absolute or not target-owned, required binaries missing (`/usr/sbin/ip`, `/usr/sbin/nft`, `/usr/bin/resolvectl`, `/usr/bin/systemctl`, `/usr/sbin/update-ca-certificates`, `/usr/bin/openssl`, `/usr/bin/ss`), `nft -c -f` rejects the ruleset, foreign file at an owned path. Each failure leaves no receipts and no runner mutations beyond inspection.
2. Run; confirm it fails.
3. Implement the preflight gate as the first apply step.
4. Run the test; confirm it passes.

Done when: every essential preflight failure is fake-covered with zero-mutation guarantee.

### Task 6: Root-owned atomic writes + backup/rollback journal

Files:
- modify `crates/own-domains-daemon/src/platform/linux/setup.rs`
- modify `crates/own-domains-daemon/tests/linux_setup_unit.rs`

Steps:
1. Write test: owned existing files that change are backed up under the root-owned backup dir preserving owner/mode; rollback restores them (revalidating owner/mode/hash); backup-dir creation as `root:root` 0700; rollback failure lists the item in `SetupFailure.unresolved` and keeps the manifest so uninstall can resume.
2. Run; confirm it fails.
3. Implement temp-file + flush + atomic rename under root-owned dirs, journal of written files/backups, reverse-order rollback per design (units → table → link → trust → files → daemon-reload).
4. Run the test; confirm it passes.

Done when: backup/rollback semantics are fake-covered including unresolved-item reporting.

### Task 7: CLI wiring — Linux platform module, gates, installer selection

Files:
- create `crates/own-domains-cli/src/platform/linux.rs` (`is_root` via euid, `staged_caddy_exe` mirroring `windows.rs` through `linux::backend::resolve_paths`)
- modify `crates/own-domains-cli/src/platform/mod.rs` (linux branch returning `LinuxHostInstaller::with_caddy`)
- modify `crates/own-domains-cli/src/command.rs` (extend the two platform guards to include linux; `--apply` requires root with actionable message)

Steps:
1. Write test: on any host, `staged_caddy_exe` returns `None` with no daemon env (no guessing); gate test asserts `execute_setup`/`execute_uninstall` no longer return "unsupported on this platform" for linux (via `#[cfg]`-aware unit or by extracting the guard into a testable fn).
2. Run `cargo test -p own-domains-cli`; confirm it fails.
3. Implement the module + guard changes (minimal: `|| !cfg!(target_os = "linux")` extensions, root check `#[cfg(target_os = "linux")]`).
4. Run CLI tests + clippy; confirm green.

Done when: CLI selects the Linux installer and gates `--apply` on root without touching macOS/Windows paths.

### Task 8: Ubuntu VM end-to-end (sudo)

Files: none (VM execution; fix fallout in `setup.rs` or tests as found).

Steps:
1. On the VM: `own-domains setup` preview — assert the four artifacts + manifest listed, no mutations (`ls /etc/own-domains`, `systemctl is-enabled`, state dir untouched).
2. `sudo -E own-domains setup --apply --lan-address <guest-ip> --interface <iface>` — assert receipts + manifest, `nft list table ip own_domains` shows chains, units enabled+active, `resolvectl dns own-domains0` → `127.0.0.1:5354`, `resolvectl domain` → `~own`, cert in system bundle.
3. Re-apply — assert unchanged, then `start` the runtime (C1 backend) and confirm `.own` resolves + app serves.
4. `sudo -E own-domains uninstall` preview, then `--apply` — assert units removed/disabled, table gone, cert untrusted, manifest gone, state preserved; `--purge` variant removes state.
5. Paste all outputs.

Done when: real privileged setup → runtime → uninstall round-trips on the VM with evidence.

### Task 9: Full verification and review

Files: none (verification only).

Steps:
1. Host: `cargo test -p own-domains-daemon -p own-domains-cli`; report results.
2. `cargo clippy -p own-domains-daemon -p own-domains-cli --all-targets -- -D warnings`, three target checks, `cargo fmt --all -- --check`; report results.
3. `git diff --stat`; present for review. Do not merge; do not touch main.

Done when: all checks reported with real output and the diff is up for review.
"
