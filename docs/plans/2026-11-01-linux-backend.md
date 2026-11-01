# Linux Backend C1 Plan

## Goal

Working Linux runtime backend mirroring `platform::windows::backend`: real `apply` (render Corefile/Caddy JSON, spawn sidecars, readiness probes, admin load, setup server), `stop`, `inspect`, manifest-gated `diagnose`, wired through the `platform::runtime_backend` facade, verified host-side with fakes plus an end-to-end test on the Ubuntu VM against real sidecars.

## Constraints

- Mirror `platform::windows::backend` structure (Backend* traits, `LinuxAdapters`, `with_adapters`, `System*` impls, `terminate_all`, idempotent re-apply, cleanup on failure).
- Linux differences: no `.exe` suffix; triple-infixed staged names (`coredns-<triple>`, `caddy-<triple>`); Caddy admin over unix socket (`CaddyAdminClient::new`), not TCP; DNS probe `127.0.0.1:5354`, HTTP probe `127.0.0.1:8080`, setup server `127.0.0.1:4781`.
- `LinuxRuntimeBackend::new` stays a manifest-gated skeleton (diagnose only); real backend is built via new `with_adapters`. Existing `tests/linux_platform.rs` must pass unmodified, including `apply` → `Err(Unavailable)`.
- `diagnose` stays manifest-gate-only (no missingSidecar check): the healthy-manifest test asserts an empty vec and there are no staged sidecars on host.
- No CLI changes (gates at `command.rs:233,268` untouched); no setup/uninstall state changes; no trust/CA work; e2e runs unprivileged (all ports > 1024).
- Setup server cert path `<runtime>/ca.crt` need not exist: `SetupServer::bind` only binds TCP; the cert is read lazily per `/ca.crt` request and `/` serves static HTML.

## Out of Scope

C2 (Linux setup/uninstall transactions, CLI `host_installer`, elevation gating), trust/CA generation, nftables/service activation, `.deb`/AppImage packaging, `platform::unix` process-module reuse (mirroring Windows keeps the seam self-contained; unification is a later refactor).

## Verification Commands

- test (host, per task): `cargo test -p own-domains-daemon --test linux_backend_unit` from `/Users/kien/workspace/domains`
- test (VM only): `OWN_DOMAINS_TEST_COREDNS_EXE=<coredns> OWN_DOMAINS_TEST_CADDY_EXE=<caddy> OWN_DOMAINS_TEST_LAN=<guest-ipv4> cargo test -p own-domains-daemon --test linux_backend_e2e` on the Ubuntu VM
- full suite (host): `cargo test -p own-domains-daemon`
- lint: `cargo clippy -p own-domains-daemon --all-targets -- -D warnings` from `/Users/kien/workspace/domains`
- build: `cargo build -p own-domains-daemon`
- fmt: `cargo fmt --all -- --check` from `/Users/kien/workspace/domains`

### Task 1: LinuxPaths, resolve_paths, bundled layout helpers, unit tests

Files:
- modify `crates/own-domains-daemon/src/platform/linux/runtime.rs` (append paths section; keep existing `new`/diagnose untouched)
- create `crates/own-domains-daemon/tests/linux_backend_unit.rs` (resolve_paths tests only for now)

