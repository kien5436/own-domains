# Daemon, IPC, and CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a single-writer Rust daemon, versioned local IPC, revisioned events, and an independent `own-domains` CLI without activating host DNS or proxy services yet.

**Architecture:** A protocol crate defines stable JSON-line messages shared by clients and the daemon. The daemon holds an exclusive process lock, delegates settings mutations to `own-domains-core`, publishes revisioned snapshots, and drives an injected `RuntimeBackend`; the CLI uses Unix-socket IPC when the daemon is live and a locked one-shot command handler for offline mutations.

**Tech Stack:** Stable Rust 1.90+, Serde/serde_json, standard Unix domain sockets and file locking, and Clap 4 derive for CLI parsing.

**Spec:** `docs/superpowers/specs/2026-10-20-native-app-architecture-design.md`

## Global Constraints

- Obtain explicit approval before adding Clap as a Rust production dependency.
- Keep the existing TypeScript backend and CLI active; Phase 2 adds parallel Rust executables and removes nothing.
- Only the daemon or a process holding the same exclusive state lock may write `settings.json`.
- Use permission-restricted Unix sockets on macOS/Linux; Windows named pipes remain in the platform phase.
- Do not expose management IPC through TCP, HTTP, Caddy, DNS, or LAN interfaces.
- `start` must return `runtimeUnavailable` until a real platform `RuntimeBackend` is supplied; never claim DNS/Caddy activation from a no-op backend.
- Offline mutations may persist settings but must return `saved; runtime stopped` and must not start services.
- Preserve schema-version-1 settings compatibility and all `own-domains-core` behavior.
- Do not change host services, resolver configuration, trust stores, or privileged files in this phase.
- Do not commit, push, or add dependencies beyond the approved plan without explicit authorization.

## Review Focus

- Two daemon processes racing for the same state directory must produce exactly one lock owner and one actionable `alreadyRunning` error; Task 2 tests this.
- A partial, oversized, malformed, or version-incompatible IPC frame must close only that client and leave the daemon healthy; Task 4 tests this.
- A subscriber that disconnects between revisions must recover from a fresh snapshot without assuming events are complete; Tasks 1 and 3 test this.
- A CLI offline mutation racing a daemon startup must never create two settings writers; Tasks 2 and 6 test this.
- `stop` or failed runtime shutdown must distinguish “daemon stopped” from “routing may still be active”; Tasks 3 and 6 test this.

---

### Task 1: Define the versioned IPC contract

**Files:**
- Create: `crates/own-domains-protocol/Cargo.toml`
- Create: `crates/own-domains-protocol/src/lib.rs`
- Create: `crates/own-domains-protocol/src/message.rs`
- Create: `crates/own-domains-protocol/tests/wire.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `Settings`, `MappingInput`, `MappingPatch`, `Diagnostic`, and `ApplyResult` from `own-domains-core`.
- Produces: `PROTOCOL_VERSION: u16 = 1`; `RequestEnvelope { protocol_version, request_id, command }`; `ResponseEnvelope { protocol_version, request_id, result }`; `EventEnvelope { protocol_version, sequence, event }`; `Command`, `CommandResult`, `ProtocolError`, `RuntimeStatus`, `StateSnapshot`, and `Event`.

- [ ] Write failing JSON round-trip tests for every command: handshake, snapshot, subscribe, start, stop, list, create, update, remove, update-network, and doctor. Assert camelCase wire fields and `type`/`payload` tagged command names.
- [ ] Add failing compatibility tests asserting protocol version `0` or `2` maps to `ProtocolError::IncompatibleVersion { supported: 1, received }`, unknown command names fail without fallback, and response `requestId` exactly echoes the request.
- [ ] Add a failing resynchronization test asserting `StateSnapshot` includes settings revision, runtime status, desired/applied revisions, diagnostics, and latest event sequence.
- [ ] Run `cargo test -p own-domains-protocol`; expect failure because the crate/types do not exist.
- [ ] Create the crate and exact produced types. Use `serde(tag = "type", content = "payload", rename_all = "camelCase")` for commands/results/events and camelCase structs.
- [ ] Run `cargo test -p own-domains-protocol && cargo test --workspace`; expect all tests to pass.
- [ ] Commit when explicitly authorized with `feat: define daemon ipc protocol`.

### Task 2: Add exclusive state-directory ownership

**Files:**
- Create: `crates/own-domains-daemon/Cargo.toml`
- Create: `crates/own-domains-daemon/src/lib.rs`
- Create: `crates/own-domains-daemon/src/process-lock.rs`
- Create: `crates/own-domains-daemon/tests/process-lock.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: a state directory path.
- Produces: `ProcessLock::acquire(directory: &Path) -> Result<ProcessLock, DaemonError>` holding an exclusive OS file lock on `own-domains.lock` until drop; `DaemonError::AlreadyRunning { path }` and typed I/O errors.

