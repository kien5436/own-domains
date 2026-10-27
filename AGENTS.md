# Repository Guidelines

## Overview

Own Domains pairs a legacy TypeScript runtime (React dashboard, dnsmasq DNS, Caddy proxy, `src/cli` setup/doctor/uninstall) with a native Rust/Tauri macOS beta (`crates/`, bundled CoreDNS/Caddy sidecars).

- Native acceptance target is Apple silicon macOS 15+; Intel macOS is buildable but not acceptance-required. Ubuntu 24+ and Windows 10/11 are support targets whose native runtime returns `runtimeUnavailable`.
- Keep the legacy TypeScript runtime and its migration/uninstall support until no beta users depend on it.
- Defer volatile platform, signing, and acceptance detail to `README.md` and `docs/` (`desktop.md`, `platform-support.md`, `native-core.md`, `cli.md`, `macos-acceptance.md`) instead of duplicating it here.

## Project Structure & Module Organization

- `src/web/` — React dashboard, organized by feature (`features/`, `app/`, `lib/`).
- `src/server/` — legacy backend: `domains/`, `http/`, `network/`, `proxy/`.
- `src/cli/` — legacy `setup`, `doctor`, `uninstall` commands and their plan/writer/service splits.
- `src/shared/` — types shared by web and server.
- `crates/own-domains-{core,protocol,daemon,client,cli,desktop}/` — portable Rust core, IPC protocol, daemon, client, CLI, and Tauri desktop app.
- `tests/` — mirrors `src/` paths, plus `e2e/` (Playwright), `integration/` (dnsmasq/Caddy), `fixtures/` (core-parity, IPC, sidecars), and `scripts/`. Rust crate tests live under `crates/*/tests/`.
- `docs/` — architecture, platform support, setup, and acceptance records. `scripts/` — `install.sh`, `uninstall.sh`, sidecar fetch/stage, release checks. `sidecars/` — pinned manifests (binaries under ignored `sidecars/build/`).
- Ignored outputs: `dist/`, `target/`, `sidecars/build/`, `artifacts/`, `coverage/`, `playwright-report/`, `test-results/`, `.worktrees/`.

Prefer small modules with one clear responsibility. Group web code by feature; keep components, hooks, API access, and pure validation in separate focused files.

## Build, Test, and Development Commands

Node.js, React, TypeScript via pnpm; Rust 1.90+ via Cargo:

- `pnpm install` — install locked dependencies.
- `pnpm run dev` — start the Vite dashboard; `pnpm run dev:server` for the local API; `pnpm run desktop:dev` for the Tauri client.
- `pnpm test` — Vitest unit/component/API tests (excludes `tests/e2e/**` and `.worktrees/**`).
- `pnpm run test:e2e` — Playwright browser tests.
- `pnpm run test:native-daemon` — `cargo test --workspace`.
- `pnpm run verify:native-core` — Rust core tests plus `pnpm test`, `lint`, `typecheck`, and `build`.
- `pnpm run lint` — eslint; `pnpm run typecheck` — `tsc -b`; `pnpm run build` — `tsc -b && vite build`.
- `pnpm run sidecars:fetch -- --verify-only` — verify pinned CoreDNS/Caddy digests (omit the flag once to download into `sidecars/build/`).
- `pnpm run desktop:prepare` / `pnpm run desktop:build` — stage daemon and build the desktop app; `:arm64`/`:x86_64` variants for arch-specific macOS builds.
- `pnpm run setup -- --dry-run` — preview privileged host changes; `pnpm run doctor` — check DNS/proxy prerequisites; `pnpm run uninstall` — remove managed services.
- `pnpm run release:preflight` / `pnpm run release:check` — distribution readiness checks.

DNS/Caddy integration tests self-skip when `dnsmasq`, `caddy`, or `dig` are absent. Do not introduce production dependencies without prior approval.

## Coding Style & Naming Conventions

- Follow `eslint-config-pk` through `eslint.config.js`. Use `camelCase` for functions/variables, `PascalCase` for React components and types, lowercase kebab-case for non-component modules. Use Tailwind CSS utilities and shared theme tokens, not large component-specific stylesheets.
- Rust uses edition 2024. Keep shared behavior in `core` and wire contracts in `protocol`; keep platform work behind the daemon/CLI host-adapter contracts and do not add platform branches to shared command or core code.
- Never re-sign the pinned CoreDNS/Caddy sidecars (signature bytes are part of the pinned SHA-256 digest). Never run the legacy TypeScript settings writer against a native state dir — it does not participate in the Rust process lock.

## Testing Guidelines

Add tests with every behavior change or bug fix. Cover Vitest/RTL for web and server, `cargo test` for Rust crates, Playwright for browser flows, and parity fixtures for core compatibility. Keep tests deterministic, independent, and named after observable behavior. Mirror `src/` paths under `tests/` with `*.test.*` naming. There is no CI workflow; run and report the relevant local checks before opening a pull request.

## Commit & Pull Request Guidelines

History uses `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:` prefixes. Use concise imperative subjects; add a body only for complex or breaking work explaining what and why. Pull requests state what changed, why, and how it was verified, and call out platform, migration, signing, or security impact. Keep each pull request narrowly scoped.
