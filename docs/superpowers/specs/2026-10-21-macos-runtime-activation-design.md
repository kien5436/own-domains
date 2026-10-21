# macOS Runtime Activation Design

## Summary

Phase 4 begins with a macOS runtime that keeps the Own Domains daemon and its CoreDNS, Caddy, and setup-page processes in the signed-in user's session. A one-time privileged setup installs narrowly scoped resolver, packet-filter, and trust-store integration. Normal GUI launches and CLI start/stop operations run without administrator credentials.

This design implements the macOS portion of the delivery order in `2026-10-20-native-app-architecture-design.md`. Linux systemd and Windows 11 adapters remain separate follow-up designs.

## Goals

- Make `own-domains start` and Tauri launch activate `.own` DNS, HTTP, HTTPS, and `setup.own` on macOS without requesting credentials after setup.
- Make `own-domains stop` and tray Quit stop every owned runtime process and listener while retaining settings and CA state.
- Keep the daemon as the only application-level runtime controller and settings writer.
- Apply accepted settings changes while running, retaining the last working runtime when a candidate configuration fails.
- Recover safely after daemon or child-process crashes without killing unrelated processes.
- Preserve the existing public CA identity and schema-version-1 settings when migrating from the TypeScript runtime.
- Remove `registry.own` from native runtime behavior while retaining the legacy TypeScript product until macOS acceptance passes.

## Non-goals

- Linux or Windows activation.
- Silent privileged installation or repeated authorization during normal start/stop.
- A browser-accessible management endpoint.
- Automatic modification of third-party VPN, DNS, proxy, or packet-filter configuration when a conflict is detected.
- Downloading or updating executable sidecars at runtime.

## Selected architecture

The user daemon directly supervises bundled, version-pinned CoreDNS and Caddy sidecars plus an in-process read-only setup server. The children listen on unprivileged ports in the user session:

| Component | Bindings |
| --- | --- |
| CoreDNS | `127.0.0.1:5354` and the selected LAN IPv4 address on `5354`, TCP and UDP |
| Caddy HTTP | `127.0.0.1:8080` and the selected LAN IPv4 address on `8080` |
| Caddy HTTPS | `127.0.0.1:8443` and the selected LAN IPv4 address on `8443` |
| Caddy admin | Unix socket inside the owner-only state directory |
| Setup server | `127.0.0.1:4781` |
| Daemon IPC | Existing owner-only Unix socket inside the state directory |

The privileged setup writes `/etc/resolver/own` so macOS queries `.own` through `127.0.0.1:5354`. It installs a dedicated PF anchor that redirects TCP/UDP 53 and TCP 80/443 addressed to the selected LAN interface to the corresponding user ports. The anchor contains only Own Domains rules and is referenced from `/etc/pf.conf`; setup backs up and atomically replaces every modified file. PF rules remain installed after runtime stop but are inert because no process listens on the target ports.

Changing the selected LAN address or interface is an explicit host-integration update. The GUI and CLI may save the desired value, but activation reports `setupRequired` until the privileged setup is rerun for that interface. Routine launches, mapping changes, and shutdown never request elevation.

This approach keeps privileged state static and auditable. It avoids a root command socket and gives the daemon direct ownership of every live runtime process.

## Bundled executables and paths

Release packages contain fixed CoreDNS and Caddy versions for each supported macOS architecture. The build records the expected version and SHA-256 digest; packaging fails when an executable does not match. Runtime code resolves sidecars from the application resources directory and never searches `PATH`. Development builds accept explicit absolute paths through development-only environment variables.

Adding the bundled CoreDNS and Caddy executables is a production dependency change and requires explicit approval before implementation.

User state remains under `~/Library/Application Support/OwnDomains` and is owner-only. It contains settings, generated candidates, the last applied configuration, applied revision, child ownership records, logs, the Caddy data directory, CA material, and runtime sockets. Generated configuration files are written with temporary-file, flush, and atomic-rename semantics.

## Privileged setup

The Rust CLI gains a macOS-only `setup` command with dry-run as its default. `setup --apply` requires root and records the original console user and UID explicitly; it never derives target paths from root's home directory.

Setup performs these steps transactionally:

1. Resolve and validate the selected LAN IPv4 address and interface.
2. Inspect ports 53, 80, and 443 plus existing resolver and PF configuration. Conflicts are errors, not automatic replacement.
3. Generate the persistent Caddy CA as the target user when none exists, then install only its public root certificate into the macOS trust store.
4. Write `/etc/resolver/own` with `nameserver 127.0.0.1` and `port 5354`.
5. Write a dedicated PF anchor for the selected interface/address and add one marked Own Domains anchor/load block to `/etc/pf.conf`.
6. Validate the full PF configuration before loading the dedicated anchor.
7. Persist a versioned install manifest containing paths, modes, owners, hashes, selected interface/address, binary versions, PF metadata, and backup locations.
8. Run end-to-end preflight probes. Any failure unloads the new anchor and restores all files and trust changes made by this invocation.

