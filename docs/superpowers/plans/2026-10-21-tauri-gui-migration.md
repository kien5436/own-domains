# Tauri GUI Migration Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task after review. Steps use checkbox (`- [ ]`) syntax for tracking. Do not commit or add production dependencies without explicit authorization.

**Goal:** Run the existing management UI inside Tauri with daemon-backed mutations, synchronized state, and native window/tray lifecycle.

**Architecture:** A thin Rust desktop shell calls `own-domains-client`; the WebView receives typed commands and snapshots through a restricted bridge. One frontend store owns the authoritative snapshot and subscription lifecycle. Keep the working TypeScript product until the platform runtime and setup endpoint reach parity.

**Tech Stack:** Rust 1.90+, Tauri 2, React, TypeScript, Vite, Tailwind, Vitest, and the existing Unix IPC client. Windows transport and host activation remain Phase 4.

**Spec:** `docs/superpowers/specs/2026-10-20-native-app-architecture-design.md`

## Global Constraints

- Only the daemon or the existing locked CLI offline path writes settings. The desktop never opens a settings writer.
- No TCP management IPC, remote WebView management access, arbitrary executable invocation, or new privileged operations.
- Preserve schema-version-1 settings and CA identity. Do not silently move data or choose a new empty state directory.
- Obtain approval for `tauri`, `tauri-plugin-single-instance`, and `@tauri-apps/api` before adding them; include `tauri-build` and `@tauri-apps/cli` in the dependency proposal with exact compatible versions.
- Runtime activation remains explicitly unavailable with `UnavailableRuntimeBackend`; saving settings must never imply routing works.
- Closing the window hides it. Quit requests daemon shutdown and exits only after confirmed success; failure remains visible and retryable.
- Native configuration contains no `registry.own` route. Retain legacy TypeScript entry points until Phase 4 parity permits their removal.
- Never auto-restart the daemon after a user explicitly stops it through the CLI. Reconnection and activation are separate operations.

## Evidence and Scope

Phase 2 implementation is present through commit `aeaef21`; its old unchecked plan boxes are not evidence that implementation is missing. Fresh verification results must be recorded separately before declaring its completion gate passed.

Planning baseline on 2026-10-21: `pnpm run lint` and `git diff --check` passed. `pnpm test` failed to load all 26 suites because the installed `@testing-library/dom/package.json` was invalid; no tests ran. Typecheck and build failed on missing `screen` exports in four existing React tests. Rust workspace verification was started but its completion was not confirmed when this plan was written. Resolve and rerun these baseline checks before implementation. Remote freshness is also unconfirmed because `git fetch --all --prune` failed GitHub SSH authentication.

- `Client::connect(&Path)`, `Client::request(&self, Command)`, and `Client::subscribe(&self, Option<u64>)` are synchronous Unix-only APIs. `EventStream::recv(&mut self)` blocks with the configured socket timeout.
- `StateSnapshot` carries settings, runtime status, desired/applied revisions, diagnostics, and latest event sequence. Mutation results and snapshots are distinct; adapters must not fabricate successful apply results.
- Protocol struct fields use camelCase, but enum payload fields such as `expected_revision` and `after_sequence` currently use snake_case. Generate fixture-backed frontend types matching actual serialization; do not silently change protocol version 1.
- Current HTTP consumers include domain APIs, status APIs, and direct calls inside `NetworkSettings`. Replacing only the domain API leaves management HTTP active.
- CLI defaults to `.own-domains` relative to the current directory. The initial desktop must require an explicit absolute state directory, supplied by its launcher, and use the matching socket. Shared installed defaults and migration belong to Phase 4.
- The setup HTTP handler is currently owned by the TypeScript server. Removing that server now would also remove phone onboarding. Final retirement therefore depends on Phase 4's daemon-owned replacement and lifecycle verification.

## Review Focus

1. CLI mutation between initial snapshot and subscription must not be missed; Task 3 tests subscription-first initialization and resynchronization.
2. A delayed response must not replace newer state; Task 3 tests snapshot ordering and daemon restart generations.
3. CLI stop while the GUI is hidden must not trigger automatic runtime restart; Task 4 tests disconnect without activation.
4. Partial shutdown must keep the application available to report/retry cleanup; Task 4 tests failed Quit.
5. Launching from another directory must not select an empty registry; Task 1 tests explicit state-directory resolution.
6. Registration clicked during an unrelated action must wait, submit with the revision that is current when it runs, and cancel cleanly on a terminal disconnect; Task 6 tests the queued path and latest-revision use.

