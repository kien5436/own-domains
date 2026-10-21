# macOS Runtime Activation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Activate Own Domains through the Rust daemon on macOS with one-time privileged host setup, directly supervised CoreDNS and Caddy sidecars, live reconciliation, complete shutdown, and a restricted `setup.own` server.

**Architecture:** A version-2 IPC contract separates durable mutation success from runtime application success. Pure macOS renderers and injected command/filesystem adapters make privileged setup, configuration staging, process ownership, and rollback testable on every host; the production adapter uses fixed paths and argument arrays. CoreDNS 1.14.7 and Caddy 2.11.7 run as user-owned sidecars on high ports behind a persistent, narrowly scoped PF anchor.

**Tech Stack:** Rust 1.90+, Serde/serde_json, Clap 4, Tauri 2, standard Unix sockets/processes, `base64` 0.22.1, `sha2` 0.10.9, `libc` 0.2.191, CoreDNS 1.14.7, Caddy 2.11.7, React 19, TypeScript, Vitest.

**Spec:** `docs/superpowers/specs/2026-10-21-macos-runtime-activation-design.md`

## Global Constraints

- Obtain explicit approval before adding the three direct Rust production dependencies or the bundled CoreDNS and Caddy executables.
- Target macOS 13 and newer first; Linux and Windows retain `runtimeUnavailable` until separate platform plans are approved.
- Use IPC protocol version 2 for every daemon, CLI, desktop, fixture, and native frontend message in this release.
- Keep ports fixed at CoreDNS `5354`, Caddy HTTP `8080`, Caddy HTTPS `8443`, setup HTTP `4781`, and a state-directory Caddy admin Unix socket.
- Never run a shell or interpolate settings/UI input into a command. Production commands use fixed absolute executables and argument arrays.
- Never mutate `/etc`, PF, or the trust store outside an explicit `setup --apply` or `uninstall --apply` command.
- Default automated tests must use temporary directories and injected adapters; they must never apply PF rules, alter DNS, install trust, or signal unrelated host processes.
- Preserve schema-version-1 settings, the last applied runtime on reconciliation failure, and the existing CA identity.
- Keep the TypeScript runtime until the macOS physical acceptance gate passes. Do not remove it in an unverified Linux session.
- Commit only when explicitly authorized by the user.

## Review Focus

- Existing VPN or administrator PF configuration must produce a conflict or compose through the dedicated anchor; failed setup must restore byte-identical files and prior PF/trust state.
- PID reuse or a forged ownership record must never let stop/recovery signal an unrelated process; exact executable, arguments, process group, and start identity must match.
- A saved mutation whose CoreDNS or Caddy apply fails must retain the previous listeners/configuration, advance only `desiredRevision`, and report a degraded runtime.
- LAN address/interface drift after setup must return `setupRequired` before starting sidecars and must not silently widen PF rules.
- Partial shutdown must identify every survivor, remain retryable, and never report `Stopped` while an owned listener or process remains.

---

### Task 1: Advance the IPC contract and native clients to version 2

**Status:** Complete; commit deferred pending explicit authorization.

**Files:**
- Modify: `crates/own-domains-protocol/src/message.rs`
- Modify: `crates/own-domains-protocol/tests/wire.rs`
- Modify: `crates/own-domains-protocol/tests/desktop_contract.rs`
- Modify: `tests/fixtures/ipc/desktop-contract.json`
- Modify: `src/web/lib/native-protocol.ts`
- Modify: `src/web/lib/native-client.ts`
- Modify: `tests/web/lib/native-protocol.test.ts`
- Modify: `tests/web/lib/native-client.test.ts`

**Interfaces:**
- Consumes: existing `Settings`, `ApplyResult`, command envelopes, and version validation.
- Produces: `PROTOCOL_VERSION: u16 = 2`; `MutationResult { settings: Settings, apply: Option<ApplyResult> }`; `CommandResult::Mutation(MutationResult)`; and `RuntimeStatus::Degraded`.