Setup is idempotent when the manifest and generated artifacts match. Existing files that are not recorded as owned cause a conflict. Updating an owned installation creates a new timestamped backup before replacement.

An `uninstall` operation uses the manifest, stops the runtime first, removes only exact owned PF/resolver/trust artifacts, restores pre-existing files from backups, and retains user settings unless `--purge` is supplied. Destructive purge remains explicit.

## Runtime backend

`MacOsRuntimeBackend` implements the existing synchronous `RuntimeBackend` interface through focused collaborators:

- `RuntimePaths` resolves all absolute state, resource, configuration, socket, log, and CA paths.
- `HostIntegrationInspector` validates the manifest, resolver file, PF anchor, selected LAN interface/address, trusted CA, required ports, and sidecar identity.
- `ConfigurationStager` builds candidate CoreDNS and Caddy configurations from `Settings`, validates them with fixed executable paths, and preserves the last applied files.
- `ProcessSupervisor` starts, identifies, stops, and waits for CoreDNS and Caddy children without shell interpolation.
- `SetupServer` exposes the approved read-only phone setup routes.
- `ListenerInspector` verifies the expected owners and bindings and detects survivors after shutdown.

Production code uses fixed executable paths and argument arrays with `std::process::Command`; it never invokes a shell. Tests inject fakes for command execution, process inspection, time, listeners, and filesystem boundaries.

On non-macOS targets the daemon continues using `UnavailableRuntimeBackend`. `--settings-only` always uses the unavailable backend on every platform so offline mutations cannot activate services.

## Start and reconciliation

Start is serialized by the existing daemon command lock:

1. Load settings and require a valid LAN address.
2. Inspect host integration, sidecar versions, ports, existing ownership records, and surviving owned processes.
3. Stop verified stale Own Domains children. An ambiguous PID or executable identity is reported and never killed.
4. Build candidate configurations. CoreDNS validation uses a temporary non-conflicting port set; Caddy uses `caddy validate` against native JSON.
5. Start the setup server, CoreDNS, and Caddy in that order with cleared environment variables except an explicit allowlist and Caddy data paths.
6. Wait for readiness probes: direct `.own` DNS over UDP and TCP, Caddy admin socket, setup endpoint, and HTTP routing. A bounded timeout names the failing component.
7. Atomically promote candidate files, persist child ownership records and applied revision, and report `Applied`.

If any step fails, newly started components stop in reverse order, candidate files remain available for diagnosis, and the previous applied files/revision remain authoritative.

While running, every successful settings mutation immediately requests reconciliation before the new revision is reported as applied. Caddy receives the complete JSON through its permission-restricted Unix admin socket; Caddy's load operation keeps its previous configuration on rejection. CoreDNS candidates are validated on alternate ports, then the daemon performs a bounded restart. If the new CoreDNS instance fails readiness, the daemon restores and restarts the prior configuration. The snapshot keeps `desiredRevision` at the saved settings revision and `appliedRevision` at the last fully successful revision.

Mutation persistence remains successful when reconciliation fails. The command result and subsequent event expose `Pending` or `Error`, diagnostics name the failed component, and the previous working routing stays active. A later valid mutation or explicit start retries reconciliation.

## Protocol evolution

The IPC protocol advances to version 2 because live mutation responses must distinguish durable settings success from runtime application success. Create, update, remove, and network-update commands return `MutationResult { settings, apply }`, where `apply` is absent for an offline settings-only mutation and contains the attempted desired/applied revisions, status, and message for a live runtime. Existing snapshot fields remain authoritative after reconnect.

Protocol fixtures cover every version-2 command, result, event, and error. Version-1 clients and daemons fail the handshake with the existing explicit incompatible-version error instead of attempting mixed semantics. The Rust CLI, Tauri bridge, and TypeScript native adapter update in the same release.

## Complete stop and crash recovery

Stop first rejects new mutations through the daemon's existing serialized handler. It then:

1. Marks the runtime stopping.
2. Stops accepting setup-page requests.
3. Gracefully stops Caddy and waits for its admin socket and HTTP/HTTPS listeners to disappear.
4. Terminates CoreDNS, waits, then escalates only for a child whose recorded PID and executable identity still match.
5. Verifies that ports 5354, 8080, 8443, and 4781 have no owned listeners.
6. Removes runtime sockets and ownership records, then reports stopped so the IPC server may exit.

