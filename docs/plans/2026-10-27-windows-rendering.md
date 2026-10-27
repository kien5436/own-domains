# Windows Rendering Plan (Phase 2, Milestone 1)

## Goal

Render byte-exact Windows host artifacts (CoreDNS Corefile with TTL 0,
Caddy JSON with ports 80/443 and h1/h2 only) with zero change to macOS output.

## Constraints

- macOS rendered output must stay byte-identical (TTL 60, no protocols field
  unless set).
- No `cfg(target_os)` branches in shared command or core logic; Windows-only
  constructors live in `platform::windows`.
- No new production dependencies.

## Out of Scope

- Runtime backend, NRPT, firewall, trust, setup transaction, probes/admin
  extraction (milestone 2), packaging, acceptance runs.

## Verification Commands

- test: `cargo test --workspace --locked` (repo root)
- windows check: `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc` (repo root)
- lint: `cargo clippy --workspace --all-targets --locked -- -D warnings` (repo root)
- fmt: `cargo fmt --all -- --check` (repo root)

### Task 1: CoreDNS TTL parameter

Files:
- modify `crates/own-domains-core/src/dns.rs`
- modify `crates/own-domains-core/tests/dns.rs`
- modify `crates/own-domains-core/tests/fixtures/Corefile.expected` only if the default render changes (it must not)

Steps:
1. Write test: `build_corefile` with TTL 0 renders `0 IN A`; default `for_settings` path still renders `60 IN A`.
2. Run `cargo test -p own-domains-core --test dns`; confirm the new test fails (no TTL parameter exists).
3. Implement: add `ttl_seconds: u32` to `DnsConfigOptions`; `for_settings` sets 60; template uses the parameter. Update the struct literal in `crates/own-domains-daemon/src/platform/macos/config.rs:90` and the two literals in `crates/own-domains-core/tests/dns.rs`.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-core`; confirm nothing regressed.

Done when: TTL 0 and TTL 60 render correctly; macOS fixture byte-identical.

### Task 2: Caddy protocols option

Files:
- modify `crates/own-domains-core/src/proxy.rs`
- modify `crates/own-domains-core/tests/proxy.rs`

Steps:
1. Write test: options with `protocols: Some(["h1", "h2"])` render `"protocols":["h1","h2"]` on both servers; `None` omits the field.
2. Run `cargo test -p own-domains-core --test proxy`; confirm it fails.
3. Implement: add `protocols: Option<Vec<String>>` to `ProxyConfigOptions`; `for_settings`/`for_runtime` set `None`; renderer emits the field only when `Some`.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-core`; confirm nothing regressed (macOS output unchanged).

Done when: protocols render only when set; existing proxy fixtures byte-identical.

### Task 3: Windows config constructors with exact-render tests

Files:
- modify `crates/own-domains-daemon/src/platform/mod.rs`
- create `crates/own-domains-daemon/src/platform/windows/mod.rs`
- create `crates/own-domains-daemon/src/platform/windows/config.rs`
- create `crates/own-domains-daemon/tests/windows_config.rs`

Steps:
1. Write test: for LAN `192.168.1.20` and upstreams `["1.1.1.1"]`, the Windows Corefile binds `127.0.0.1:53` and the LAN on `53`, TTL 0, upstream port 53; the Caddy JSON binds `127.0.0.1:80`/LAN `:80` and `:443`, `protocols` h1/h2, admin `unix/<runtime>/caddy-admin.sock`, no wildcard or UDP listeners.
2. Run `cargo test -p own-domains-daemon --test windows_config`; confirm it fails (module missing).
3. Implement: register `#[cfg(target_os = "windows")] pub mod windows`; constructors `windows_dns_options` / `windows_proxy_options` plus render wrappers in `platform/windows/config.rs`.
4. Run the test; confirm it passes.
5. Run the verification commands; confirm nothing regressed.

Done when: exact Windows artifacts render from tested constructors; macOS suite untouched.

### Task 4: Full verification

Steps:
1. Run `cargo test --workspace --locked`; all green.
2. Run the windows check, lint, and fmt commands; all clean.
3. Commit and push the milestone.

Done when: everything above passes with output shown.
