# Windows Backend Plan (Phase 2, Milestone 2)

## Goal

A `WindowsRuntimeBackend` that applies rendered configs, starts real CoreDNS/Caddy sidecars, waits for readiness, live-reloads Caddy, and stops everything — tested with fake adapters on all platforms and with real sidecars on the Windows VM.

## Constraints

- Reuse the Milestone 1 constructors and renderers; no behavior change on macOS.
- Shared code goes in daemon-shared modules (reusable by the Linux backend), never in `platform::windows`, and never behind `cfg` in shared command/core code.
- The `RuntimeAdapters` seam mirrors macOS (`with_adapters` for fakes); no new production dependencies.
- No Job Objects (Gate D blocked on this host), no NRPT/firewall/trust/installer work (milestone 3).

## Out of Scope

- Host inspector, setup/uninstall transaction, elevation exchange, manifest, backups, rollback, diagnostics catalog.
- `nssm`/service behavior, auto-start, log rotation.
- Authenticode verification (lands with signed packaging).
- Linux backend (separate plan).

## Verification Commands

- test: `cargo test --workspace --locked` (repo root)
- windows check: `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc` (repo root)
- lint: `cargo clippy --workspace --all-targets --locked -- -D warnings` (repo root)
- fmt: `cargo fmt --all -- --check` (repo root)
- windows VM: `cargo test -p own-domains-daemon --test windows_backend` (ARM64 guest)

## Task 1: Extract the shared probe module

Files:
- create `crates/own-domains-daemon/src/probe.rs`
- modify `crates/own-domains-daemon/src/lib.rs`
- modify `crates/own-domains-daemon/src/platform/macos/process.rs`
- modify `crates/own-domains-daemon/src/platform/macos/runtime.rs`
- modify `crates/own-domains-daemon/tests/macos_process.rs`

Steps:
1. Write test: list the moved items (`CaddyAdmin` trait, `CaddyAdminClient`, `ReadinessProbe`, `DnsReadinessProbe`, `HttpReadinessProbe`, `read_http_response`) imported from `own_domains_daemon::probe` in a new `crates/own-domains-daemon/tests/probe.rs`; confirm it fails (module missing).
2. Run `cargo test -p own-domains-daemon --test probe`; confirm the import failure.
3. Implement: move the items verbatim into `probe.rs` (keep the `#[cfg(unix)]`/`#[cfg(not(unix))]` transport split inside `CaddyAdminClient::request`); re-export from `platform::macos::process` for compatibility; update `runtime.rs` and `macos_process.rs` imports.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-daemon`; confirm nothing regressed.

Done when: probes and admin client importable from `own_domains_daemon::probe`; macOS suite green.

## Task 2: Windows AF_UNIX admin request

Files:
- modify `crates/own-domains-protocol/src/pipe.rs` (or new `crates/own-domains-daemon/src/probe.rs` helper)
- modify `crates/own-domains-daemon/tests/windows_backend.rs` (new)

Steps:
1. Write test: start a real Caddy (Gate F download pattern, test config with loopback-TCP admin `127.0.0.1:2019`) and assert `CaddyAdminClient::load` switches the served body and `is_ready` passes afterwards.
2. Run `cargo test -p own-domains-daemon --test windows_backend` on Windows; confirm it fails (admin request is stubbed).
3. Implement: TCP `connect_timeout` + HTTP/1.1 request/response over `TcpStream` in the `#[cfg(windows)]` request path, reusing `read_http_response`. (An earlier AF_UNIX attempt failed on real hardware: POSIX exact-length `addrlen` is refused with WSAECONNREFUSED, and Caddy itself dies silently on backslash `unix/` admin paths.)
4. Run the test on the VM; confirm it passes.
5. Run the verification commands; confirm nothing regressed.

Done when: live `/load` over AF_UNIX works from the shared client on Windows.

## Task 3: Windows process spawn, kill, and wait

Files:
- create `crates/own-domains-daemon/src/platform/windows/process.rs`
- modify `crates/own-domains-daemon/src/platform/windows/mod.rs`
- modify `crates/own-domains-daemon/tests/windows_backend.rs`