---

### Task 1: Add a restricted desktop command bridge

**Files:** Create `crates/own-domains-desktop/{Cargo.toml,build.rs,tauri.conf.json}`, `crates/own-domains-desktop/capabilities/main.json`, `crates/own-domains-desktop/src/{lib.rs,main.rs,bridge.rs}`, `crates/own-domains-desktop/tests/bridge.rs`, `src/web/lib/native-protocol.ts`, and `tests/fixtures/ipc/*.json`. Modify `Cargo.toml`, `package.json`, and `crates/own-domains-protocol/tests/wire.rs`.

**Interfaces:** `DesktopBridge::connect(socket: &Path) -> Result<DesktopBridge, BridgeError>`; `DesktopBridge::request(command: Command) -> Result<CommandResult, BridgeError>`. `BridgeError` is a serializable tagged error preserving protocol errors and distinguishing unavailable, incompatible, transport, and invalid-message failures. Expose Tauri command `daemon_request(command: Command)` only to the local main window; reject handshake/subscribe through this command because the bridge owns them.

- [ ] Confirm production dependency approval and compatible pinned versions against official Tauri documentation before installation.
- [ ] Add wire fixtures covering all frontend commands, results, snapshots, events, and errors, generated by Rust serialization and checked by TypeScript tests. Include `expected_revision` and `after_sequence` assertions.
- [ ] Write `bridge.rs` tests proving revision conflicts survive serialization, malformed/unavailable IPC returns an error, requests use the configured socket, and relative/missing state-directory arguments fail without creating settings.
- [ ] Run `cargo test -p own-domains-desktop --test bridge`; expect failure before bridge implementation.
- [ ] Implement the shell and bridge using blocking workers for synchronous IPC. Restrict commands/capabilities to the main local WebView; omit filesystem, shell, HTTP, and remote-origin capabilities. Use local bundled assets in release and a loopback Vite URL only in development.
- [ ] Run bridge and wire tests; expect all assertions to pass. Verify a development WebView can request a snapshot without any management HTTP request.

### Task 2: Make subscriptions cancellable and forward them to the WebView

**Files:** Modify `crates/own-domains-client/src/unix.rs` and `crates/own-domains-client/tests/client.rs`. Create `crates/own-domains-desktop/src/subscription.rs` and `crates/own-domains-desktop/tests/subscription.rs`. Modify desktop `src/lib.rs` and capabilities.

**Interfaces:** Add `EventStream::cancellation_handle(&self) -> Result<EventStreamCancellation, ClientError>` and `EventStreamCancellation::cancel(&self) -> Result<(), ClientError>` using a cloned socket and shutdown to interrupt reads. Desktop commands `daemon_subscribe(after_sequence: Option<u64>) -> Result<u64, BridgeError>` and `daemon_unsubscribe(subscription_id: u64) -> Result<(), BridgeError>` manage one stream per window. Emit `daemon-event` payload `{ subscriptionId, envelope }` and `daemon-disconnected` payload `{ subscriptionId, error }`.

- [ ] Write tests that cancel an idle reader promptly, replace a subscription without leaking its worker, close a window and release its stream, reject incompatible event versions, and preserve split frames through idle timeouts.
- [ ] Run `cargo test -p own-domains-client` and desktop subscription tests; expect the new tests to fail.
- [ ] Implement cancellation and frame buffering. Treat idle timeouts as idle rather than daemon failure; never discard partial bytes on timeout. Validate version and maximum frame length before forwarding.
- [ ] Implement subscription ownership and cleanup in the shell. Register frontend listeners before invoking subscribe, buffering events until its returned ID is known.
- [ ] Run client and subscription tests; expect all to pass, including clean worker termination without a five-second wait.

### Task 3: Replace native UI HTTP calls with shared snapshot state

