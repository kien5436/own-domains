# Windows Facade Wiring Plan (release slice 1)

## Goal

`own-domains-daemon.exe --pipe <name>` on Windows drives the real
`WindowsRuntimeBackend` through `platform::runtime_backend`, so CLI
`start/status/stop` operate a live runtime instead of `runtimeUnavailable`.

## Constraints

- No protocol/core changes (v2 / schema 1 frozen); no macOS behavior change.
- `settings-only` still yields `UnavailableRuntimeBackend` (mirror macOS).
- Sidecar exes resolve as `<sidecars>/coredns.exe` + `<sidecars>/caddy.exe`,
  defaulting to the daemon executable's directory; manifest support stays in
  slice 3 — no throwaway manifest parsing here.
- System adapters are process-lifetime singletons via `Box::leak` (macOS
  `SystemWaiter` precedent); no lifetime refactoring of the tested seam.

## Out of Scope

Setup/doctor/uninstall (slice 2), manifest/fetch (slice 3), Tauri bundle
(slice 4), version bump and release checks (slice 5).

## Verification Commands

- test: `cargo test -p own-domains-daemon --test windows_backend_unit` (host)
- test: `cargo test -p own-domains-daemon --test windows_facade` (Windows VM only)
- lint: `cargo clippy --workspace --all-targets --locked -- -D warnings` and
  `cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings`
- build: `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc`

### Task 1: Pure path resolution with host unit test

Files:
- modify `crates/own-domains-daemon/src/platform/windows/backend.rs` (or new `paths.rs`)
- create `crates/own-domains-daemon/tests/windows_paths.rs` (runs everywhere)

Steps:
1. Write test: `resolve_paths` absolutizes a relative state dir against a
   given current dir, defaults the sidecars dir to the executable dir,
   puts the runtime dir under the state dir, and yields an absolute admin
   socket; explicit sidecars dir wins.
2. Run `cargo test -p own-domains-daemon --test windows_paths`; confirm it
   fails (no such function).
3. Implement: pure `pub fn resolve_paths(state_dir, current_dir, exe_dir, sidecars) -> Result<WindowsPaths, String>`.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-daemon --test windows_backend_unit`; confirm nothing regressed.

Done when: path defaults are pinned by a host-run test with no I/O beyond temp dirs.

### Task 2: Windows factory returning the live backend

Files:
- create `crates/own-domains-daemon/src/platform/windows/factory.rs`
- modify `crates/own-domains-daemon/src/platform/windows/mod.rs`
- modify `crates/own-domains-daemon/src/platform/mod.rs`
- create `crates/own-domains-daemon/tests/windows_facade.rs` (`#![cfg(target_os = "windows")]`)

Steps:
1. Write test: `runtime_backend` with `settings_only=true` returns a backend
   whose `inspect` is `Stopped` and whose `start` reports unavailable;
   with `settings_only=false`, a temp state dir, and a sidecars dir holding
   zero-byte `coredns.exe`/`caddy.exe`, it returns a backend whose
   `diagnose` reports two `missingSidecar` errors (zero-byte files are not
   `is_file`… adjust: create real empty files — `is_file` is true for
   zero-byte files, so point at nonexistent names instead) and `inspect`
   is `Stopped`.
2. Run the test on the target check; confirm it fails (no factory).
3. Implement: `factory::runtime_backend` mirroring `macos/factory.rs`
   (absolutize via `resolve_paths`, leak system adapters, wrap construction
   errors as `Box<dyn Error>`); wire `#[cfg(target_os = "windows")]` in
   `platform/mod.rs`, keeping `Unavailable` for remaining platforms.
4. Run host unit tests; confirm green.
5. Run the Windows target check; confirm clean.

Done when: construction paths are wired and the VM test compiles (run in Task 4).

### Task 3: CLI smoke runbook on the ARM64 VM

Files: none (runbook only).

Steps:
1. On the VM, pull the branch, `cargo build -p own-domains-daemon`,
   run `own-domains-daemon.exe --state-dir <temp> --pipe od-facade`.
2. In a second shell, `own-domains.exe --pipe od-facade status` shows a
   stopped runtime (not `runtimeUnavailable`); `doctor`/`diagnose` path
   surfaces missing sidecars when pointed at an empty dir.
3. Stop the daemon with Ctrl+C; no listeners remain.

Done when: the daemon serves a live (stopped) runtime over the pipe and
exits cleanly.

### Task 4: Full verification and commit

Steps:
1. Run `cargo test --workspace --locked`; all green with output shown.
2. Run both clippy commands and the Windows target check; all clean.
3. Run `cargo fmt --all -- --check` and `git diff --check`; clean.
4. On the VM: `cargo test -p own-domains-daemon --test windows_facade`
   passes; paste output.
5. Commit with `feat:` subject and push `feat/windows-release`.

Done when: slice 1 is committed and pushed with host + VM evidence.
