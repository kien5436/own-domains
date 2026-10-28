# Changelog

## 0.1.0-beta.3 - 2026-10-28

First beta with Windows host support (Windows 10/11, x64), alongside the native macOS app.

### Added

- Windows setup, doctor, and uninstall through a manifest-owned NRPT rule for `.own`, a LAN firewall rule, and a per-user Root CA certificate. Setup and uninstall are fully reversible.
- Windows backend serving `.own` over CoreDNS and Caddy with a loopback-only admin endpoint, verified end to end on x64 hardware with real sidecars.
- Unsigned NSIS installer for x64 Windows (SmartScreen warning expected; WebView2 required).

### Fixed

- Setup persists the selected LAN address so a later start serves it without further privileged steps.
- Uninstall removes the installed CA certificate instead of leaving it trusted.

### Known limitations

- No Developer ID-signed or notarized binary is attached; the Windows installer is unsigned.
- The legacy TypeScript test suite is macOS/Linux-only (file modes, shell scripts, dnsmasq/launchd fixtures); Windows is validated through the Rust suites and the setup runbook instead.
- Encrypted-DNS coexistence review stays deferred (see PLAN.md).

## 0.1.0-beta.2 - 2026-10-26

Second beta of the native macOS app. This release makes the privileged host integration independent of the current LAN address.

### Changed

- macOS setup now discovers every eligible LAN interface and installs dynamic `(if:0)` PF redirect rules for all of them. The install manifest uses schema 2 and records interface names instead of a LAN address.
- Changing the selected LAN address or switching between covered interfaces no longer requires privileged setup. Newly attached hardware interfaces may require one additional setup.
- The GUI network-settings save path reports interface-coverage and inactive-address diagnostics instead of host-integration drift.
- An existing schema 1 install is migrated to schema 2 by running setup once.

### Fixed

- PF loopback exemptions prevent macOS redirect state from misdirecting direct high-port traffic.
- Invalid or tampered install manifests report a manual-repair remedy instead of an unusable automatic action.
- Uninstall removes the installed Caddy root certificate without requiring an interactive trust-settings prompt, and skips removal when the certificate is already absent.

## 0.1.0-beta.1 - 2026-10-22

First beta of the native macOS app.

### Added

- Native Tauri desktop client with a restricted daemon IPC bridge.
- Rust daemon, CLI, and version-2 IPC protocol.
- Bundled CoreDNS and Caddy sidecars with digest verification.
- Privileged macOS setup for the `.own` resolver, PF redirect, and Caddy root certificate.
- Optional HTTPS and LAN device onboarding through `setup.own`.
- Reversible uninstall that restores host resolver and PF state.
- macOS lifecycle integration: tray restore, single-instance focus, hide-on-close, and stop-before-quit.

### Fixed

- The Register button keeps its label when unrelated daemon actions are running. Its submission state is now local to the form.

### Known limitations

- This is a source release; no Developer ID-signed or notarized binary is attached.
- Apple silicon macOS 15+ is the acceptance target. Intel macOS is build-only.
- Ubuntu 24+ and Windows 10/11 runtime adapters are not implemented.
- The legacy TypeScript runtime remains available for existing installations.
- No CI workflow is configured; release checks were run locally.