- [x] Write failing Rust wire tests asserting every mutation serializes as `mutation`, `apply: null` is valid for offline writes, degraded status is camelCase, version 1 is rejected with supported version 2, and request IDs still echo exactly.
- [x] Update the Rust protocol types and replace mutation uses of `CommandResult::Settings` with `CommandResult::Mutation`; retain `Settings` only for read/list results.
- [x] Regenerate the Rust-owned desktop contract fixture and update its parity test to cover version 2, both mutation outcomes, degraded status, and every existing error.
- [x] Write failing TypeScript tests that parse the version-2 fixture and reject version 1 before returning a typed result.
- [x] Update `native-protocol.ts` and `native-client.ts` to expose `MutationResult` without weakening discriminated-union exhaustiveness.
- [x] Run `cargo test -p own-domains-protocol && pnpm test -- tests/web/lib/native-protocol.test.ts tests/web/lib/native-client.test.ts`; expect all focused tests to pass.
- [ ] Commit only when authorized with `feat: evolve runtime mutation protocol`.

### Task 2: Reconcile successful mutations without losing durable success

**Status:** Complete; commit deferred pending explicit authorization.

**Files:**
- Modify: `crates/own-domains-daemon/src/runtime.rs`
- Modify: `crates/own-domains-daemon/src/handler.rs`
- Modify: `crates/own-domains-daemon/tests/handler.rs`
- Modify: `crates/own-domains-cli/src/command.rs`
- Modify: `crates/own-domains-cli/tests/cli.rs`
- Modify: `src/web/features/runtime/runtime-store.ts`
- Modify: `tests/web/features/runtime/runtime-store.test.ts`
- Modify: `tests/web/native-management.test.tsx`

**Interfaces:**
- Consumes: Task 1 `MutationResult`, `RuntimeStatus::Degraded`, and existing daemon serialization.
- Produces: `RuntimeBackend::apply(&self, settings: &Settings) -> Result<ApplyResult, RuntimeError>` replacing `start`; live mutation reconciliation; and CLI exit/output semantics for saved-but-unapplied changes.

- [x] Add failing daemon tests for stopped/running/degraded mutation reconciliation and recovery.
- [x] Refactor `RuntimeBackend::start` to `apply` and preserve durable mutation success on application failure.
- [x] Publish settings, runtime, and diagnostics events after the complete serialized mutation/apply attempt.
- [x] Add CLI tests for saved-but-unapplied and stopped-runtime outcomes.
- [x] Preserve the complete `MutationResult` in CLI JSON output.
- [x] Add frontend degraded-state, revision-divergence, recovery, and CLI-originated mutation coverage.
- [x] Run focused Rust and frontend verification.
- [ ] Commit only when authorized with `feat: reconcile live runtime mutations`.

### Task 3: Render and inspect macOS host integration

**Status:** Complete; commit deferred pending explicit authorization.

**Files:**
- Create: `crates/own-domains-daemon/src/platform/mod.rs`
- Create: `crates/own-domains-daemon/src/platform/macos/mod.rs`
- Create: `crates/own-domains-daemon/src/platform/macos/paths.rs`
- Create: `crates/own-domains-daemon/src/platform/macos/host.rs`
- Create: `crates/own-domains-daemon/tests/macos_host.rs`
- Modify: `crates/own-domains-daemon/src/lib.rs`

**Interfaces:**
- Consumes: validated target user/home, state/resource directories, selected LAN IPv4 address/interface, fixed ports, and sidecar metadata.
- Produces: `RuntimePaths`; `SidecarManifest`; `HostSetupPlan { artifacts, commands, manifest }`; `build_host_setup_plan(&MacOsSetupOptions)`; `HostIntegrationInspector::inspect(&Settings) -> Vec<Diagnostic>`; and injected `CommandRunner`/`HostFileSystem` traits.

- [x] Add golden and adversarial host-integration tests.
- [x] Implement validated renderers, paths, manifest, and narrowly scoped PF rules.
- [x] Implement fixed-path, bounded host inspection with stable diagnostics.
- [x] Run the focused host-integration test suite.
- [ ] Commit only when authorized with `feat: model macos host integration`.

### Task 4: Add transactional privileged setup and uninstall commands

