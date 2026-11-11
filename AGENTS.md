# Repository Guidelines

## Overview

Own Domains pairs a React dashboard (`src/web/`) with a native Rust/Tauri runtime (`crates/`, bundled CoreDNS/Caddy sidecars). `src/` holds only `shared` types and the web app.

- Native acceptance target is Apple silicon macOS 15+; Intel macOS is buildable but not acceptance-required. Ubuntu 24+ has a live Linux backend (setup, inspector, `.deb`); Windows 10/11 has a stub backend whose full host-integration catalog is still pending.
- Defer volatile platform, signing, and acceptance detail to `README.md` and `docs/` (`desktop.md`, `platform-support.md`, `native-core.md`, `cli.md`, `macos-acceptance.md`, `linux-install.md`) instead of duplicating it here.

## Project Structure & Module Organization

- `src/web/` — React dashboard, organized by feature (`features/`, `app/`, `lib/`).
- `src/shared/` — types shared by web and native core.
- `crates/own-domains-{core,protocol,daemon,client,cli,desktop}/` — portable Rust core, IPC protocol, daemon, client, CLI, and Tauri desktop app.
- `tests/` — mirrors `src/web/` paths (`*.test.*`), plus `fixtures/` and `scripts/`. Rust crate tests live under `crates/*/tests/`.
- `docs/` — architecture, platform support, setup, and acceptance records. `scripts/` — `install.sh`/`install.ps1`, `uninstall.sh`/`uninstall.ps1`, sidecar fetch/stage, and release checks. `sidecars/` — pinned manifests (binaries under ignored `sidecars/build/`).
- Ignored outputs: `dist/`, `target/`, `sidecars/build/`, `artifacts/`, `coverage/`, `.worktrees/`.

Prefer small modules with one clear responsibility. Group web code by feature; keep components, hooks, API access, and pure validation in separate focused files.

## Build, Test, and Development Commands

Node.js, React, TypeScript via pnpm; Rust 1.90+ via Cargo:

- `pnpm install` — install locked dependencies.
- `pnpm run dev` — start the Vite dashboard; `pnpm run desktop:dev` for the Tauri client.
- `pnpm test` — Vitest unit/component tests.
- `pnpm run test:native-daemon` — `cargo test --workspace`.
- `pnpm run verify:native-core` — Rust core tests plus `pnpm test`, `lint`, `typecheck`, and `build`.
- `pnpm run lint` — eslint; `pnpm run typecheck` — `tsc -b`; `pnpm run build` — `tsc -b && vite build`.
- `pnpm run sidecars:fetch -- --verify-only` — verify pinned CoreDNS/Caddy digests (omit the flag once to download into `sidecars/build/`).
- `pnpm run desktop:prepare` / `pnpm run desktop:build` — stage daemon and build the desktop app; `:arm64`/`:x86_64` variants for arch-specific macOS builds, `:linux` for the `.deb`, `:windows` for NSIS.
- Native CLI via `cargo run -p own-domains-cli -- <setup|doctor|uninstall>`: `setup` and `uninstall` apply by default and require root (`sudo`); preview with `--dry-run`. `doctor` reports `*Unverifiable` warnings when run unprivileged.
- `pnpm run release:preflight` / `pnpm run release:check` — distribution readiness checks.

Integration tests that exercise the real sidecars need the pinned binaries under `sidecars/build/`; fetch them with `pnpm run sidecars:fetch`. Do not introduce production dependencies without prior approval.

## Coding Style & Naming Conventions

- Follow `eslint-config-pk` through `eslint.config.js`. Use `camelCase` for functions/variables, `PascalCase` for React components and types, lowercase kebab-case for non-component modules. Use Tailwind CSS utilities and shared theme tokens, not large component-specific stylesheets.
- Rust uses edition 2024. Keep shared behavior in `core` and wire contracts in `protocol`; keep platform work behind the daemon/CLI host-adapter contracts and do not add platform branches to shared command or core code.
- Never re-sign the pinned CoreDNS/Caddy sidecars (signature bytes are part of the pinned SHA-256 digest).

## Testing Guidelines

Add tests with every behavior change or bug fix. Cover Vitest/RTL for the web app and `cargo test` for Rust crates. Keep tests deterministic, independent, and named after observable behavior. Mirror `src/web/` paths under `tests/web/` with `*.test.*` naming. There is no CI workflow; run and report the relevant local checks before opening a pull request.

## Commit & Pull Request Guidelines

History uses `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:` prefixes. Keep history linear: fast-forward only, rebase instead of merge, never create merge commits. Confirm the exact commit message and scope with the user before committing; propose one commit per finished task and wait for approval before starting the next. Use concise imperative subjects; add a body only for complex or breaking work explaining what and why. Pull requests state what changed, why, and how it was verified, and call out platform, migration, signing, or security impact. Keep each pull request narrowly scoped.
