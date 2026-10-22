# Platform support

Own Domains has one native runtime backend today: macOS. A platform is not supported until its adapter exists and its acceptance evidence is recorded; buildability alone does not imply runtime support.

| Platform | Native runtime status | Acceptance |
| --- | --- | --- |
| Apple silicon macOS 15+ | Implemented; first beta | Passed 2026-10-22: ad-hoc-signed arm64 app and CLI, automated runner, GUI checks, and one physical-phone pass |
| Intel macOS 15+ | Buildable; runtime not acceptance-tested | Not required |
| Ubuntu 24+ | Support target; returns `runtimeUnavailable` | Separate design, implementation plan, and acceptance required |
| Windows 10/11 | Support target; IPC and runtime are not implemented | Separate design, implementation plan, and acceptance required |

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

Ubuntu 24+ and Windows 10/11 are implementation targets, not current runtime support. Linux and Windows return `runtimeUnavailable`; Windows IPC is also unimplemented. Each platform needs a separate architecture design, implementation plan, and acceptance record before this matrix can claim runtime support.
