# Shared Unix Extraction Plan (Linux Milestone 2, Phase A)

## Goal

Move the platform-neutral macOS runtime machinery into `platform/unix/*`
exactly as listed in `docs/superpowers/specs/2026-10-26-linux-host-integration-design.md`
("Shared Unix extraction"), with zero behavior change. Linux host work
starts only after this lands with the macOS suite green.

## Constraints

- No behavior change on macOS: no artifact, path, manifest field,
  diagnostic, or runtime behavior differences. Existing tests are the
  contract; no test expectations change.
- Windows code untouched: no new `cfg` branches in shared command or core
  logic; `cargo check` for `x86_64-pc-windows-msvc` and the Windows test
  files stay green.
- Moves are verbatim first; only the imports and module paths change.
  Renames or cleanups are separate follow-ups, not part of this plan.
- `platform/unix` modules are unconditional (`pub mod unix;`); Linux-only
  pieces (`ss` listeners) land in Phase C, not here.

## Out of Scope

Linux runtime backend, setup transactions, inspector, sidecars, packaging
(Phases B–E). Any macOS behavior fix found during the move gets its own
bugfix commit, not smuggled into a move.

## Verification Commands

- baseline + per-task: `cargo test -p own-domains-daemon --test macos_host --test macos_setup --test macos_runtime --test macos_config --test macos_process --test platform_contract` (repo root, native macOS)
- full: `cargo test --workspace --locked` (repo root)
- lint: `cargo clippy --workspace --all-targets --locked -- -D warnings` (repo root)
- windows: `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc` (repo root)
- all targets: also `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`
- fmt: `cargo fmt --all -- --check` (repo root)

### Task 1: `platform/unix/command.rs`

Files:
- create `crates/own-domains-daemon/src/platform/unix/mod.rs`
- create `crates/own-domains-daemon/src/platform/unix/command.rs`
- modify `crates/own-domains-daemon/src/platform/macos/host.rs`
- modify `crates/own-domains-daemon/src/platform/macos/mod.rs`
- modify `crates/own-domains-daemon/src/platform/mod.rs`

Steps:
1. Confirm the baseline suite passes (command above); record the result.
2. Move `HostCommand`, `HostCommandOutput`, `CommandRunner`, `SystemCommandRunner` verbatim into `platform/unix/command.rs`; re-export from `platform/unix/mod.rs` and `platform::unix`; import them back in `platform/macos/host.rs`; re-export unchanged names from `platform/macos/mod.rs`.
3. Run the baseline suite; confirm identical green.
4. Run the full verification commands; confirm nothing regressed.

Done when: the macOS suite is green with the types living in `platform::unix`.

### Task 2: `platform/unix/files.rs`

Files:
- create `crates/own-domains-daemon/src/platform/unix/files.rs`
- modify `crates/own-domains-daemon/src/platform/macos/host.rs`
- modify `crates/own-domains-daemon/src/platform/macos/mod.rs`

Steps:
1. Confirm the baseline suite passes.
2. Move `HostFileKind`, `HostFileMetadata`, `HostFileSystem`, `SystemHostFileSystem`, `SetupFile`, and the atomic-write/chown/remove-file/remove-tree/symlink-safe helpers verbatim; re-export through `platform::unix` and `platform::macos` under unchanged names.
3. Run the baseline suite; confirm identical green.
4. Run the full verification commands; confirm nothing regressed.

Done when: file operations resolve through `platform::unix` with the macOS suite green.

### Task 3: `platform/unix/process.rs`

Files:
- create `crates/own-domains-daemon/src/platform/unix/process.rs`
- modify `crates/own-domains-daemon/src/platform/macos/process.rs`
- modify `crates/own-domains-daemon/src/platform/macos/mod.rs`

