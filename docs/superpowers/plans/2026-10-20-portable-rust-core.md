# Portable Rust Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a portable Rust library that matches the existing domain/settings behavior and produces native-app-ready Caddy and CoreDNS configurations without replacing the working TypeScript application.

**Architecture:** A Cargo workspace at the repository root contains a focused `own-domains-core` crate. The crate owns serializable models, validation, compatible atomic settings persistence, revision-checked mutations, and pure configuration generation; it has no GUI, IPC, service-manager, or subprocess responsibilities.

**Tech Stack:** Stable Rust, Cargo, Serde/serde_json, UUID, thiserror, and Rust's standard filesystem/network primitives; existing pnpm/Vitest checks remain active.

**Spec:** `docs/superpowers/specs/2026-10-20-native-app-architecture-design.md`

## Global Constraints

- Obtain explicit user approval before adding the proposed Rust production dependencies.
- Preserve compatibility with the existing schema-version-1 `settings.json`; do not discard settings, mapping IDs, revisions, or CA-related paths.
- Preserve the working TypeScript application until a later phase passes behavioral parity tests.
- Keep the Rust core platform-neutral and free of Tauri, IPC, service-manager, shell-command, and elevated-privilege behavior.
- Remove no `registry.own` behavior during this phase; removal occurs only when the native GUI/runtime replaces the browser management path.
- Keep `setup.own` reserved and unavailable for user mappings.
- Use fixed, typed inputs for generated configurations; never interpolate unvalidated settings into shell commands.
- Do not commit, install dependencies, or change host services without explicit user authorization.

## Review Focus

- A schema-valid settings file containing an unknown field must load without data loss on the next save; Task 3 pins compatibility behavior.
- A crash after temporary-file sync but before rename must leave the previous `settings.json` readable and the temporary file recoverable; Task 3 covers the boundary.
- Two writers presenting the same expected revision must not both succeed; Task 4 proves serialized conflict handling.
- An upstream DNS address equal to the LAN address or loopback must be rejected before CoreDNS configuration exists; Tasks 2 and 6 cover both paths.
- Generated Caddy routes must never expose `registry.own`, the future IPC endpoint, or arbitrary upstream addresses; Task 5 pins exact allowed routes.

---

### Task 1: Establish the Cargo workspace and core models

**Files:**
- Create: `Cargo.toml`
- Create: `Cargo.lock`
- Create: `crates/own-domains-core/Cargo.toml`
- Create: `crates/own-domains-core/src/lib.rs`
- Create: `crates/own-domains-core/src/model.rs`
- Create: `crates/own-domains-core/tests/model_compatibility.rs`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: Schema-version-1 JSON defined by `src/shared/types.ts`.
- Produces: `DomainMapping`, `Settings`, `MappingInput`, `MappingPatch`, `ApplyResult`, `ApplyStatus`, `Diagnostic`, and `DiagnosticStatus` with Serde camelCase compatibility; `Settings::default() -> Settings`.

- [ ] **Step 1: Obtain dependency approval**

Request approval for `serde`, `serde_json`, `uuid`, and `thiserror` as runtime dependencies. Do not create or resolve `Cargo.lock` until approval is granted.

- [ ] **Step 2: Write the failing model compatibility test**

In `model_compatibility.rs`, add `deserializes_existing_schema_v1_settings` using the exact existing fields `schemaVersion`, `revision`, `lanAddress`, `upstreamDns`, `mappings`, `upstreamPort`, `tls`, and `enabled`; assert that serialization round-trips the same semantic values and mapping ID. Add `default_settings_match_existing_defaults`; assert revision `0`, blank LAN address, upstreams `1.1.1.1` and `9.9.9.9`, and no mappings.

- [ ] **Step 3: Run the focused test and verify it fails**

Run: `cargo test -p own-domains-core --test model_compatibility`

Expected: FAIL because the workspace/crate or model types do not exist.

- [ ] **Step 4: Create the workspace and model types**

Create a workspace containing `crates/own-domains-core`. In `model.rs`, define the produced types with `Serialize`, `Deserialize`, equality/debug traits, camelCase field serialization, and `schema_version: u32`. Define `ApplyStatus` as `Applied | Pending | Error` and `DiagnosticStatus` as `Ok | Warning | Error` with lowercase wire values.

