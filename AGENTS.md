# Repository Guidelines

## Project Structure & Module Organization

This repository is currently a clean starting point. Keep implementation code under `src/`, automated tests under `tests/`, and static resources under `assets/`. Mirror source paths in the test tree—for example, test `src/domain/parser.ts` in `tests/domain/parser.test.ts`. Place project-wide configuration at the repository root and keep generated output in ignored directories such as `dist/` or `coverage/`.

Prefer small modules with one clear responsibility. Group code by domain or feature once the project grows; avoid broad catch-all directories such as `utils/` unless the contents are genuinely shared.

## Build, Test, and Development Commands

This project uses Node.js, React, TypeScript, and pnpm:

- `pnpm install` — install locked dependencies.
- `pnpm run dev` — start the Vite dashboard; run `pnpm run dev:server` separately for the local API.
- `pnpm test` — run the complete test suite.
- `pnpm run lint` — check formatting and static-analysis rules.
- `pnpm run typecheck` — validate TypeScript without emitting files.
- `pnpm run build` — create production artifacts.
- `pnpm run setup -- --dry-run` — preview required macOS service changes.
- `pnpm run doctor` — report missing DNS, proxy, and resolver prerequisites.

Do not introduce production dependencies without prior approval.

## Coding Style & Naming Conventions

Follow `eslint-config-pk` through `eslint.config.js`. Use `camelCase` for functions and variables, `PascalCase` for React components and types, and lowercase kebab-case for non-component modules. Organize frontend code by feature; keep components, hooks, API access, and pure validation in separate focused files. Use Tailwind CSS utilities and shared theme tokens instead of large component-specific stylesheets.

## Testing Guidelines

Add tests with every behavior change or bug fix. Vitest covers unit, component, and API integration tests; React Testing Library exercises UI behavior. Keep tests deterministic, independent, and named after observable behavior. Use `*.test.*` and mirror source paths under `tests/`. Run tests, lint, typecheck, and build before opening a pull request.

## Commit & Pull Request Guidelines

There is no existing commit history to establish a convention. Use concise imperative subjects, optionally prefixed by type, such as `feat: add domain parser` or `docs: clarify setup`. Pull requests should explain what changed, why it changed, and how it was verified. Link relevant issues and include screenshots for visible interface changes. Keep each pull request narrowly scoped and call out configuration, migration, or security implications explicitly.