A partial failure returns the exact surviving component and leaves runtime status `Error`; the client must not claim shutdown. Repeating stop is safe.

The daemon installs orderly handlers for supported termination signals and runs the same stop sequence. Since `SIGKILL` and power loss cannot be handled, ownership records are durable. On the next start or doctor run, exact executable path, recorded arguments, PID, and process start identity must all match before a survivor is treated as owned. Ambiguous survivors produce recovery instructions.

## Read-only setup page

The Rust setup server replaces the TypeScript server and accepts only `GET` and `HEAD` for:

- `/` — static phone DNS and certificate installation instructions.
- `/ca.crt` — the public Caddy root certificate.
- `/ca.mobileconfig` — an Apple configuration profile containing that public certificate.
- `/fingerprint` — the public certificate SHA-256 fingerprint as plain text.

Every other path returns 404 and every other method returns 405. Requests have a small header-size limit and short read/write deadlines. Responses set `X-Content-Type-Options: nosniff`, `Cache-Control: no-store`, and an explicit content type. The server never reads a path from the request, exposes directory listings, follows request-controlled redirects, or serves private keys, settings, logs, IPC, or management functions.

Caddy routes `setup.own` and the selected LAN address over HTTP only to this loopback server. No management route exists. `registry.own` is absent from CoreDNS, Caddy, setup content, and native tests.

## Diagnostics

`doctor` reports stable diagnostic codes for:

- unsupported platform;
- setup missing or manifest incompatible;
- resolver mismatch;
- PF anchor missing, invalid, or inactive;
- LAN interface/address drift;
- sidecar missing, wrong version, or wrong digest;
- CA missing or not trusted;
- port conflict with the owning process when available;
- stale owned child;
- ambiguous unowned process;
- invalid generated CoreDNS or Caddy configuration;
- failed readiness probe;
- incomplete shutdown.

Diagnostics contain no private keys, full environment, or arbitrary file content. Remedies distinguish rerunning privileged setup from retrying an unprivileged start/stop.

## Security boundaries

- Privileged setup accepts only validated address/interface inputs and emits fixed paths, templates, executable paths, and argument arrays.
- Runtime children run as the signed-in user with a minimal environment and owner-only state.
- Caddy's admin endpoint is an owner-only Unix socket and is never exposed through HTTP, DNS, PF, or LAN listeners. Caddy documents Unix admin sockets as its process-level access-control option.
- The PF anchor redirects only the selected destination address and ports 53, 80, and 443. It does not enable IP forwarding or alter unrelated traffic.
- Sidecar identity is verified before execution and again before signals are sent to a recovered PID.
- The setup server publishes only public certificate material and static guidance.

## Verification and acceptance

Automated tests run on every host for pure rendering, transaction rollback, command arrays, state transitions, reconciliation, process ownership, setup HTTP behavior, and failure reporting. macOS integration tests use temporary state plus injected command/process adapters and include fixtures for both Apple silicon and Intel resource paths.

A dedicated macOS CI or physical runner must additionally prove:

- setup dry-run changes nothing;
- failed setup restores resolver, PF, and trust state;
- CLI-only start resolves a mapping over DNS TCP/UDP and routes HTTP/HTTPS;
- GUI and CLI mutations reconcile to the same applied revision;
- a rejected candidate preserves the previous working mapping;
- `setup.own` serves only the four approved resources;
- `registry.own` neither resolves nor routes;
- stop and tray Quit remove all owned processes, sockets, and listeners;
- relaunch restores persisted mappings and the same CA identity;
- a killed daemon yields actionable survivor recovery;
- a physical phone can use the Mac as DNS, reach HTTP/HTTPS mappings, install the CA through `setup.own`, and loses all `.own` access after shutdown.

Linux development verifies portable tests and retains the runtime-unavailable behavior. macOS is not declared complete from mocked tests alone.

## Source constraints

- Apple recommends launchd for long-lived user agents and daemons, but this design keeps live routing children under direct daemon ownership so application shutdown can synchronously verify them: https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html
- Apple's resolver implementation provides the system resolver used by macOS; setup uses the existing `/etc/resolver` mechanism already exercised by the legacy product: https://github.com/apple-oss-distributions/libresolv
- CoreDNS supports explicit non-root ports and binding to selected addresses: https://coredns.io/manual/toc/ and https://coredns.io/plugins/bind/
- Caddy validates native JSON before activation, supports graceful signal shutdown, rolls back rejected API loads, and supports a permissioned Unix admin socket: https://caddyserver.com/docs/command-line, https://caddyserver.com/docs/api, and https://caddyserver.com/docs/caddyfile/options