- [ ] **Step 5: Export the model API and ignore Rust build output**

Export the model types from `lib.rs`, implement `Settings::default()`, and add `/target/` to `.gitignore` without changing existing ignore rules.

- [ ] **Step 6: Run the focused test**

Run: `cargo test -p own-domains-core --test model_compatibility`

Expected: PASS.

- [ ] **Step 7: Run baseline project verification**

Run: `cargo test -p own-domains-core && pnpm test && pnpm run lint && pnpm run typecheck`

Expected: all commands exit `0`.

- [ ] **Step 8: Commit when explicitly authorized**

Stage only Task 1 files and use `feat: establish portable rust core`.

### Task 2: Port hostname, mapping, and network validation

**Files:**
- Create: `crates/own-domains-core/src/error.rs`
- Create: `crates/own-domains-core/src/validation.rs`
- Create: `crates/own-domains-core/tests/validation.rs`
- Modify: `crates/own-domains-core/src/lib.rs`

**Interfaces:**
- Consumes: `DomainMapping`, `MappingInput`, and `Settings` from Task 1.
- Produces: `CoreError`, `CoreResult<T>`, `normalize_hostname(input: &str) -> String`, `validate_hostname(input: &str) -> CoreResult<String>`, `validate_mapping(input: MappingInput) -> CoreResult<DomainMapping>`, `validate_network(lan_address: &str, upstream_dns: &[String]) -> CoreResult<()>`, and `validate_settings(settings: Settings) -> CoreResult<Settings>`.

- [ ] **Step 1: Write failing hostname and mapping tests**

Add tests asserting that `Shop.OWN.` normalizes to `shop.own`, `admin.api.shop.own` succeeds, and empty labels, Unicode, wildcards, IP literals, labels over 63 bytes, names over 253 bytes, `setup.own`, descendants of `setup.own`, ports `0`, `53`, `80`, `443`, `4780`, and `65536` fail. Assert that `registry.own` is accepted by the portable model only as migration data but cannot be created through `validate_mapping`; this preserves old files without permitting new browser-management mappings.

- [ ] **Step 2: Write failing network and settings tests**

Assert that a valid IPv4 LAN address and nonempty IPv4 upstream list succeed; blank/IPv6/malformed LAN addresses, empty upstreams, IPv6 upstreams, `127.0.0.1`, or an upstream matching the LAN address fail. Assert that schema versions other than `1`, duplicate IDs, and duplicate normalized hostnames fail.

- [ ] **Step 3: Run the validation test and verify it fails**

Run: `cargo test -p own-domains-core --test validation`

Expected: FAIL with unresolved validation/error imports.

- [ ] **Step 4: Implement typed validation errors**

Define `CoreError` variants `Validation { message: String }`, `RevisionConflict { expected: u64, actual: u64 }`, `NotFound { entity: &'static str, id: String }`, `DuplicateHostname { hostname: String }`, `CorruptSettings { path: PathBuf, message: String }`, and `Io { operation: &'static str, path: PathBuf, source: std::io::Error }`. Define `type CoreResult<T> = Result<T, CoreError>`.

- [ ] **Step 5: Implement the validation API**

Match the existing ASCII DNS-label rules, single trailing-dot removal, lowercase normalization, `.own` requirement, reserved ports, UUID generation for missing IDs, network forwarding-loop rejection, schema checks, and normalized duplicate detection. Separate loading compatibility from new-mapping validation so legacy `registry.own` data can be read but not newly registered.

- [ ] **Step 6: Run focused and crate tests**

Run: `cargo test -p own-domains-core --test validation && cargo test -p own-domains-core`

Expected: PASS.

- [ ] **Step 7: Commit when explicitly authorized**

Stage only Task 2 files and use `feat: port domain validation to rust`.

### Task 3: Add compatible crash-safe settings persistence

**Files:**
- Create: `crates/own-domains-core/src/store.rs`
- Create: `crates/own-domains-core/tests/store.rs`
- Modify: `crates/own-domains-core/src/lib.rs`
- Modify: `crates/own-domains-core/src/error.rs`