**Status:** Complete for the injected transaction and CLI contract; real macOS execution remains gated on Task 8’s verified sidecar manifest.

**Files:**
- Create: `crates/own-domains-daemon/src/platform/macos/setup.rs`
- Create: `crates/own-domains-daemon/tests/macos_setup.rs`
- Modify: `crates/own-domains-cli/src/lib.rs`
- Modify: `crates/own-domains-cli/src/command.rs`
- Modify: `crates/own-domains-cli/tests/cli.rs`
- Modify: `docs/cli.md`

**Interfaces:**
- Consumes: Task 3 `HostSetupPlan`, injected host adapters, `SUDO_USER`/`SUDO_UID`, and explicit `--lan-address`/`--interface`.
- Produces: CLI `setup [--apply] --lan-address <IPv4> --interface <name>` and `uninstall [--apply] [--purge]`; `preview_setup`; `apply_setup`; `preview_uninstall`; and `apply_uninstall`.

- [x] Add parser, rollback, idempotency, conflict, and uninstall coverage.
- [x] Implement injected transactional setup/uninstall and rollback journal.
- [x] Wire dry-run and guarded CLI setup/uninstall behavior.
- [x] Run focused daemon and CLI verification.
- [ ] Commit only when authorized with `feat: add macos runtime setup`.

### Task 5: Replace the setup page with a restricted Rust server

**Status:** Complete; commit deferred pending explicit authorization.

**Files:**
- Create: `crates/own-domains-daemon/src/setup_server.rs`
- Create: `crates/own-domains-daemon/tests/setup_server.rs`
- Modify: `crates/own-domains-daemon/Cargo.toml`
- Modify: `crates/own-domains-daemon/src/lib.rs`

**Interfaces:**
- Consumes: an explicit loopback `SocketAddr`, a public PEM certificate path, `base64` 0.22.1, and `sha2` 0.10.9.
- Produces: `SetupServer::bind`; cancellation/join handle; GET/HEAD routes `/`, `/ca.crt`, `/ca.mobileconfig`, and `/fingerprint`.

- [x] Add loopback HTTP route, error, header, and cancellation coverage.
- [x] Implement bounded HTTP parsing and one-response connection handling.
- [x] Render deterministic certificate-only Apple profiles and decoded-DER fingerprints.
- [x] Run setup-server tests and warnings-denied Clippy.
- [ ] Commit only when authorized with `feat: add restricted setup server`.

### Task 6: Stage configurations and supervise owned sidecars

**Status:** Complete; commit deferred pending explicit authorization.

**Files:**
- Create: `crates/own-domains-daemon/src/platform/macos/config.rs`
- Create: `crates/own-domains-daemon/src/platform/macos/process.rs`
- Create: `crates/own-domains-daemon/tests/macos_config.rs`
- Create: `crates/own-domains-daemon/tests/macos_process.rs`
- Modify: `crates/own-domains-core/src/dns.rs`
- Modify: `crates/own-domains-core/src/proxy.rs`
- Modify: `crates/own-domains-daemon/Cargo.toml`

**Interfaces:**
- Consumes: core configuration builders, Task 3 paths/runner, Task 5 setup server, and `libc` 0.2.191.
- Produces: `CandidateConfiguration`; `ConfigurationStager::stage/validate/promote/restore`; `OwnedProcessRecord`; and `ProcessSupervisor::start/inspect/stop/wait_for_readiness`.

- [x] Add core coverage for high-port listeners, Unix admin-socket configuration, setup-only routing, and absent legacy routes.
- [x] Write staging tests for owner-only candidate files, Caddy native-JSON validation arguments, CoreDNS alternate-port validation, atomic promotion, failed-promotion rollback, and unchanged applied revision.
- [x] Write process tests for minimal environment, fixed arguments, process groups, bounded startup/shutdown, graceful-to-forced escalation, log paths, durable records, PID reuse, forged arguments/path/start identity, ambiguous survivors, and listener verification.
- [x] Implement child supervision with direct `Command` execution, new process groups, retained `Child` handles for the live daemon, and exact record verification before recovery signals. Treat any incomplete identity match as diagnostic-only.
- [x] Implement Caddy admin HTTP over the Unix socket and direct UDP/TCP DNS plus HTTP readiness probes; cap frames, response bodies, and deadlines.
- [x] Run `cargo test -p own-domains-core --test dns --test proxy && cargo test -p own-domains-daemon --test macos_config --test macos_process`; expect all tests to pass with fake executables/listeners.
- [ ] Commit only when authorized with `feat: supervise macos runtime sidecars`.