- [ ] Write a failing test that acquires the same state directory from two independent child processes and asserts one succeeds while the other returns `alreadyRunning`; a different directory must succeed concurrently.
- [ ] Write failing tests for owner-only directory/file permissions on Unix, automatic unlock after normal exit, and automatic unlock after the lock-holder child is killed.
- [ ] Run `cargo test -p own-domains-daemon --test process-lock`; expect failure because the crate/API does not exist.
- [ ] Raise workspace `rust-version` to `1.90`, implement the lock with `std::fs::File::try_lock`, and retain the file handle for the guard lifetime. Never use a PID-only stale lock file.
- [ ] Run the focused test 20 times, then `cargo test --workspace`; expect all runs to pass.
- [ ] Commit when explicitly authorized with `feat: enforce daemon process ownership`.

### Task 3: Implement daemon state, commands, and events

**Files:**
- Create: `crates/own-domains-daemon/src/handler.rs`
- Create: `crates/own-domains-daemon/src/runtime.rs`
- Create: `crates/own-domains-daemon/src/events.rs`
- Create: `crates/own-domains-daemon/tests/handler.rs`
- Modify: `crates/own-domains-daemon/src/lib.rs`

**Interfaces:**
- Consumes: `DomainService`, protocol commands, and the Task 2 lock.
- Produces: `RuntimeBackend` with `start(&self, settings: &Settings) -> Result<ApplyResult, RuntimeError>`, `stop(&self) -> Result<(), RuntimeError>`, `inspect(&self) -> Result<RuntimeStatus, RuntimeError>`, and `diagnose(&self, settings: &Settings) -> Result<Vec<Diagnostic>, RuntimeError>`; `UnavailableRuntimeBackend`; `DaemonState::new`; `DaemonState::handle(request) -> ResponseEnvelope`; and `EventBroker::subscribe(after_sequence) -> Subscription`.

- [ ] Write failing command tests for snapshot/list and every domain/network mutation. Assert core errors map to stable protocol codes, successful mutations emit one settings event containing the new revision, and rejected mutations emit none.
- [ ] Write failing runtime tests asserting `UnavailableRuntimeBackend` makes start return `runtimeUnavailable`, stop never reports success when backend shutdown fails, and runtime transitions emit `starting`, `running`, `stopping`, or `error` events in order.
- [ ] Write failing subscription tests for monotonic sequence numbers, multiple subscribers, disconnect cleanup, and `after_sequence` older than retained history returning `resyncRequired` followed by a fresh snapshot.
- [ ] Run `cargo test -p own-domains-daemon --test handler`; expect unresolved handler/runtime APIs.
- [ ] Implement a bounded 256-event history and subscriber channels. Serialize command handling so core mutations and event publication have one ordering.
- [ ] Run `cargo test -p own-domains-daemon && cargo test --workspace`; expect all tests to pass.
- [ ] Commit when explicitly authorized with `feat: add daemon command state`.

### Task 4: Serve authenticated local Unix IPC

**Files:**
- Create: `crates/own-domains-daemon/src/ipc.rs`
- Create: `crates/own-domains-daemon/src/bin/own-domains-daemon.rs`
- Create: `crates/own-domains-daemon/tests/ipc.rs`
- Modify: `crates/own-domains-daemon/src/lib.rs`

**Interfaces:**
- Consumes: `DaemonState` and protocol envelopes.
- Produces: `IpcServer::bind(socket_path, state) -> Result<IpcServer, DaemonError>`, `IpcServer::run()`, `IpcServer::shutdown()`, and daemon binary flags `--state-dir`, `--socket`, and `--settings-only`.

- [ ] Write failing integration tests proving the socket and its parent directory are owner-only, the socket is removed on clean shutdown, and a second bind cannot replace a live socket.
- [ ] Write failing frame tests for split reads, multiple newline-delimited requests, invalid UTF-8, malformed JSON, unknown protocol version, EOF without newline, and a 1 MiB maximum frame. Assert each bad client is disconnected while a subsequent healthy client succeeds.
- [ ] Write a failing subscription test that receives an initial snapshot, observes a mutation from a second connection, disconnects, reconnects with its last sequence, and receives either missed events or `resyncRequired` plus snapshot.
- [ ] Run `cargo test -p own-domains-daemon --test ipc`; expect unresolved IPC APIs.
- [ ] Implement blocking Unix-socket accept plus one scoped thread per client. Bind only after acquiring `ProcessLock`; reject symlink/non-socket stale paths and remove only a socket owned by this server.
- [ ] Run `cargo test -p own-domains-daemon --test ipc && cargo test --workspace`; expect all tests to pass.
- [ ] Commit when explicitly authorized with `feat: serve local daemon ipc`.