**Interfaces:**
- Consumes: `Settings`, `validate_settings`, `CoreError`, and `CoreResult<T>`.
- Produces: `SettingsStore::new(directory: impl Into<PathBuf>) -> SettingsStore`, `SettingsStore::load(&self) -> CoreResult<Settings>`, `SettingsStore::save(&self, settings: &Settings, expected_revision: Option<u64>) -> CoreResult<()>`, and internal replaceable `FileOps` for deterministic fault tests.

- [ ] **Step 1: Write failing load and compatibility tests**

Assert missing files return `Settings::default()`, existing TypeScript-formatted JSON loads, corrupt JSON and unsupported schemas return `CorruptSettings` without overwriting the source, duplicate IDs/hostnames fail, and unknown top-level fields survive a load/save cycle through a flattened compatibility map.

- [ ] **Step 2: Write failing atomic-save tests**

Assert directory mode is owner-only where Unix permissions apply, the temporary file is flushed before rename, the containing directory is synced after rename on Unix, saved JSON ends with a newline, file mode is owner read/write on Unix, and injected pre-rename failure leaves the previous settings readable plus the temporary file available for recovery.

- [ ] **Step 3: Write the failing revision test**

Save revision `3`, attempt a save with expected revision `2`, and assert `CoreError::RevisionConflict { expected: 2, actual: 3 }` while the file remains unchanged.

- [ ] **Step 4: Run the store test and verify it fails**

Run: `cargo test -p own-domains-core --test store`

Expected: FAIL with unresolved `SettingsStore`.

- [ ] **Step 5: Implement load and compatibility preservation**

Store unknown top-level JSON properties in a flattened `BTreeMap<String, serde_json::Value>` on `Settings`. Reject unknown schema versions but preserve unknown fields for schema version `1` when saving.

- [ ] **Step 6: Implement atomic save**

Write `settings.json.tmp` in the same directory, apply restrictive permissions, flush file contents, atomically rename to `settings.json`, then sync the parent directory where supported. Map every filesystem failure to the operation and exact path in `CoreError::Io`.

- [ ] **Step 7: Run focused and crate tests**

Run: `cargo test -p own-domains-core --test store && cargo test -p own-domains-core`

Expected: PASS.

- [ ] **Step 8: Run TypeScript compatibility tests**

Run: `pnpm test -- tests/server/domains/store.test.ts tests/server/domains/validation.test.ts`

Expected: PASS.

- [ ] **Step 9: Commit when explicitly authorized**

Stage only Task 3 files and use `feat: add rust settings store`.

### Task 4: Add revision-checked serialized domain mutations

**Files:**
- Create: `crates/own-domains-core/src/service.rs`
- Create: `crates/own-domains-core/tests/service.rs`
- Modify: `crates/own-domains-core/src/lib.rs`

**Interfaces:**
- Consumes: `SettingsStore`, validation APIs, `MappingInput`, `MappingPatch`, and `CoreResult<T>`.
- Produces: `DomainService::new(store: SettingsStore) -> DomainService`, `list(&self) -> CoreResult<Settings>`, `create(&self, input: MappingInput, expected_revision: u64) -> CoreResult<Settings>`, `update(&self, id: &str, patch: MappingPatch, expected_revision: u64) -> CoreResult<Settings>`, `remove(&self, id: &str, expected_revision: u64) -> CoreResult<Settings>`, and `update_network(&self, lan_address: String, upstream_dns: Vec<String>, expected_revision: u64) -> CoreResult<Settings>`.

- [ ] **Step 1: Write failing CRUD tests**

Assert create normalizes hostnames and increments the revision once; update preserves ID while changing only supplied fields; remove deletes the target; missing IDs return `NotFound`; duplicate normalized hostnames return `DuplicateHostname`; and every rejected mutation leaves the stored revision and mappings unchanged.

- [ ] **Step 2: Write failing network mutation tests**

Assert valid network settings replace the LAN/upstream values and increment once, while every invalid/looping network input leaves storage unchanged.

- [ ] **Step 3: Write the failing concurrency test**

Start two threads with expected revision `0` against the same service barrier. Assert exactly one create succeeds, exactly one returns `RevisionConflict`, and the resulting file contains one mapping at revision `1`.

- [ ] **Step 4: Run the service test and verify it fails**

Run: `cargo test -p own-domains-core --test service`

Expected: FAIL with unresolved `DomainService`.