### Task 7: Implement and wire `MacOsRuntimeBackend`

**Status:** Complete; commit deferred pending explicit authorization.

**Files:**
- Create: `crates/own-domains-daemon/src/platform/macos/runtime.rs`
- Create: `crates/own-domains-daemon/tests/macos_runtime.rs`
- Modify: `crates/own-domains-daemon/src/bin/own-domains-daemon.rs`
- Modify: `crates/own-domains-daemon/src/runtime.rs`
- Modify: `crates/own-domains-daemon/src/ipc.rs`
- Modify: `crates/own-domains-desktop/src/shell.rs`
- Modify: `crates/own-domains-desktop/tests/lifecycle.rs`

**Interfaces:**
- Consumes: Tasks 3–6 host inspection, setup server, staging, process supervision, and Task 2 `RuntimeBackend::apply`.
- Produces: `MacOsRuntimeBackend::new(RuntimePaths, SidecarManifest)`; transactional `apply/stop/inspect/diagnose`; macOS daemon selection; and orderly signal shutdown.

- [x] Write failing runtime tests for first start, idempotent start, live reconcile, Caddy rejection, CoreDNS replacement failure with previous-config restart, setup-server failure, degraded recovery, exact applied revision persistence, stale verified children, ambiguous children, port conflict, LAN drift, and retryable partial stop.
- [x] Implement apply ordering: inspect, clean verified stale children, stage/validate, start setup/CoreDNS/Caddy, probe, promote, and record. Roll back newly started components in reverse order on every failure.
- [x] Implement live Caddy load over its Unix socket and bounded CoreDNS replace/restore; return the previous applied revision on any failed candidate.
- [x] Implement stop ordering and listener verification. Install supported termination handlers that request the same stop path without bypassing daemon serialization.
- [x] Select `MacOsRuntimeBackend` only in normal macOS daemon mode; keep `UnavailableRuntimeBackend` for non-macOS and `--settings-only`.
- [x] Update desktop lifecycle tests so tray Quit waits for verified runtime/daemon shutdown and restores the window with the exact failure on partial cleanup.
- [x] Run `cargo test -p own-domains-daemon --test macos_runtime && cargo test -p own-domains-desktop`; expect all tests to pass.
- [ ] Commit only when authorized with `feat: activate macos runtime`.

### Task 8: Pin, fetch, verify, and package sidecars

**Status:** Implemented except signed macOS package/version execution; blocked on macOS hardware.

**Files:**
- Create: `sidecars/manifest.json`
- Create: `scripts/fetch-sidecars.ts`
- Create: `tests/scripts/fetch-sidecars.test.ts`
- Modify: `.gitignore`
- Modify: `package.json`
- Modify: `crates/own-domains-desktop/tauri.conf.json`
- Modify: `crates/own-domains-desktop/Cargo.toml`
- Modify: `docs/desktop.md`

**Interfaces:**
- Consumes: upstream CoreDNS 1.14.7 and Caddy 2.11.7 release assets/checksum files for `darwin/amd64` and `darwin/arm64`.
- Produces: checked-in metadata with exact upstream URLs and pinned archive digests plus separate executable SHA-256 digests; ignored downloaded executables; `pnpm run sidecars:fetch`; Tauri external-binary/resource entries; and release preflight.