Steps:
1. Write test: `resolve_paths` with explicit `--sidecars` dir returns `coredns`/`caddy` (no `.exe`) under the absolutized dir; with `resources` pointing at a staged `sidecars/<triple>/` dir containing both triple-infixed binaries it returns those paths; without resources it falls back to exe-dir plain names; relative `state_dir` resolves against `current_dir`. Assert `bundled_target()` equals `x86_64-unknown-linux-gnu` on x86_64.
2. Run `cargo test -p own-domains-daemon --test linux_backend_unit`; confirm it fails (no `LinuxPaths`/`resolve_paths` yet).
3. Implement `LinuxPaths { runtime_directory, coredns_exe, caddy_exe, admin_socket }` where `admin_socket` is `<runtime>/caddy-admin.sock`; `bundled_target()` mapping x86_64/aarch64 to `x86_64-unknown-linux-gnu`/`aarch64-unknown-linux-gnu`; `bundled_resources_dir` (exe-dir `sidecars/` subdir probe, mirroring Windows); `resolve_paths` mirroring the Windows branch logic (explicit dir wins, staged triple-infixed names used only when both files exist, else exe-dir fallback).
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-daemon --test linux_platform`; confirm nothing regressed.

Done when: path resolution is pinned by tests and the existing suite is green.

### Task 2: Backend traits, adapters, and real apply()/stop()/inspect()

Files:
- modify `crates/own-domains-daemon/src/platform/linux/runtime.rs` (BackendProcessHost/Child/Readiness/Admin/Setup traits, LinuxAdapters, with_adapters, full apply/stop/inspect)
- modify `crates/own-domains-daemon/tests/linux_backend_unit.rs` (fake adapters + orchestration tests)

Steps:
1. Write test with fakes (FakeHost/FakeChild recording spawn args, FakeReadiness recording probes, FakeAdmin recording load, FakeSetup tracking started/stopped): apply writes Corefile + caddy.json under runtime dir, spawns `coredns -conf <Corefile>` and `caddy run --config <json>`, probes DNS `127.0.0.1:5354` for the LAN address and HTTP `127.0.0.1:8080` for the mapped host (skipped when no plain-HTTP mapping), loads admin config, starts setup server on `127.0.0.1:4781`, inspect reports Running; second apply with same revision is a no-op (no new spawns); stop terminates children and stops setup, inspect reports Stopped; apply failure (readiness error) terminates spawned children and stops setup.
2. Run the test; confirm it fails (no `with_adapters`, no traits).
3. Implement mirroring `platform/windows/backend.rs` apply/stop/inspect/terminate_all/children_alive exactly, except: no `quote_arg`/CREATE_NO_WINDOW (Unix argv needs no quoting), DNS port 5354, HTTP port 8080, admin via injected `BackendAdmin`, no Windows-only bits.
4. Run the unit test; confirm it passes.
5. Run `cargo test -p own-domains-daemon --test linux_platform`; confirm existing tests still pass unmodified.

Done when: the full orchestration is covered by fakes, including idempotent re-apply and failure cleanup.

### Task 3: Real System* impls (process host, readiness, admin, setup)

Files:
- modify `crates/own-domains-daemon/src/platform/linux/runtime.rs` (SystemProcessHost via std::process::Command, SystemReadiness retry loop over probe-crate probes, BackendAdmin impl for CaddyAdminClient, SystemSetup wrapping crate::SetupServer)
- modify `crates/own-domains-daemon/tests/linux_backend_unit.rs` (spawn/kill/wait test, setup lifecycle test)

Steps:
1. Write test: `SystemProcessHost::spawn` runs `/bin/true` and `wait_exit` returns 0; spawning `sleep 60` then `terminate` + `wait_exit` succeeds; `SystemSetup::new` is not running, `start(127.0.0.1:0, <temp>/ca.crt)` marks running, second start is a no-op success, `stop` clears it.
2. Run the test; confirm it fails (no System* impls).
3. Implement: spawn via `std::process::Command` (no shell, no quoting), terminate via child kill, wait_exit polls with 5s cap like Windows; `SystemReadiness` with the same 30s-deadline/2s-attempt retry loop as Windows driving `DnsReadinessProbe`/`HttpReadinessProbe`; `BackendAdmin for CaddyAdminClient` delegating to `CaddyAdmin::load`; `SystemSetup` holding `Mutex<Option<SetupServer>>` with start/stop/is_running mirroring Windows.
4. Run the unit test; confirm it passes.
5 plasma. Run `cargo build -p own-domains-daemon`; confirm it compiles.

Done when: real adapters exist and the spawn/kill/setup lifecycle is covered on host (readiness against live DNS stays e2e-only).

### Task 4: Facade wiring to the real backend

Files:
- modify `crates/own-domains-daemon/src/platform/linux/runtime.rs` (`runtime_backend` builds via `resolve_paths` + `Box::leak`ed System impls + `CaddyAdminClient::new`)
- modify `crates/own-domains-daemon/src/platform/linux/mod.rs` (re-export new items as needed)

Steps:
1. Write test: on Linux only (`#[cfg(target_os = "linux")]` in a host-runnable spot is impossible — instead assert via existing facade test path): extend nothing; rather verify by code path — construct via `with_adapters` already covered. For the factory itself, add a unit test that `runtime_backend` with `settings_only=true` returns a backend whose diagnose is empty, and with a temp state dir (no manifest) diagnose reports `setupRequired` (no sidecars needed since diagnose is manifest-gated).
2. Run the test; confirm it fails (factory still returns the skeleton path / missing wiring).
3. Implement factory mirroring `platform/windows/factory.rs`: absolutize state dir, derive exe dir, `bundled_resources_dir` fallback, `resolve_paths`, build `CaddyAdminClient::new(admin_socket)`, `Box::leak` the four System impls, `with_adapters`.
4. Run `cargo test -p own-domains-daemon --test linux_platform`; confirm green (facade test is cfg(linux)-gated so host only type-checks it — full run happens on the VM).
5. Run full host suite: `cargo test -p own-domains-daemon`.