- [ ] **Step 5: Implement serialized mutations**

Protect each load/validate/save transaction with a shared mutex scoped to the canonical settings path. Recheck `expected_revision` inside the critical section, increment exactly once, and return the persisted `Settings` snapshot. Do not invoke DNS, proxy, subprocess, or IPC behavior.

- [ ] **Step 6: Run focused and crate tests**

Run: `cargo test -p own-domains-core --test service && cargo test -p own-domains-core`

Expected: PASS, including the repeated concurrency test under `cargo test -p own-domains-core --test service concurrent_writers -- --test-threads=1`.

- [ ] **Step 7: Commit when explicitly authorized**

Stage only Task 4 files and use `feat: add rust domain service`.

### Task 5: Generate native-runtime Caddy configuration

**Files:**
- Create: `crates/own-domains-core/src/proxy.rs`
- Create: `crates/own-domains-core/tests/proxy.rs`
- Modify: `crates/own-domains-core/src/lib.rs`

**Interfaces:**
- Consumes: validated `Settings`.
- Produces: `ProxyConfigOptions { admin_listen: String, listen_addresses: Vec<Ipv4Addr>, http_port: u16, https_port: u16, setup_port: u16 }`, `ProxyConfigOptions::for_settings(settings: &Settings) -> CoreResult<Self>`, and `build_proxy_config(settings: &Settings, options: &ProxyConfigOptions) -> CoreResult<serde_json::Value>`.

- [ ] **Step 1: Write failing route tests**

Assert enabled HTTP mappings proxy to `127.0.0.1:<port>`, enabled TLS mappings redirect HTTP to the exact HTTPS hostname and proxy on HTTPS, disabled mappings have no route, nested names remain exact, and unknown hosts return `404`.

- [ ] **Step 2: Write failing exposure tests**

Assert the HTTP setup route matches only `setup.own` and the configured LAN address, proxies only to `127.0.0.1:<setup_port>`, and no serialized value contains `registry.own`, port `4780`, an IPC path, or a user-provided upstream host. Assert Caddy admin defaults to loopback and listeners include only loopback plus the selected LAN IPv4 address.

- [ ] **Step 3: Run the proxy test and verify it fails**

Run: `cargo test -p own-domains-core --test proxy`

Expected: FAIL with unresolved proxy APIs.

- [ ] **Step 4: Implement pure Caddy JSON generation**

Port exact-host HTTP/HTTPS behavior from `src/server/proxy/config.ts` but omit the management route. Keep HSTS absent, disable automatic redirects, preserve request paths through Caddy reverse-proxy handlers, and issue internal certificates only for enabled TLS mappings.

- [ ] **Step 5: Run Rust and TypeScript proxy tests**

Run: `cargo test -p own-domains-core --test proxy && pnpm test -- tests/server/proxy/config.test.ts`

Expected: PASS. Intentional `registry.own` differences must be isolated to Rust assertions; do not change the live TypeScript behavior in this phase.

- [ ] **Step 6: Commit when explicitly authorized**

Stage only Task 5 files and use `feat: generate caddy config in rust`.

### Task 6: Generate authoritative CoreDNS configuration

**Files:**
- Create: `crates/own-domains-core/src/dns.rs`
- Create: `crates/own-domains-core/tests/dns.rs`
- Create: `crates/own-domains-core/tests/fixtures/Corefile.expected`
- Modify: `crates/own-domains-core/src/lib.rs`

**Interfaces:**
- Consumes: validated `Settings`.
- Produces: `DnsConfigOptions { listen_addresses: Vec<Ipv4Addr>, port: u16, upstream_port: u16 }`, `DnsConfigOptions::for_settings(settings: &Settings) -> CoreResult<Self>`, and `build_corefile(settings: &Settings, options: &DnsConfigOptions) -> CoreResult<String>`.

- [ ] **Step 1: Write the failing Corefile fixture test**

For LAN address `192.168.1.20`, upstreams `1.1.1.1` and `9.9.9.9`, loopback/LAN listeners, port `1053`, and upstream port `5353`, assert exact fixture output that serves `.own` A records as `192.168.1.20`, returns no AAAA record, keeps the `.own` zone authoritative, and forwards only non-`.own` queries to the two explicit upstream endpoints.

