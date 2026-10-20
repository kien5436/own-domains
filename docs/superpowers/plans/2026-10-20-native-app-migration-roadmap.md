# Native Application Migration Roadmap

**Spec:** `docs/superpowers/specs/2026-10-20-native-app-architecture-design.md`

The native migration is split into four implementation plans. Each phase must leave independently testable software and preserve the working TypeScript implementation until its replacement reaches parity.

1. **Portable Rust core** — model, validation, compatible settings persistence, serialized mutations, Caddy configuration, and CoreDNS configuration. Plan: `docs/superpowers/plans/2026-10-20-portable-rust-core.md`.
2. **Daemon, IPC, and CLI** — one authoritative writer, local authenticated transport, revisioned event subscriptions, runtime state machine, and the independent `own-domains` executable.
3. **Tauri GUI migration** — reuse the React/Vite/Tailwind frontend, replace HTTP management calls with IPC, add tray/single-instance/hide-on-close behavior, retain `setup.own`, and remove `registry.own`.
4. **Platform activation and release verification** — privileged service integration and complete shutdown on macOS, supported systemd Linux, and Windows 11, followed by physical-device acceptance tests.

Plans 2–4 must be written against the actual interfaces and evidence produced by the preceding phase. This avoids freezing IPC, lifecycle, and privilege decisions before the portable core behavior is executable.