**Files:** Create `src/web/lib/native-client.ts`, `src/web/features/runtime/{runtime-store.ts,use-runtime.ts}`, and matching tests under `tests/web/lib/` and `tests/web/features/runtime/`. Modify `src/web/app/App.tsx`, `src/web/features/domains/{api.ts,use-domains.ts}`, `src/web/features/status/{api.ts,use-status.ts,StatusPanel.tsx}`, `src/web/features/setup/NetworkSettings.tsx`, `src/web/main.tsx`, and `tests/web/App.test.tsx`.

**Interfaces:** `NativeClient.request(command: NativeCommand): Promise<NativeCommandResult>` and `NativeClient.subscribe(afterSequence: number | null, onEvent, onDisconnect): Promise<() => Promise<void>>` wrap Task 1/2. `RuntimeStore` exposes `getSnapshot`, `subscribe`, `connect`, `disconnect`, and `mutate(command): Promise<void>`; `useRuntime(store)` consumes it with React's external-store subscription API. State includes `snapshot: StateSnapshot | null`, connection status, pending mutation state, and typed error.

- [ ] Write tests for initial snapshot, all mapping operations, network changes, stale-revision reload without retrying the write, and CLI mutations updating both domain and status views.
- [ ] Add tests for a mutation during initial connection, sequence gaps, `resyncRequired`, duplicate events, old snapshots arriving late, daemon restart resetting event sequence, and listener cleanup on unmount. Use subscription generation plus event sequence to reject stale callbacks.
- [ ] Add native-mode tests where `fetch` throws on any management request; domain, network, and status operations must still work. Assert runtime-unavailable and stopped/pending states never render as applied.
- [ ] Run `pnpm test`; expect new tests to fail before implementation.
- [ ] Implement the store with subscription-first initialization. Refresh a full snapshot for change notifications; coalesce reads and reject older snapshots within a generation. On reconnect begin a new generation and accept an authoritative fresh snapshot before enabling writes.
- [ ] Keep presentation components shared. Inject a native application adapter from the desktop entry path; retain the existing legacy adapter for the working TypeScript product until retirement. Native execution must have no fallback to HTTP.
- [ ] Run `pnpm test`, `pnpm run lint`, `pnpm run typecheck`, and `pnpm run build`; expect all to pass.

### Task 4: Implement daemon launch and native lifecycle

**Files:** Create `crates/own-domains-client/src/lifecycle.rs`, `crates/own-domains-client/tests/lifecycle.rs`, `crates/own-domains-desktop/src/lifecycle.rs`, and `crates/own-domains-desktop/tests/lifecycle.rs`. Modify client exports, `crates/own-domains-cli/src/command.rs`, CLI integration tests, and desktop configuration/entry points.

**Interfaces:** Shared `connect_or_launch(state_dir: &Path, socket: &Path, daemon_executable: &Path, timeout: Duration) -> Result<Client, LifecycleError>` and `stop_and_wait(client: &Client, socket: &Path, timeout: Duration) -> Result<(), LifecycleError>`. Desktop lifecycle exposes launch, show, hide, retry-start, and quit operations over an injectable client/window abstraction.

- [ ] Write tests for simultaneous GUI/CLI startup, incompatible existing daemon without spawning another, missing executable, launch timeout, and successful handshake before Start. Reuse the process lock to decide the winning daemon.
- [ ] Write lifecycle tests proving close hides without Stop, second launch focuses the original window, Quit sends Stop then waits for confirmed daemon disappearance, failed Stop keeps the GUI/tray alive, and CLI stop never triggers an automatic relaunch.
- [ ] Run focused client, CLI, and desktop lifecycle tests; expect new cases to fail.
- [ ] Extract existing CLI lifecycle behavior into the shared client module and use it from both clients. Spawn only a trusted configured/bundled daemon executable with argument arrays; never accept its path from the WebView. Do not kill an unverified PID.
- [ ] Register the single-instance plugin before daemon startup and add tray Show/Quit actions. Launch requests Start once and displays runtime-unavailable truthfully; explicit retry is required after external Stop. Restore the window to display Quit failures.
- [ ] Run workspace tests and manually verify launch, close, restore, second instance, CLI mutation, and Quit on macOS. Record manual checks as pending where the host is unavailable.

### Task 5: Document the migration boundary and prove native isolation