- [x] Write failing downloader tests with a local fake fetcher for wrong digest, wrong archive member, wrong reported version, partial download, unsupported architecture, executable mode, and atomic cache replacement.
- [x] Record the four archive digests directly from upstream checksum assets, verify checksum signatures where the upstream publishes them, and separately record each extracted executable digest; reject redirects outside GitHub's release asset hosts. Caddy 2.11.7 publishes SHA-512 archive checksums, so the manifest records that upstream algorithm and a separate executable SHA-256.
- [x] Implement download/extract/verify using Node standard libraries and existing `tsx`; do not add a JavaScript production package.
- [x] Configure Tauri packaging and standalone CLI release layout to resolve only verified resource-relative binaries. Set `minimumSystemVersion` to `13.0` under `bundle.macOS`. Added verified resource resolution for setup/uninstall and Tauri `externalBin`/resource mappings.
- [x] Add a release check that runs both sidecars' version commands, verifies hashes, builds arm64/x86_64 packages, and fails if either binary is absent or mutable after signing (`pnpm run release:check`). It requires macOS and `APPLE_SIGNING_IDENTITY`; preflight verifies both architectures without executing them.
- [x] Run `pnpm test -- tests/scripts/fetch-sidecars.test.ts && pnpm run sidecars:fetch -- --verify-only && PKG_CONFIG=/usr/bin/pkg-config pnpm run desktop:build`; tests and the local Linux build pass when matching sidecars are present. `release:check` packaging and native version execution remain macOS-only.
- [ ] Commit only when authorized with `build: package macos runtime sidecars`.

### Task 9: Verify macOS behavior and gate legacy retirement

**Status:** In progress; scripts and documentation are implemented, but Apple silicon/Intel activation and the physical-phone pass are blocked on unavailable macOS hardware.

**Files:**
- Create: `scripts/verify-macos-runtime.sh`
- Create: `docs/macos-acceptance.md`
- Modify: `README.md`
- Modify: `docs/desktop.md`
- Modify: `docs/superpowers/plans/2026-10-20-native-app-migration-roadmap.md`
- Conditionally modify after acceptance: legacy TypeScript backend/CLI files and their tests.

**Interfaces:**
- Consumes: signed macOS package, CLI, test upstream servers, explicit disposable test hostname/state, and a physical phone for the final manual section.
- Produces: machine-readable acceptance log; recovery instructions; and a documented pass/fail gate for removing the TypeScript runtime.

- [x] Write a non-destructive preflight mode that records OS/architecture, verifies signatures/hashes, checks conflicting listeners/PF/resolver state, and refuses to apply changes without an explicit acceptance flag (`scripts/verify-macos-runtime.sh`, default preflight, `--accept` required).
- [x] Implement the reversible acceptance sequence: setup dry-run, setup apply, CLI-only start, UDP/TCP DNS, HTTP/HTTPS routes, setup-page route allowlist, `registry.own` negative checks, stop/restart, listener negatives, and final uninstall restoration. GUI/CLI parity and rejected-candidate rollback remain manual steps because they require the interactive app or an injected failing backend.
- [x] Document physical-phone DNS, HTTP, HTTPS, profile installation, fingerprint comparison, restart, and shutdown checks; record results separately from automated output (`docs/macos-acceptance.md`).
- [x] Run the full repository gates: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm test`, `pnpm run lint`, `pnpm run typecheck`, `pnpm run build`, feature-enabled desktop Clippy/check, release build, and `git diff --check`.
- [ ] Run the acceptance script on both Apple silicon and Intel macOS 13+ and complete one physical-phone pass. Do not claim macOS completion if any item is missing. Blocked: no macOS hardware or physical phone is available in this session.
- [ ] Only after recorded acceptance, write a separate reviewed removal change for the TypeScript management server, backend launch agent, legacy CLI, and `registry.own` compatibility paths. Preserve migration/uninstall support for existing installations. Blocked on the acceptance gate.
- [ ] Commit verification documentation only when authorized with `test: verify macos runtime activation`.

## Phase completion gate

The macOS runtime is complete only when protocol v2 clients interoperate, host setup is reversible, runtime mutations reconcile truthfully, all owned services stop completely, `setup.own` exposes only public setup material, `registry.own` is absent, signed arm64/x86_64 packages pass, and physical LAN/phone checks are recorded. Until then, the legacy TypeScript runtime remains available and the roadmap must describe macOS activation as in progress.
