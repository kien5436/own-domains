# Changelog

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