**Files:** Create `docs/desktop.md` and `tests/web/native-management.test.tsx`. Modify `README.md`, `docs/cli.md`, and `docs/superpowers/plans/2026-10-20-native-app-migration-roadmap.md`.

- [ ] Add regression tests proving every native UI management flow uses the bridge and never calls `/api/*`; keep Rust Caddy/DNS tests proving the native generated configuration omits `registry.own`.
- [ ] Run the focused tests before implementing remaining integration wiring; expect failures for any uncovered HTTP consumer.
- [ ] Document development/build commands, explicit shared state-directory selection, daemon executable packaging, unavailable runtime status, and supported Unix-only transport. Include manual lifecycle acceptance steps and dependency/version evidence.
- [ ] Record the Phase 4 cutover requirements: daemon-owned read-only `setup.own` endpoint, public certificate/profile/fingerprint only, explicit route/method allowlist, no management/private-key/path exposure, real service stop verification, and settings/CA preservation. Only after these pass may legacy browser management, `registry.own`, and TypeScript backend/CLI entry points be removed with explicit deletion authorization.
- [ ] Run `cargo test --workspace`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `pnpm test`, `pnpm run lint`, `pnpm run typecheck`, and `pnpm run build`. Run the desktop bundle build on macOS and record actual output. Do not infer native GUI success from jsdom tests.

### Task 6: Keep registration available during unrelated actions

**Status:** Planned follow-up from the 2026-10-22 GUI check. Do not implement without explicit authorization.

**Problem:** `NativeApp` derives `busy = state.busy || acting || state.connection !== 'connected'`, and `DomainForm` currently disables its submit button whenever `busy` is true. The local `submitting` state fixed the incorrect “Registering…” label, but Start, Stop, Quit, a domain edit, a TLS change, or a network save still greys out Register for the whole action. Registration should remain available; only the mutation should wait for the active action and use the revision that is current when the mutation is sent.

**Files:** Modify `src/web/features/domains/DomainForm.tsx`, `src/web/app/NativeApp.tsx`, `src/web/app/App.tsx`, `tests/web/DomainForm.test.tsx`, `tests/web/native-management.test.tsx`, and `docs/desktop.md`. No Rust, protocol, state-directory, privilege, or dependency changes.

**Interfaces:**

- `DomainFormProps` becomes `{ busy: boolean; disabled?: boolean; onSubmit: (input: MappingInput) => Promise<void> }`. `busy` means “wait before submitting”; `disabled` means “the connection is terminally unavailable and queued work must be cancelled.” Existing callers may omit `disabled`.
- The button is disabled only by `disabled || submitting`. `busy` must not disable it.
- `NativeApp` passes `busy={busy}` and `disabled={state.connection !== 'connected' && !acting}`. The `!acting` exemption keeps Register available while Start/Stop is in progress, including Start's reconnect; a terminal Stop disconnect then cancels the queue.
- The native create callback reads `store.getSnapshot().snapshot?.settings.revision ?? 0` when it is invoked. Do not use the render-time `revision`, which can be stale after a queued wait.
- `App` passes `disabled={domains.busy}` so the legacy browser dashboard keeps its existing all-actions-disabled behavior. The queue behavior is scoped to the native app.

**Behavior contract:**

1. A click starts local `submitting`, validates the form, and then waits while `busy` is true. The label changes to “Registering…” only after the user clicks Register, never because an unrelated action started.
2. If `disabled` becomes true while waiting, resolve the wait as cancelled, show `Reconnect before registering a domain.`, do not call `onSubmit`, preserve the entered hostname/port/TLS values for retry, and reset the label to “Register domain.”
3. When the wait completes while still enabled, call the latest `onSubmit` handler. The parent callback must read the latest revision at invocation time.
4. A second click while `submitting` is true is ignored. This is a UI-side single-flight guard; it does not add a second mutation queue to the backend.
5. `RuntimeStore.mutate` remains the serialization guard. The form must not call it while `busy` is true. No concurrent writes are introduced.
6. Start may pass through `connecting` while `acting` is true; that transient state must not cancel the queue. A completed Start with a connected daemon proceeds; a Start ending disconnected cancels. Stop cancellation happens when `acting` clears and the connection remains disconnected.
7. Unmount must resolve pending waiters without setting state, so a hidden window or test cleanup cannot leave a promise or a “Registering…” label stuck.