- [ ] **Step 2: Write failing safety tests**

Assert empty/invalid/looping upstreams fail before output, only loopback and the selected LAN address are accepted as listeners, `registry.own` receives no special record, `setup.own` resolves through the same authoritative `.own` rule while active, and output ends with one newline.

- [ ] **Step 3: Run the DNS test and verify it fails**

Run: `cargo test -p own-domains-core --test dns`

Expected: FAIL with unresolved DNS APIs.

- [ ] **Step 4: Implement deterministic CoreDNS generation**

Generate one authoritative `.own` server block and one public forwarding block using only validated IPv4 values and numeric ports. Keep directive ordering deterministic so the fixture is reviewable. Do not execute or install CoreDNS.

- [ ] **Step 5: Run focused and crate tests**

Run: `cargo test -p own-domains-core --test dns && cargo test -p own-domains-core`

Expected: PASS.

- [ ] **Step 6: Commit when explicitly authorized**

Stage only Task 6 files and use `feat: generate coredns config in rust`.

### Task 7: Prove TypeScript-to-Rust parity and document the boundary

**Files:**
- Create: `tests/fixtures/core-parity/settings-valid.json`
- Create: `tests/fixtures/core-parity/settings-invalid-duplicate.json`
- Create: `crates/own-domains-core/tests/parity.rs`
- Create: `docs/native-core.md`
- Modify: `package.json`

**Interfaces:**
- Consumes: all public APIs produced by Tasks 1–6 and the existing TypeScript test suite.
- Produces: a `verify:native-core` pnpm script and documented compatibility/runtime boundaries for the next daemon plan.

- [ ] **Step 1: Add shared compatibility fixtures**

Create one representative schema-version-1 settings file with HTTP, HTTPS, disabled, and nested mappings plus one duplicate-normalized-hostname rejection fixture. Use stable UUID strings and LAN address `192.168.1.20`.

- [ ] **Step 2: Write the failing Rust parity test**

Load the shared valid fixture, assert exact normalized model values and expected Caddy/CoreDNS semantics, save/reload it without semantic loss, and assert the invalid fixture fails with duplicate-hostname classification.

- [ ] **Step 3: Run the parity test and verify its initial result**

Run: `cargo test -p own-domains-core --test parity`

Expected: FAIL until fixture handling exposes any remaining compatibility gaps; if it passes immediately, record that the preceding APIs already satisfy the fixture and continue without manufacturing a failure.

- [ ] **Step 4: Close only demonstrated parity gaps**

Modify the owning Task 1–6 module for each failing assertion; do not add daemon, IPC, CLI, Tauri, service-manager, or host-mutation behavior.

- [ ] **Step 5: Document the phase boundary**

In `docs/native-core.md`, record exported modules, settings compatibility, error classifications, atomic-write guarantees, Caddy/CoreDNS generation behavior, intentionally retained TypeScript runtime, and the exact responsibilities deferred to the daemon/IPC/CLI plan.

- [ ] **Step 6: Add one aggregate verification command**

Add `verify:native-core` using pnpm to invoke `cargo test -p own-domains-core`, then the existing test, lint, typecheck, and build scripts. Use pnpm's existing script conventions and do not add a package dependency.

- [ ] **Step 7: Run complete verification**

Run: `pnpm run verify:native-core`

Expected: Rust tests, 98 or more existing Vitest tests, ESLint, TypeScript checking, and the Vite production build all pass.

- [ ] **Step 8: Run formatting checks**

Run: `cargo fmt --all -- --check && git diff --check`

Expected: both commands exit `0`.

- [ ] **Step 9: Commit when explicitly authorized**

Stage only Task 7 and any demonstrated parity-fix files and use `test: verify native core parity`.

## Phase completion gate

Phase 1 is complete only when:

- `pnpm run verify:native-core` passes from a clean checkout.
- Existing schema-version-1 settings load without semantic loss.
- Rust validation and mutation behavior is pinned by parity fixtures.
- Caddy output contains no browser management route.
- CoreDNS output is deterministic, authoritative for `.own`, and rejects forwarding loops.
- No TypeScript production runtime file has been removed or redirected.
- No host service or resolver configuration has been changed.

After this gate, write the daemon/IPC/CLI implementation plan against the actual `own-domains-core` public API.