Done when: the facade constructs the live backend and host tests pass (VM run covers the cfg-gated assertions).

### Task 5: Linux e2e test file (VM-only)

Files:
- create `crates/own-domains-daemon/tests/linux_backend_e2e.rs` (`#![cfg(target_os = "linux")]`)

Steps:
1. Write test mirroring `windows_backend_e2e.rs`: read `OWN_DOMAINS_TEST_COREDNS_EXE`, `OWN_DOMAINS_TEST_CADDY_EXE` (must exist, absolute), `OWN_DOMAINS_TEST_LAN` (plain IPv4, trimmed); spawn upstream TCP responder on loopback ephemeral port serving `200 ok`; `apply` a one-mapping config; assert `Running`; assert DNS serves the LAN address on 127.0.0.1:5354 via `SystemReadiness.wait_dns`; assert `Host: app.own` on 127.0.0.1:8080 returns 200 ending in `ok`; assert setup page on 127.0.0.1:4781 contains `Own Domains`; `stop`; assert `Stopped`; assert ports 5354, 8080, 8443, 4781 refuse connections; loop twice to catch port/state leaks.
2. Run `cargo test -p own-domains-daemon --test linux_backend_e2e` on host (macOS): confirm it compiles and reports 0 tests run (cfg-gated out).
3. Document staging in the file header: fetch/stage Linux x86_64 CoreDNS + Caddy (e.g. via repo `sidecars:fetch` for linux target into a dir, `chmod +x` both), then the three env vars.

Done when: the e2e file compiles everywhere, runs nowhere except Linux, and documents its own staging.

### Task 6: Ubuntu VM run to green

Files: none (VM execution; fix fallout in `crates/own-domains-daemon/src/platform/linux/runtime.rs` or `tests/linux_backend_e2e.rs` as found).

Steps:
1. On the Ubuntu VM: stage CoreDNS + Caddy Linux x86_64 binaries, `chmod +x`, export `OWN_DOMAINS_TEST_COREDNS_EXE`, `OWN_DOMAINS_TEST_CADDY_EXE`, and `OWN_DOMAINS_TEST_LAN` (guest IPv4, e.g. from `ip -4 addr show`).
2. Run `cargo test -p own-domains-daemon --test linux_backend_e2e`; paste output.
3. Fix any failures in the backend or test (likely suspects: readiness timing on slow VM disks — extend retry deadline, not the probe logic; CoreDNS x64-vs-ARM64 guest mismatch — x64 binary under emulation works but is slow, prefer native arch build).
4. Re-run until green, including the second loop iteration proving no leaked children/ports.
5. Run the full host suite once more (`cargo test -p own-domains-daemon`) to confirm nothing regressed.

Done when: e2e passes twice in a row on the VM and the host suite is green.

### Task 7: Full verification and review

Files: none (verification only).

Steps:
1. Run `cargo test -p own-domains-daemon` on host; report results.
2. Run `cargo build -p own-domains-daemon --tests` (compile check incl. tests); report results.
3. Run `cargo clippy -p own-domains-daemon --all-targets` and `cargo fmt --all -- --check`; report results.
4. Re-run `cargo test -p own-domains-daemon --test linux_platform --test linux_backend_unit` explicitly; report results.
5. Present the diff for review (`git diff --stat`).

Done when: all checks above are reported with real output, and the diff is up for review. Do not merge; do not touch main.
