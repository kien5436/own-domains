# Platform support

Own Domains ships native runtime backends for macOS, Ubuntu, and Windows. A platform is not supported until its adapter exists and its acceptance evidence is recorded; buildability alone does not imply runtime support.

| Platform | Native runtime status | Acceptance |
| --- | --- | --- |
| Apple silicon macOS 15+ | Implemented; first beta | Passed 2026-10-22: ad-hoc-signed arm64 app and CLI, automated runner, GUI checks, and one physical-phone pass |
| Intel macOS 15+ | Buildable; runtime not acceptance-tested | Not required |
| Ubuntu 24+ | Implemented; install via `.deb`, then `setup` | Covered by `linux_backend_e2e` plus `docs/linux-install.md` |
| Windows 10/11 | Implemented; beta.3 unsigned NSIS installer (x64; ARM64 runs the x64 build emulated) | Recorded in `docs/windows-acceptance.md` |

## Host integration boundary

The daemon exposes platform-neutral `RuntimeBackend`, `HostInspector`, and `HostInstaller` contracts. The CLI selects an installer through its platform module at compile time. macOS PF, resolver, trust-store, and install-manifest details remain under the macOS implementation. Future platform adapters must implement these contracts and provide their own privileged artifacts and acceptance evidence; they must not add platform branches to shared command or core code. On macOS, the privileged anchor and schema 2 install manifest contain no selected LAN address. Address changes and switching between covered interfaces stay unprivileged; only initial setup and covering a newly appeared interface require root.

## macOS signing

Local acceptance uses ad-hoc signing:

```bash
codesign --force --deep --sign - "Own Domains.app"
codesign --verify --deep --strict "Own Domains.app"
```

Ad-hoc signing requires no Apple Developer Program account. `codesign --verify` must pass for the app, CLI, and sidecars. The upstream CoreDNS and Caddy sidecars are already ad-hoc signed; do not re-sign them, because changing their signature bytes also changes the SHA-256 digest pinned in the manifest. Sign the app and CLI, then verify the sidecars.

Developer ID signing, notarization, and Gatekeeper `spctl` assessment are distribution requirements only. They require a paid Apple Developer Program account and are not part of local acceptance. `pnpm run release:check` is the all-architecture Developer ID distribution check; the local acceptance path does not run it.

## Intel macOS

Intel macOS is buildable with `pnpm run desktop:build:x86_64`. It is not acceptance-required. The macOS runtime backend is shared with Apple silicon, but no Intel acceptance log is required by this support policy.

## Ubuntu and Windows

Windows 10/11 is supported as of beta.3 (see `docs/windows-acceptance.md`).
Ubuntu 24+ ships a working runtime: the `LinuxLiveBackend` renders configs,
spawns both sidecars, and passes the end-to-end backend suite
(`linux_backend_e2e`); Milestone 1 rendering records remain in
`docs/linux-acceptance.md` as history, and the install walkthrough is in
`docs/linux-install.md`. Each platform keeps its own architecture
design, implementation plan, and acceptance record before this matrix
claims runtime support.
