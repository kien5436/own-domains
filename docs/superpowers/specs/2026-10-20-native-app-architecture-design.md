# Native Application Architecture Design

## Summary

Own Domains will become a native desktop product built with Tauri 2, the existing React/TypeScript/Vite/Tailwind frontend, and a new Rust core. A Rust daemon will be the single runtime authority for the native GUI and an independent Rust CLI. The browser-based management dashboard and `registry.own` will be removed. The read-only `setup.own` phone onboarding page will remain available while the runtime is active.

The application will target macOS first, followed by supported systemd Linux distributions and Windows 11. Portable core behavior and IPC contracts will remain platform-neutral from the beginning.

## Goals

- Deliver a native system-WebView desktop application without Electron or a bundled browser engine.
- Preserve the existing React, TypeScript, Vite, and Tailwind frontend where practical.
- Provide equivalent management capabilities through the GUI and CLI.
- Ensure changes made through either client immediately appear in the other.
- Keep one authoritative settings writer and runtime controller.
- Hide the GUI window to the tray when closed.
- Completely stop the daemon, DNS resolution, and proxy routing when the user selects **Quit** or runs the CLI stop command.
- Retain saved mappings across shutdown and restore them on the next start.
- Keep `setup.own` available for phone instructions and public CA distribution while routing is running.

## Non-goals

- Retaining a browser-accessible management interface.
- Retaining `registry.own` as a DNS name, route, or compatibility alias.
- Allowing the GUI and CLI to operate separate runtime implementations.
- Starting DNS or proxy services implicitly when an offline mutation is made.
- Running management IPC over a LAN-accessible TCP listener.

## Architecture

```text
Tauri GUI -- local IPC --+
                         +--> Own Domains daemon --> CoreDNS
Rust CLI --- local IPC --+                       +--> Caddy
```

The Rust workspace will contain these primary packages:

- `own-domains-core`: domain validation, settings, revisions, configuration generation, reconciliation, diagnostics, and platform-neutral service interfaces.
- `own-domains-daemon`: the single authoritative process that owns runtime state, serializes mutations, controls services, and publishes events.
- `own-domains-cli`: the independent `own-domains` command-line program.
- `own-domains-desktop`: the Tauri shell containing native window, tray, lifecycle, and IPC client integration.

The React frontend will run only inside the Tauri WebView. It will not host or call a browser-accessible management API. The Tauri process and CLI will both communicate with the daemon through the same versioned IPC protocol.

## Runtime ownership

Only the daemon may write settings, generate active configurations, or change CoreDNS and Caddy state. The GUI and CLI are clients. This prevents competing writers and guarantees that identical validation and reconciliation behavior applies to both interfaces.

The daemon will expose:

- Request/response operations for reads, mutations, diagnostics, start, and stop.
- A subscription stream for settings revisions, desired/applied state, diagnostics, and runtime status.
- A health/version handshake so incompatible clients fail with an actionable error.

Local IPC will use a permission-restricted Unix domain socket on macOS and Linux and a user-scoped named pipe with an explicit ACL on Windows. It will not use a TCP management listener.

## Lifecycle

### GUI launch

1. Enforce a single GUI instance.
2. Connect to the daemon or start it if absent.
3. Ask the daemon to start and reconcile CoreDNS and Caddy.
4. Load current settings, applied state, and diagnostics.
5. Show the native window and tray item.

Launching the GUI therefore activates Own Domains. Persisted mappings are restored, but activation succeeds only after generated configurations pass validation.

### Window close

Closing the main window cancels destruction and hides the window. DNS, Caddy, the daemon, and the tray item continue running. Selecting the tray item restores and focuses the existing window.

### Complete shutdown

Tray **Quit** and `own-domains stop` perform the same operation:

1. Reject new mutations and mark the runtime as stopping.
2. Stop CoreDNS and Caddy.
3. Verify that owned listeners and processes have stopped.
4. Shut down the daemon.
5. Remove the tray item and exit the GUI when Quit originated there.

Saved settings and mappings are retained. All `.own` names, including `setup.own`, become unresolvable after shutdown. If cleanup fails, the initiating client reports the remaining active component and provides retry/recovery guidance instead of claiming that the runtime stopped.

An unexpected daemon termination will attempt orderly service cleanup. Platform inspection on the next launch will detect and report any surviving owned services.

## CLI behavior

The initial command surface will be:

```text
own-domains start
own-domains stop
own-domains status
own-domains list
own-domains add shop.own --port 3000 --https
own-domains edit shop.own --port 4000
own-domains enable shop.own
own-domains disable shop.own
own-domains remove shop.own
own-domains doctor
```

`start` launches the daemon and routing without opening the GUI. `stop` performs a complete shutdown. Read and mutation commands use the daemon when it is running.