### Task 5: Add the shared IPC client

**Files:**
- Create: `crates/own-domains-client/Cargo.toml`
- Create: `crates/own-domains-client/src/lib.rs`
- Create: `crates/own-domains-client/src/unix.rs`
- Create: `crates/own-domains-client/tests/client.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: protocol envelopes and a Unix socket path.
- Produces: `Client::connect(path) -> Result<Client, ClientError>`, `Client::request(command) -> Result<CommandResult, ClientError>`, `Client::subscribe(after_sequence) -> Result<EventStream, ClientError>`, and errors `DaemonUnavailable`, `IncompatibleVersion`, `Protocol`, and `Io`.

- [ ] Write failing tests for handshake-before-command, request ID correlation, daemon-unavailable classification, incompatible versions, server disconnect mid-frame, request timeout, and concurrent clients.
- [ ] Write failing event tests proving snapshots and events decode independently of request responses and reconnect resumes from the supplied sequence.
- [ ] Run `cargo test -p own-domains-client`; expect failure because the crate/API does not exist.
- [ ] Implement a fresh request connection per synchronous command and a dedicated long-lived connection for subscriptions. Use a five-second default timeout and a configurable timeout constructor for tests.
- [ ] Run `cargo test -p own-domains-client && cargo test --workspace`; expect all tests to pass.
- [ ] Commit when explicitly authorized with `feat: add daemon ipc client`.

### Task 6: Build the independent Rust CLI and offline mutation path

**Files:**
- Create: `crates/own-domains-cli/Cargo.toml`
- Create: `crates/own-domains-cli/src/lib.rs`
- Create: `crates/own-domains-cli/src/main.rs`
- Create: `crates/own-domains-cli/src/command.rs`
- Create: `crates/own-domains-cli/tests/cli.rs`
- Modify: `Cargo.toml`
- Modify: `package.json`
- Create: `docs/cli.md`

**Interfaces:**
- Consumes: `own-domains-client`, `own-domains-daemon`, protocol commands, and core models.
- Produces: binary `own-domains`; commands `start`, `stop`, `status`, `list`, `add`, `edit`, `enable`, `disable`, `remove`, and `doctor`; global flags `--state-dir`, `--socket`, and `--json`.

- [ ] Obtain explicit approval and add Clap 4 with derive support to `own-domains-cli`.
- [ ] Write failing parser tests for the complete command surface, required hostname/port/revision values, `--https`, JSON output, help, unknown flags, and reserved/out-of-range ports delegated to core validation.
- [ ] Write failing live-daemon tests asserting list/mutations use IPC, stale revisions return exit code `3`, validation returns `2`, daemon/protocol failure returns `4`, runtime failure returns `5`, and success returns `0`.
- [ ] Write failing offline tests asserting list/status report stopped, mutations acquire the Task 2 lock and persist through an in-process settings-only `DaemonState`, output `saved; runtime stopped`, and never call `RuntimeBackend::start`. Race offline mutation against daemon lock acquisition and assert only the lock winner writes.
- [ ] Write failing lifecycle tests asserting `start` spawns the daemon binary and waits for a successful handshake but returns runtime-unavailable until a platform backend exists; `stop` sends the stop command and waits for socket disappearance without killing an unverified PID.
- [ ] Run `cargo test -p own-domains-cli`; expect parser/runner APIs to be absent.
- [ ] Implement parsing separately from command execution. Human output goes to stdout, actionable errors to stderr, and `--json` emits one protocol-compatible JSON value with no prose.
- [ ] Add `test:native-daemon` to run all Rust workspace tests; document commands, exit codes, offline behavior, and the intentional runtime-unavailable start result.
- [ ] Run `cargo test --workspace`, CLI integration tests, `pnpm run verify:native-core`, `pnpm run lint`, and `pnpm run typecheck`; expect all commands to pass.
- [ ] Commit when explicitly authorized with `feat: add native daemon cli`.

## Phase completion gate

Phase 2 is complete only when:

- One process exclusively owns each state directory and crash release is proven with child processes.
- Versioned local IPC passes malformed-frame, permission, reconnect, and resynchronization tests.
- Live and offline CLI mutations share `DaemonState`/`DomainService` behavior and cannot race as two writers.
- Runtime start cannot claim success without a real backend; stop failures remain visible.
- The TypeScript product remains operational and all existing verification passes.
- No TCP management listener or host service mutation has been introduced.

After this gate, write the Tauri GUI migration plan against the actual protocol/client APIs.
