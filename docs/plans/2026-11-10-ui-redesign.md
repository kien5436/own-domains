# UI Redesign + Legacy Retirement Plan

## Goal
Single-shell UI (drop the legacy TS server + `App.tsx`) restyled per the approved design: revision hidden from display, `#618764` primary with dark/light/system theme, nav list (Dashboard, Settings, User guide, Diagnostic), icon-first buttons (`lucide-react`), dashboard with start/stop toggle + quit + searchable domain grid with inline register, clickable domains last via Tauri opener, dismissible 5s notifications.

## Constraints
- User explicitly chose to retire the legacy TS runtime now. This overrides the standing "keep legacy until no beta users depend on it" policy; the override and its consequences (no browser-only dev mode, no Playwright e2e) are recorded here, not revisited per task.
- `revision` stays in core/protocol/mutations (concurrency token). Only its *display* is removed.
- No daemon/protocol/core/CLI changes, except the Tauri opener plugin registration required for external links.
- `src/cli/*` (TS setup/doctor/uninstall scripts) and `scripts/install.sh` stay untouched; only `src/server/*` (the dashboard API backend) is retired.
- TDD per task where behavior changes; deletion tasks verify by suite-green.

## Out of Scope
- `desktop:build:linux` bundle target, .deb/AppImage packaging.
- New diagnostic content (relocation only).
- Unifying anything beyond the web shell.
- `src/cli/*` was initially scoped as untouched, but `src/cli/setup-plan.ts` imported `buildDnsConfig`/`buildProxyConfig` from `src/server`, so the TS CLI was retired in A1 as well (deleted `src/cli/`, `tests/cli/`, npm scripts `setup`/`doctor`/`uninstall`). `tests/integration/` went with it (it imported `src/server`). The `@playwright/test` devDependency is left installed but unused.

## Verification Commands
- test: `pnpm test` (vitest, repo root)
- single file: `pnpm test <path>` (e.g. `pnpm test tests/web/Shell.test.tsx`)
- lint: `pnpm run lint`
- types: `pnpm run typecheck`
- web build: `pnpm run build`
- rust (opener task only): `cargo clippy -p own-domains-desktop --all-targets --locked --offline -- -D warnings`

---

## Phase A — Retire legacy, single shell

