# Windows release spec — 0.1.0-beta.3 (GUI + CLI, unsigned)

Date: 2026-10-27. Target: Windows 10/11, x86_64 native + ARM64 (CoreDNS x64
emulated — no ARM64 upstream asset). Version: 0.1.0-beta.3.

## Problem

The Windows backend is implemented and VM-proven, but no user can run it:
the daemon facade returns `runtimeUnavailable` off-macOS, CLI setup answers
"unsupported on this platform", the sidecar manifest is macOS-only, and the
Tauri bundle has no Windows target. This spec closes those four gaps in
order, ending in an installable unsigned release.

## Goals

1. `own-domains-daemon.exe` on Windows drives the real `WindowsRuntimeBackend`
   through the existing `runtime_backend` facade (same `start/status/stop`
   semantics as macOS, `runtimeUnavailable` only when genuinely unavailable).
2. `setup / doctor / uninstall` work on Windows: one-time privileged NRPT +
   prerequisite setup, per-user trust (two dialogs accepted), clean reversible
   uninstall. No recurring elevation.
3. Windows sidecars pinned like macOS: manifest entries, `fetch-sidecars`
   support, `release:preflight` hash verification for both Windows archs
   (CoreDNS x64-only; Caddy x64 + ARM64).
4. Tauri GUI + CLI bundle installs and runs on both archs; version bumped to
   `0.1.0-beta.3` across manifests.
5. Acceptance rerun on ARM64 VM plus full pass on x86_64 hardware before release.

## Non-goals

Authenticode signing (ships unsigned; SmartScreen path documented, not
suppressed). Job-Object supervision (Gate D stays as-is; plain
spawn/terminate). Linux. macOS behavior changes. Auto-updater. MSI
enterprise features (GPO/ADMX).

## Design (slice order; each slice independently verifiable)

1. **Facade wiring.** `runtime_backend` returns the Windows backend on
   Windows: state-dir layout, sidecar paths (bundled resources with
   dev/staged override), admin socket, `ca.crt` placeholder path,
   `settings-only` passthrough. Verified by CLI `start/status/stop` against
   the real runtime on the VM.
2. **Setup / doctor / uninstall.** Mirror the CLI command seams; NRPT
   add/remove by namespace, per-user trust install/remove (interactive
   session only — SSH-proven limitation), prerequisite checks in doctor.
   Firewall strategy for the LAN listener decided in planning (rule vs
   documented prompt). Reversibility asserted by uninstall-then-relist tests.
3. **Sidecars.** Manifest schema gains Windows releases; fetch stages and
   verifies per-arch; `stage-desktop` includes them as Tauri resources.
4. **Bundle.** `tauri.conf.json` Windows resources + bundle target (NSIS
   recommended for unsigned beta; MSI deferred), WebView2 bootstrapper
   decision in planning, existing icons reused.
5. **Release.** Version bump, `release:preflight` green cross-platform,
   `release:check` adjusted (Apple-signing gate must not block Windows;
   unsigned status recorded, not failed).

## Interfaces

No protocol/core changes (v2 / schema 1 frozen). New surface is CLI
behavior on Windows (`setup --apply`, `doctor`, `uninstall --apply`) and
the installer artifact. Tauri frontend unchanged.

## Testing

- Existing suites stay green on host + both Windows archs
  (`cargo test --workspace --locked`, both clippies, fmt).
- New: facade selection test, setup/uninstall reversibility runbook on VM
  (elevated), fresh-install → apply → stop → uninstall cycle per arch.
- x64-gated (queued for hardware arrival): native CoreDNS behavior/perf,
  Job-Object behavioral retest (Gate D unblock attempt), full e2e, reboot
  persistence spot-check.

## Risks

- x86_64 hardware arrives tomorrow: slices 1–4 are host-side and
  ARM64-verifiable, so work starts now; slice 5's x64 half waits.
- WebView2 missing on targets → bundler decision covers bootstrapper.
- LAN-listener firewall prompt → decided in planning, verified on VM.
- Unsigned SmartScreen → documented expectation, not a blocker for beta.
- Emulated-CoreDNS perf on ARM64 → 30 s readiness budget already accounts
  for it; release notes carry the caveat.