Steps:
1. Write test: spawn a short-lived child (`cmd /c exit 0` and a `ping -n` sleeper), assert PID identity, terminate the sleeper, and assert exit is observed; assert `CREATE_NO_WINDOW` leaves no visible console (MainWindowHandle is zero).
2. Run on Windows; confirm it fails (module missing).
3. Implement: `CREATE_NO_WINDOW` spawn with admin-socket-free environment notes, `TerminateProcess` stop, and wait/poll for exit.
4. Run the test on the VM; confirm it passes.
5. Run the verification commands; confirm nothing regressed.

Done when: spawn/kill/wait works without a console window.

## Task 4: WindowsRuntimeBackend skeleton with fake adapters

Files:
- create `crates/own-domains-daemon/src/platform/windows/backend.rs`
- modify `crates/own-domains-daemon/src/platform/windows/mod.rs`
- create `crates/own-domains-daemon/tests/windows_backend_unit.rs` (runs everywhere)

Steps:
1. Write test: `with_adapters`-style fakes assert apply writes rendered configs, stop terminates processes, and double-stop is harmless.
2. Run `cargo test -p own-domains-daemon --test windows_backend_unit`; confirm it fails.
3. Implement: backend struct mirroring the macOS `RuntimeAdapters` seam (fakeable spawn/probe/admin/files), real config rendering, no-op inspector.
4. Run the test on all platforms; confirm it passes.
5. Run the verification commands; confirm nothing regressed.

Done when: backend logic is unit-tested without real binaries.

## Task 5: Backend apply/stop against real sidecars on the VM

Files:
- modify `crates/own-domains-daemon/tests/windows_backend.rs`

Steps:
1. Write test: with real CoreDNS/Caddy executables staged on the VM (Gate F pattern), apply a two-mapping config, assert DNS resolves and HTTP serves, then stop and assert ports are free.
2. Run on Windows; confirm it fails (backend cannot start sidecars yet).
3. Implement: wire spawn, readiness probes, and admin load into apply; terminate-and-wait into stop.
4. Run on the VM; confirm green, twice in a row (no stale state).
5. Run the verification commands; confirm nothing regressed.

Done when: real sidecars serve and stop cleanly twice.

## Task 6: Setup server wiring and console Ctrl handler

Files:
- modify `crates/own-domains-daemon/src/platform/windows/backend.rs`
- modify `crates/own-domains-daemon/src/bin/own-domains-daemon.rs`
- modify `crates/own-domains-daemon/tests/windows_backend.rs`

Steps:
1. Write test: after apply, `http://127.0.0.1:4781/` serves the setup page; after stop it refuses; sending Ctrl+C (or killing the console) leaves no listening sockets behind.
2. Run on Windows; confirm it fails.
3. Implement: start the existing setup server on apply and stop it on stop; install the `SetConsoleCtrlHandler` handler that was deferred as a TODO (request shutdown over the pipe).
4. Run on the VM; confirm it passes.
5. Run the verification commands; confirm nothing regressed.

Done when: setup page lifecycle matches start/stop, and Ctrl+C shuts down gracefully.

## Task 7: inspect/diagnose basics

Files:
- modify `crates/own-domains-daemon/src/platform/windows/backend.rs`
- modify `crates/own-domains-daemon/tests/windows_backend_unit.rs`

Steps:
1. Write test: inspect reports running after apply and stopped after stop; diagnose reports `runtimeUnavailable`-style actionable messages for missing sidecars (not panics).
2. Run; confirm it fails.
3. Implement: minimal inspect/diagnose over backend state (full inspector catalog stays in milestone 3).
4. Run; confirm it passes.
5. Run the verification commands; confirm nothing regressed.

Done when: status and doctor paths return truthful values.

## Task 8: Full verification and commit

Steps:
1. Run `cargo test --workspace --locked`; all green with output shown.
2. Run the windows check, lint, and fmt commands; all clean with output shown.
3. Run the full Windows VM suite (`cargo test --workspace` on the guest); all green.
4. Commit and push the milestone.

Done when: everything above passes and is reported.