When a mutation command runs while the runtime is stopped, the CLI starts a settings-only daemon mode, commits the mutation, reports `saved; runtime stopped`, and terminates that temporary daemon. It does not silently start CoreDNS or Caddy. This preserves the daemon as the exclusive writer without treating configuration edits as runtime activation.

## State consistency

Settings retain a monotonically increasing revision. Every mutation includes `expectedRevision`. A stale request fails without mutation and returns the current revision so the client can reload.

The daemon serializes all mutations. Durable writes use a temporary file, flush, and atomic rename before reconciliation. Desired and applied revisions remain distinct. A rejected CoreDNS or Caddy configuration leaves the previous working runtime configuration active while exposing the unapplied desired state.

After every accepted mutation or runtime transition, the daemon publishes a revisioned event. An open GUI reloads affected state immediately when a CLI command changes it. Clients that miss events reconnect, compare revisions, and fetch a complete snapshot.

## DNS, proxy, and setup page

CoreDNS will own `.own` DNS resolution while the runtime is active. Caddy will route only explicitly enabled mappings plus the reserved read-only setup host.

`registry.own` will be removed from DNS configuration, Caddy configuration, documentation, tests, and application behavior. It will not redirect to the native application.

`setup.own` will remain a narrowly scoped HTTP endpoint for devices on the LAN. It may expose only:

- Phone DNS instructions.
- The public CA certificate and supported installation profile.
- The public certificate fingerprint.
- Non-sensitive connectivity guidance.

It must not expose management operations, settings, logs, local paths, private keys, IPC access, or arbitrary proxy destinations.

## Privilege and platform integration

Installation and host integration remain explicit privileged operations. Normal GUI launches and CLI commands must not repeatedly request administrator credentials.

Each platform adapter will define application data paths, IPC endpoints, service installation, resolver configuration, service activation/deactivation, listener inspection, ownership, and rollback. Fixed executable paths and argument arrays will be used for privileged operations; settings or UI input will never be interpolated into shell commands.

The final service topology may use platform service managers for the privileged DNS/proxy processes, but the daemon remains their sole application-level controller. Tray Quit and CLI stop must disable those processes and verify shutdown.

## Error handling and recovery

- An incompatible IPC protocol produces a clear client/daemon version error.
- An unavailable daemon is distinguished from a stopped runtime.
- Invalid input is rejected before persistence or configuration generation.
- Failed persistence leaves both desired and applied state unchanged.
- Failed activation retains the previous working configuration.
- Partial shutdown reports exactly which service or listener remains active.
- Corrupt settings produce a recovery error and preserve the corrupt file; they never silently create an empty registry.
- Setup-page failures cannot grant access to management capabilities.

## Migration strategy

1. Introduce a Rust workspace and contract tests without replacing the running TypeScript application.
2. Port domain validation, versioned settings, revisions, and crash-safe persistence while retaining compatibility with the existing JSON schema.
3. Port Caddy/CoreDNS generation, reconciliation, diagnostics, and platform abstractions.
4. Implement the daemon, IPC protocol, event subscriptions, and Rust CLI.
5. Add the Tauri shell and connect the existing React UI to IPC-backed state.
6. Implement tray behavior, hide-on-close, single-instance handling, and complete shutdown.
7. Retain the isolated `setup.own` handler and remove `registry.own` plus the browser management server.
8. Remove TypeScript backend and CLI entry points only after behavioral parity tests pass.
9. Validate macOS before implementing Linux and Windows host adapters.

No existing settings or CA identity will be discarded during migration. A migration failure will leave the original data available for recovery.

## Testing and acceptance criteria

Automated verification will cover:

- Shared core behavior used by the daemon, CLI, and GUI.
- Existing hostname, port, duplicate, and reserved-name validation.
- Settings-schema compatibility and crash-safe migration.
- Concurrent GUI/CLI mutations and stale revisions.
- IPC authentication, protocol versioning, disconnects, and event resynchronization.
- GUI launch, close-to-hide, tray restoration, single instance, and tray Quit.
- CLI-only start, offline mutation, live mutation, status, and complete stop.
- CoreDNS and Caddy validation, rejected activation, restart, and cleanup failure.
- Proof that all `.own` names stop resolving after Quit.
- Proof that `registry.own` never resolves or routes.
- Proof that `setup.own` exposes only approved read-only resources.
- Platform adapter behavior on macOS, supported systemd Linux, and Windows 11.

Before a platform is declared complete, physical-device verification must confirm HTTP and trusted HTTPS routing, CA installation through `setup.own`, runtime restart, complete shutdown, and the absence of LAN-accessible management interfaces.

## Delivery order

1. Portable Rust core and daemon contracts.
2. macOS daemon, CLI, Tauri GUI, CoreDNS/Caddy lifecycle, and physical-phone verification.
3. Supported systemd Linux integration and verification.
4. Windows 11 integration and verification.

The product is not cross-platform complete until all three host families pass their automated and physical-device acceptance criteria.
