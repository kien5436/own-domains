# Changelog

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