### Task A1: Delete the TS server, its tests, and the e2e harness
Files:
- delete `src/server/` (index, domains/*, http/*, proxy/*, network/*)
- delete `tests/server/` and `tests/e2e/domains.spec.ts`
- delete `playwright.config.ts`
- modify `package.json` (remove `dev:server` script; remove `test:e2e` script and `@playwright/test` devDependency only if nothing else references them — verify with grep first)

Steps:
1. Grep for remaining references to `src/server`, `dev:server`, `playwright` outside the files above.
2. Delete the files; edit `package.json`; run `pnpm install --frozen-lockfile=false` only if the lockfile needs the playwright removal (note: prefer keeping the dependency if removal churns the lockfile — decide by diff size).
3. Run `pnpm test`; confirm green except `tests/web/App.test.tsx` (orphaned — goes in A2).

Done when: no `src/server` references remain and the only failing suite is `App.test.tsx`.

### Task A2: Delete App-exclusive web files, promote NativeApp to the single shell
Files:
- delete `src/web/app/App.tsx`, `src/web/features/domains/api.ts`, `src/web/features/domains/use-domains.ts`, `src/web/features/status/api.ts`, `src/web/features/status/use-status.ts`, `src/web/features/setup/NetworkSettings.tsx`, `tests/web/App.test.tsx`
- modify `src/web/main.tsx` (always mount the single shell; non-Tauri renders it in disconnected state — no backend exists in a bare browser anymore)
- modify `src/web/app/NativeApp.tsx` only to remove the `Revision {revision}` display line (keep the `revision` variable — mutations still need `expected_revision`)
- modify `tests/web/native-management.test.tsx` (fix any import paths if the shell file is renamed; prefer keeping the `NativeApp.tsx` filename to minimize churn)

Steps:
1. Write/adjust a test asserting `main.tsx` mounts the shell without the legacy `domainApi` import (extend `native-management.test.tsx` or add `tests/web/Shell.test.tsx` asserting no `Revision` text renders).
2. Run `pnpm test tests/web/Shell.test.tsx`; confirm it fails (Revision text present).
3. Delete the files; edit `main.tsx` + revision display line.
4. Run `pnpm test tests/web/Shell.test.tsx`; confirm it passes.
5. Run `pnpm test`, `pnpm run lint`, `pnpm run typecheck`; confirm all green.

Done when: full Node suite green with one shell, no `Revision` text in the UI, mutations still send `expected_revision`.

### Task A3: Update docs that reference the retired pieces
Files:
- modify `docs/setup.md` (remove `dev:server` two-terminal instructions)
- modify `docs/linux-acceptance.md` scope note if it claims the TS suites as acceptance evidence (check only; edit if stale)
- leave `AGENTS.md` and `docs/platform-support.md` legacy-runtime statements to the user (they encode policy, not code state)

Steps:
1. Grep `docs/` for `dev:server`, `test:e2e`, `playwright`, `src/server`.
2. Fix stale references; no code changes.
3. Run `pnpm run lint`; confirm green.

Done when: docs grep clean (excluding this plan file and historical acceptance logs, which stay verbatim).

### Task A4: Phase A full verification
Files: none (verification only).

Steps:
1. Run `pnpm test`, `pnpm run lint`, `pnpm run typecheck`, `pnpm run build`; confirm all green.
2. Run `cargo test --workspace --locked --offline`; confirm green (no Rust changes expected, guards against accidental edits).

Done when: all five commands green on `feat/` branch, tree otherwise unchanged.

---

## Phase B — Redesign (depends on Phase A)

### Task B1: Theme foundation (primary #618764, dark/light/system, no flash)
Files:
- modify `src/web/styles/app.css` (Tailwind v4 `@custom-variant dark`, primary token from `#618764`, `color-scheme` handling)
- modify `index.html` (inline pre-hydration script: read `localStorage` theme or `prefers-color-scheme`, set `.dark` class)
- create `src/web/features/theme/theme.ts` (get/set/effective-theme + system listener) and `src/web/features/theme/use-theme.ts`
- create `tests/web/theme.test.ts` (system default, stored override, invalid stored value falls back to system)

Steps:
1. Write the test file; run `pnpm test tests/web/theme.test.ts`; confirm it fails (no theme module).
2. Implement `theme.ts` + `use-theme.ts` + CSS variant + index.html script.
3. Run the test; confirm it passes.
4. Run `pnpm run lint`, `pnpm run typecheck`; confirm green.

Done when: theme resolves system/light/dark, persists, and no unstyled flash on reload.

### Task B2: Notification stack (dismissible, 5s auto-hide)
Files:
- create `src/web/features/notifications/Notifications.tsx` (stack + timeout + manual dismiss, `role="alert"`)
- create `tests/web/Notifications.test.tsx` (renders message, auto-hides after 5s with fake timers, dismiss button removes it)
- modify `src/web/app/NativeApp.tsx` (replace inline `actionError`/`state.error` `<p>` with the stack; keep error state as the source)

Steps:
1. Write the test; run `pnpm test tests/web/Notifications.test.tsx`; confirm it fails.
2. Implement the component; wire into `NativeApp.tsx` without changing error sources.
3. Run the test; confirm it passes.
4. Run `pnpm run lint`, `pnpm run typecheck`; confirm green.

Done when: action/connection errors appear as dismissible toasts that vanish after 5s.

### Task B3: Nav shell (Dashboard, Settings, User guide, Diagnostic)
Files:
- create `src/web/app/Shell.tsx` (nav list + content area + notification stack mount)
- create `tests/web/Shell.test.tsx` (four nav items render, switching shows the right section, active item indicated)
- modify `src/web/app/NativeApp.tsx` (render sections inside `Shell`; sections initially reuse existing components verbatim)

Steps:
1. Write the test; run `pnpm test tests/web/Shell.test.tsx`; confirm it fails.
2. Implement `Shell.tsx` with `lucide-react` icons (add dependency: `pnpm add lucide-react`); mount in `NativeApp.tsx`.
3. Run the test; confirm it passes.
4. Run `pnpm run lint`, `pnpm run typecheck`; confirm green.

Done when: nav switches sections, icons render, existing content unmodified inside sections.

### Task B4: Dashboard header (start/stop toggle, quit, search-as-register)
Files:
- modify `src/web/app/NativeApp.tsx` (replace Start/Stop/Quit button row with a single routing toggle + quit icon-button at top; keep `startRouting` reconnect logic untouched)
- modify `src/web/features/domains/DomainForm.tsx` (search input above the list drives it: typing a valid unregistered name shows help text + port/TLS register inline; registered/invalid names show the corresponding message; reuse `form-validation.ts` rules)
- create/extend test `tests/web/DomainForm.test.tsx` (unregistered name reveals register UI; registered name shows taken message; invalid shows validation message)

Steps:
1. Write the new test cases; run `pnpm test tests/web/DomainForm.test.tsx`; confirm they fail.
2. Implement the toggle row and the search-driven form.
3. Run the test; confirm it passes.
4. Run `pnpm run lint`, `pnpm run typecheck`; confirm green.

Done when: one toggle controls routing, search box registers new names inline, no revision text anywhere.

### Task B5: Domain cards grid (enable/disable, TLS toggle)
Files:
- modify `src/web/features/domains/DomainTable.tsx` (grid of cards instead of list; keep empty-state message)
- modify `src/web/features/domains/DomainRow.tsx` (card: hostname, port/TLS summary, enable/disable icon-button, TLS switch, edit-port, delete-confirm; no external-link yet — deferred to B9)
- extend `tests/web/` domain tests (card renders, enable toggle calls `onToggle`, TLS switch calls `onSetTls`, delete still confirms)

Steps:
1. Write the test cases; run them; confirm they fail.
2. Implement cards with `lucide-react` icons and a switch input for TLS.
3. Run the tests; confirm they pass.
4. Run `pnpm run lint`, `pnpm run typecheck`; confirm green.

Done when: grid renders, all four actions work, TLS is a toggle not a button.

### Task B6: Settings (network as-is + theme selector)
Files:
- modify `src/web/features/setup/NetworkSettingsEditor.tsx` (no behavior change; style to new theme only if needed — prefer untouched)
- create theme selector wiring in the Settings section (reuse `use-theme` from B1: dark/light/system radio or select)
- extend `tests/web/theme.test.ts` or add selector test (choosing an option persists and applies `.dark`)

Steps:
1. Write the selector test; run it; confirm it fails.
2. Implement the selector in Settings.
3. Run the test; confirm it passes.
4. Run `pnpm run lint`, `pnpm run typecheck`; confirm green.

Done when: theme selectable from Settings, network editor behavior unchanged.

### Task B7: User guide (phone setup + CA trust + troubleshooting)
Files:
- modify `src/web/features/setup/DeviceInstructions.tsx` (move into User guide section; add CA-trust steps and a short troubleshooting list; content only, no logic)
- add/extend `tests/web/DeviceInstructions.test.tsx` (sections render)

Steps:
1. Write the content test; run it; confirm it fails.
2. Implement the guide content.
3. Run the test; confirm it passes.
4. Run `pnpm run lint`, `pnpm run typecheck`; confirm green.

Done when: guide covers phone DNS setup, CA install + trust, and common failures.

### Task B8: Diagnostic (healthy message + doctor button)
Files:
- modify `src/web/features/status/StatusPanel.tsx` (empty diagnostics render a healthy message instead of "No diagnostics available yet"; add "Run doctor" button wired to the `doctor` command through the existing store/client path — verify the protocol `Command` variant name during implementation)
- extend `tests/web/StatusPanel.test.tsx` (healthy message on empty; doctor button calls through)

Steps:
1. Write the test cases; run them; confirm they fail.
2. Implement healthy message + doctor wiring.
3. Run the tests; confirm they pass.
4. Run `pnpm run lint`, `pnpm run typecheck`; confirm green.

Done when: clean state shows healthy, doctor button refreshes diagnostics.

### Task B9: External links via Tauri opener (last) + full verification
Files:
- modify `crates/own-domains-desktop/src/shell.rs` (register `tauri_plugin_opener`; add `plugin-opener` capability — check `tauri.conf.json` capabilities first)
- modify `src/web/features/domains/DomainRow.tsx` (hostname card link opens in the default browser via the opener JS API)
- add `Cargo.toml`/`package.json` dependency entries for the opener plugin as required

Steps:
1. Confirm current behavior: click a domain link in the desktop app, observe it fails or stays in-webview (record the outcome — this is the RED).
2. Implement opener registration + link handling.
3. Confirm links open in the system browser.
4. Run `pnpm test`, `pnpm run lint`, `pnpm run typecheck`, `pnpm run build`, and `cargo clippy -p own-domains-desktop --all-targets --locked --offline -- -D warnings`; confirm all green.

Done when: domain cards open in the default browser; every verification command green; plan complete.