**Test first:**

- [ ] In `tests/web/DomainForm.test.tsx`, import `waitFor` and add a test proving that `busy={true}` leaves the button enabled until the user clicks it, then queues the submission and invokes the latest handler after `busy={false}`:

```tsx
it('waits for an active action and uses the latest submit handler', async () => {
  const user = userEvent.setup();
  const first = vi.fn().mockResolvedValue(undefined);
  const latest = vi.fn().mockResolvedValue(undefined);
  const view = render(<DomainForm busy onSubmit={first} />);

  await user.type(screen.getByLabelText('Domain'), 'queued');
  await user.click(screen.getByRole('button', { name: 'Register domain' }));

  expect(first).not.toHaveBeenCalled();
  expect(screen.getByRole('button', { name: 'Registering…' })).toBeDisabled();

  view.rerender(<DomainForm busy={false} onSubmit={latest} />);

  await waitFor(() => expect(latest).toHaveBeenCalledWith({
    hostname: 'queued.own',
    upstreamPort: 3000,
    tls: false,
    enabled: true,
  }));
  expect(first).not.toHaveBeenCalled();
});
```

- [ ] Add a second `DomainForm` test proving that a terminal disconnect cancels the queued submission, shows the reconnect error, and preserves the input:

```tsx
it('cancels a queued registration when the connection is lost', async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  const view = render(<DomainForm busy onSubmit={onSubmit} />);

  await user.type(screen.getByLabelText('Domain'), 'queued');
  await user.click(screen.getByRole('button', { name: 'Register domain' }));
  expect(screen.getByRole('button', { name: 'Registering…' })).toBeDisabled();

  view.rerender(<DomainForm busy disabled onSubmit={onSubmit} />);

  expect(await screen.findByRole('alert')).toHaveTextContent('Reconnect before registering a domain.');
  expect(onSubmit).not.toHaveBeenCalled();
  expect(screen.getByLabelText('Domain')).toHaveValue('queued');
  expect(screen.getByRole('button', { name: 'Register domain' })).toBeDisabled();
});
```

- [ ] Update the existing “does not relabel Register while an unrelated action is running” test in `tests/web/native-management.test.tsx` to assert that Register is **enabled** and still named “Register domain” while Start is pending. After releasing Start, assert it remains enabled. This test must fail against the current disabled-on-busy implementation.
- [ ] Add a native integration test using the existing mock client pattern: start with revision 0 and one mapping, delay the `update` response with a `releaseUpdate` promise, click “Turn TLS on,” wait until the request is pending, verify Register is enabled, type a new domain, click Register, and assert the button shows “Registering…” only after that click. Release the update and assert the queued `create` command receives `expected_revision: 1` (the revision produced by the update), not the stale revision 0, and that the new row appears:

```tsx
it('submits a queued registration with the revision that is current when it runs', async () => {
  const state: StateSnapshot = {
    settings: {
      schemaVersion: 1,
      revision: 0,
      lanAddress: '',
      upstreamDns: ['1.1.1.1'],
      mappings: [{ id: 'one', hostname: 'shop.own', upstreamPort: 3000, enabled: true, tls: false }],
    },
    runtimeStatus: 'running',
    desiredRevision: 0,
    appliedRevision: 0,
    diagnostics: [],
    latestEventSequence: 0,
  };
  let releaseUpdate!: () => void;
  let createRevision = -1;
  const client: NativeClient = {
    subscribe: async () => async () => {},
    request: vi.fn<NativeClient['request']>(async (command) => {
      if (command.type === 'snapshot') return { type: 'snapshot', payload: structuredClone(state) };
      if (command.type === 'update') {
        await new Promise<void>((resolve) => { releaseUpdate = resolve });
        Object.assign(state.settings.mappings[0], command.payload.patch);
      }
      if (command.type === 'create') {
        createRevision = command.payload.expected_revision;
        state.settings.mappings.push({ ...command.payload.input, id: 'created' });
      }

      state.settings.revision++;

      return { type: 'mutation', payload: { settings: structuredClone(state.settings), apply: null } };
    }),
  };
  const user = userEvent.setup();
  const store = new RuntimeStore(client);
  const view = render(<NativeApp store={store} onStart={async () => {}} onStop={async () => {}} onQuit={async () => {}} />);

  await screen.findByText('shop.own');
  await user.click(screen.getByRole('button', { name: 'Turn TLS on' }));
  await waitFor(() => expect(releaseUpdate).toBeTypeOf('function'));
  expect(screen.getByRole('button', { name: 'Register domain' })).toBeEnabled();

  await user.type(screen.getByLabelText('Domain'), 'queued');
  await user.click(screen.getByRole('button', { name: 'Register domain' }));
  expect(screen.getByRole('button', { name: 'Registering…' })).toBeDisabled();

  releaseUpdate();
  await waitFor(() => expect(createRevision).toBe(1));
  expect(await screen.findByText('queued.own')).toBeInTheDocument();
  view.unmount();
});
```

