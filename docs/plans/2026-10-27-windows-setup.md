# Windows Setup/Uninstall Plan (release slice 2)

## Goal

`setup --apply`, `setup` (preview), `uninstall --apply`, and `uninstall`
(preview) work on Windows: NRPT rule for `.own`, per-user trust install,
LAN-listener firewall handling, all reversible and previewable.

## Constraints

- No protocol/core changes; `HostInstaller` trait is frozen — Windows
  actions map onto `SetupAction{kind,path,backup}` where `path` is a
  receipt file under `<state_dir>/applied/` (`nrpt.json`, `trust.json`,
  `firewall.json`). Apply performs the system mutation then writes the
  receipt; uninstall reverses from receipts.
- All system effects go through a `CommandRunner` seam (fake in tests);
  no real mutation in host tests.
- Trust writes require the interactive user session (Gate G proved
  non-interactive writes fail); the installer returns an actionable
  failure there instead of attempting.
- Firewall strategy: installer adds an inbound rule for the LAN listener
  during setup and removes it on uninstall (prompt-avoidance beats
  documenting the prompt).

## Out of Scope

Facade (done, slice 1), manifest/fetch (slice 3), Tauri bundle (slice 4),
version bump/release checks (slice 5), Job Objects, signing.

## Verification Commands

- test: `cargo test -p own-domains-daemon --test windows_setup_unit` (host)
- test: `cargo test -p own-domains-cli` (host)
- lint: both clippy commands with `-- -D warnings`
- build: `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc`

### Task 1: Installer skeleton with runner seam and receipt model

Files:
- create `crates/own-domains-daemon/src/platform/windows/setup.rs`
- modify `crates/own-domains-daemon/src/platform/windows/mod.rs`
- create `crates/own-domains-daemon/tests/windows_setup_unit.rs` (runs everywhere)

Steps:
1. Write test: `WindowsHostInstaller` with a fake runner —
   `preview_setup` lists `nrpt.json` + `trust.json` + `firewall.json`
   receipt actions without running anything; `apply_setup` runs the
   three mutations then writes the receipts; second `apply_setup` reports
   unchanged; `preview_uninstall` lists removals; `apply_uninstall`
   reverses and deletes receipts.
2. Run the test; confirm it fails (no module).
3. Implement: `CommandRunner` trait + `SystemCommandRunner`,
   `WindowsHostInstaller` with the receipt flow above, mutations as
   small private fns returning `Err("not yet implemented: …")` — tasks
   2–4 fill them.
4. Run the new test plus `windows_backend_unit`; confirm green.
5. Run host clippy; confirm clean.

Done when: receipt lifecycle is pinned and each mutation is one stub call.

### Task 2: NRPT add/remove through the runner

Files:
- modify `crates/own-domains-daemon/src/platform/windows/setup.rs`
- modify `crates/own-domains-daemon/tests/windows_setup_unit.rs`

Steps:
1. Write test: fake runner captures argv; apply issues
   `powershell -NoProfile -Command Add-DnsClientNrptRule -Namespace .own
   -NameServers 127.0.0.1` (exact argv pinned); uninstall issues the
   matching `Remove-DnsClientNrptRule` by namespace; runner failure
   surfaces as `SetupFailure` with the stderr text, no receipt written.
2. Run; confirm it fails (stub error).
3. Implement the two fns with `Get-DnsClientNrptRule` idempotency check
   before add.
4. Run tests; confirm green. Run host clippy.

Done when: NRPT argv and failure behavior are pinned without touching a machine.

### Task 3: Per-user trust install/remove with session guard

Files: same as Task 2.

Steps:
1. Write test: apply issues `certutil -addstore -user Root <cer>` and
   records the thumbprint in `trust.json`; uninstall issues
   `certutil -delstore -user Root <thumbprint>`; a non-interactive
   session marker makes apply fail with "run setup from the interactive
   console" and writes no receipt.
2. Run; confirm it fails.
3. Implement: session check via `GetProcessWindowStation` NULL test
   (or `WTSSessionInfo` — exact API chosen at implementation, kept
   behind the runner seam where possible); a missing `ca.crt` is
   generated in place (`New-SelfSignedCertificate` + `Export-Certificate`,
   thumbprint from stdout, `generated` recorded in the receipt).
4. Run tests; confirm green. Run host clippy.

Done when: trust lifecycle and the interactive-session guard are pinned.

### Task 4: Firewall rule add/remove

Files: same as Task 2.

Steps:
1. Write test: apply issues
   `netsh advfirewall firewall add rule name=OwnDomains-LAN …` for the
   configured LAN TCP 80/443 (exact argv pinned); uninstall issues the
   matching `delete rule`; runner failure → `SetupFailure`, no receipt.
2. Run; confirm it fails.
3. Implement with existence check before add.
4. Run tests; confirm green. Run host clippy.

Done when: firewall argv and failure behavior are pinned.

### Task 5: CLI wiring and elevation gate

Files:
- create `crates/own-domains-cli/src/platform/windows.rs`
- modify `crates/own-domains-cli/src/platform/mod.rs`
- modify `crates/own-domains-cli/src/command.rs`

Steps:
1. Write test (host-runnable): `host_installer()` on the test double —
   extend the existing `platform::tests` with a Windows-equivalent
   assertion that the installer is not the unsupported one when compiled
   for Windows (cfg-gated, compiles everywhere, runs on VM).
2. Run; confirm the Windows branch is missing.
3. Implement: `windows::WindowsHostInstaller` delegating to the daemon
   installer; `host_installer()` returns it on Windows; `execute_setup`
   / `execute_uninstall` accept Windows (keep macOS exactly as-is);
   elevation gate: `--apply` without an elevated token fails with
   "run from an elevated prompt", preview never requires elevation.
4. Run `cargo test -p own-domains-cli` on host; confirm green. Run the
   Windows target check; confirm clean.

Done when: CLI routes to the real installer on Windows with preview
unelevated and apply gated.

### Task 6: VM end-to-end runbook (elevated)

Files: none (runbook only).

Steps:
1. Pull the branch on the ARM64 VM; `setup` (preview) lists the three
   receipts without changes.
2. Elevated `setup --apply --lan-address <guest-IP>`: NRPT rule present
   (`Get-DnsClientNrptRule -Namespace .own`), cert in per-user Root,
   firewall rule present; `start` serves traffic.
3. `stop`, elevated `uninstall --apply`: rule, cert, and firewall entry
   gone; receipts deleted; re-run `setup` preview shows a clean slate.
4. Paste all outputs.

Done when: install → serve → uninstall leaves no trace.

### Task 7: Full verification and commit

Steps:
1. `cargo test --workspace --locked` green on host.
2. Both clippies, fmt, `git diff --check`, all three target checks clean.
3. VM: `cargo test -p own-domains-daemon --test windows_setup_unit` green.
4. Commit `feat:` + push `feat/windows-release`.

Done when: slice 2 is pushed with host + VM evidence.