Steps:
1. Confirm the baseline suite passes.
2. Move the shared supervision core (`ProcessComponent`, `OwnedProcessRecord`, `ProcessIdentity`, `ListenerBinding`, `SpawnSpec`, `ChildProcess`, `ProcessHost`, `ProcessRecords`, `FileProcessRecords`, `ProcessSupervisor`, `RetryPolicy`, `Waiter`, `SystemWaiter`, `SystemProcessHost`) plus `canonical_program`, `system_spawn`, and `system_signal_group` verbatim. Keep the macOS `system_identity` and `system_listeners` implementations callable under unchanged names (re-exported through `platform::macos`); Linux variants land in Phase C.
3. Move the readiness probes only if they still live in macOS files (they were extracted to `platform::probe` already — verify, do not duplicate).
4. Run the baseline suite; confirm identical green.
5. Run the full verification commands; confirm nothing regressed.

Done when: supervision resolves through `platform::unix`, probes stay single-sourced, suite green.

### Task 4: `platform/unix/config.rs`

Files:
- create `crates/own-domains-daemon/src/platform/unix/config.rs`
- modify `crates/own-domains-daemon/src/platform/macos/config.rs`
- modify `crates/own-domains-daemon/src/platform/macos/mod.rs`

Steps:
1. Confirm the baseline suite passes.
2. Move `ConfigurationStager`, `ConfigurationValidator`, `ConfigurationFiles`, `SystemConfigurationFiles`, `SystemConfigurationValidator`, `CandidateConfiguration`, `ConfigurationError`, and `DEFAULT_DNS_VALIDATION_PORT` verbatim; re-export under unchanged names.
3. Run the baseline suite; confirm identical green.
4. Run the full verification commands; confirm nothing regressed.

Done when: staging/validation resolves through `platform::unix` with the suite green.

### Task 5: `platform/unix/runtime.rs`

Files:
- create `crates/own-domains-daemon/src/platform/unix/runtime.rs`
- modify `crates/own-domains-daemon/src/platform/macos/runtime.rs`
- modify `crates/own-domains-daemon/src/platform/macos/mod.rs`

Steps:
1. Confirm the baseline suite passes.
2. Move the start/reconcile/stop/inspect orchestration verbatim, parameterized exactly as the design specifies (platform inspector, runtime paths, setup server, readiness implementation); `MacOsRuntimeBackend` becomes a thin wrapper calling it.
3. Run the baseline suite; confirm identical green.
4. Run the full verification commands; confirm nothing regressed.

Done when: the orchestration lives in `platform::unix` with macOS as a thin wrapper and the suite green.

### Task 6: `platform/unix/setup.rs` and `platform/unix/paths.rs`

Files:
- create `crates/own-domains-daemon/src/platform/unix/setup.rs`
- create `crates/own-domains-daemon/src/platform/unix/paths.rs`
- modify `crates/own-domains-daemon/src/platform/macos/setup.rs`
- modify `crates/own-domains-daemon/src/platform/macos/paths.rs`
- modify `crates/own-domains-daemon/src/platform/macos/mod.rs`

Steps:
1. Confirm the baseline suite passes.
2. Move the transaction journal, backup naming, atomic file planning, preview/apply result construction, generic rollback, and `ensure_caddy_ca` verbatim; move the common state layout with a parameterized backup directory (macOS keeps `backups/setup/` under the state dir), target triple, and platform-specific binary names.
3. Run the baseline suite; confirm identical green.
4. Run the full verification commands; confirm nothing regressed.

Done when: setup transactions and paths resolve through `platform::unix` with the suite green.

### Task 7: Full verification and commit

Steps:
1. `cargo fmt --all`; then `cargo fmt --all -- --check`.
2. `cargo test --workspace --locked`; all green with output shown.
3. `cargo check` for all three non-host targets; all clean.
4. Both clippies; clean.
5. `git diff --stat` shows moves only (plus import updates); confirm no macOS fixture changes.
6. Commit per moved module (one commit each, `refactor:`) + push `feat/linux-host-rendering`.

Done when: Phase A is pushed with host + target evidence and zero behavior delta.