- [ ] In the existing Stop integration test, assert that after `Runtime: disconnected` the Register button is disabled. This proves the terminal-disconnect transition remains enforced after `busy` stops disabling the button directly.
- [ ] Run `pnpm exec vitest run tests/web/DomainForm.test.tsx tests/web/native-management.test.tsx`; expect the new tests and the changed Start assertion to fail before implementation.

**Implementation:**

- [ ] Update `DomainForm.tsx` to import `useEffect` and `useRef`, add the optional `disabled` prop, and implement refs for `busy`, `disabled`, the latest `onSubmit`, and pending waiters. Resolve waiters with `true` when `busy` becomes false and `disabled` is false; resolve them with `false` when `disabled` becomes true. On unmount, resolve pending waiters with `false` and suppress state updates via a mounted ref.
- [ ] Add a `waitForReady` helper that returns immediately when ready, returns `false` when disabled, and otherwise stores a resolver. After validation and `setSubmitting(true)`, await this helper. On cancellation, set the reconnect error and return; on success, call `onSubmitRef.current` with the normalized input and clear the hostname only after success. Always reset `submitting` in `finally` when still mounted.
- [ ] Change the submit button to `disabled={disabled || submitting}`. Keep the label derived only from local `submitting`. Do not use `busy` in the button's disabled expression.
- [ ] In `NativeApp.tsx`, compute the registration-specific disabled value as `state.connection !== 'connected' && !acting`, pass it to `DomainForm`, and change only the create callback's `expected_revision` to read the latest snapshot revision at call time. Keep `busy` for `DomainTable` and `NetworkSettingsEditor` unchanged so their existing all-actions lock remains.
- [ ] In `App.tsx`, pass `disabled={domains.busy}` to preserve the legacy dashboard's lock. Do not change the legacy `useDomains` revision or queue behavior.
- [ ] Add a manual acceptance item to `docs/desktop.md` under “Lifecycle and manual acceptance”: while Start/Stop, a TLS toggle, or a network save is pending, Register must remain enabled; a queued registration must submit after the action with the latest revision; Stop must cancel it with the reconnect error and no stuck “Registering…” label.

**Verification:**

- [ ] Run the focused tests and expect all to pass.
- [ ] Run `pnpm test`, `pnpm run lint`, `pnpm run typecheck`, and `pnpm run build`; expect all to pass, including the existing `App.test.tsx` coverage.
- [ ] Rebuild the native app using the existing macOS acceptance procedure in `docs/desktop.md` and `docs/macos-acceptance.md`, then manually verify the three actions above against the native state directory. Do not infer GUI behavior from jsdom alone.
- [ ] Commit only after explicit authorization, using a concise subject such as `feat: queue registration during native actions`.

## Completion Gate

Phase 3 delivers a native management client with synchronized state and tested lifecycle orchestration. It does not declare DNS/proxy activation, full shutdown of host services, Windows support, physical-device onboarding, or legacy retirement complete. Those require the real platform backend and acceptance evidence from Phase 4. Task 6 is a post-acceptance UX refinement and does not change those boundaries.

## References

- Tauri configuration: https://v2.tauri.app/reference/config/
- Capability scoping: https://v2.tauri.app/security/capabilities/
- Single instance: https://v2.tauri.app/plugin/single-instance/
- System tray: https://v2.tauri.app/learn/system-tray/
