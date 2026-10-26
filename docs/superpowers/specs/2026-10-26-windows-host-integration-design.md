# Windows Host Integration Design

> **Status:** Design only. No item in this document has been executed on a real Windows host. Every statement marked **[verify]** is a blocking feasibility-gate item and must be confirmed on Windows 10 22H2 x64 and Windows 11 x64 before the design is implemented beyond a skeleton. Mocked tests are not acceptance evidence.

## Summary

Own Domains targets Windows 10 22H2 x64 and Windows 11 x64, with arm64 build-only until a separate acceptance run exists. Windows needs the same product boundary as macOS: one UAC-elevated setup installs narrowly scoped, static host integration; normal GUI launches, CLI start/stop/status, mapping edits, LAN address changes, and switching between Ethernet and Wi-Fi never request administrator credentials.

The Windows adapter uses:

- a user-scoped named pipe for daemon IPC, preserving the existing JSON-line framing and protocol version 2;
- direct binding of CoreDNS to port 53 and Caddy to ports 80 and 443, on loopback plus the selected LAN IPv4 address only;
- a machine-wide NRPT rule that sends `.own` queries to `127.0.0.1`;
- four program-scoped Windows Firewall rules for TCP/UDP 53 and TCP 80/443, restricted to the Domain and Private profiles and the local subnet;
- `certutil -addstore -user Root` trust in `CurrentUser\Root` for the Caddy local CA;
- admin-owned CoreDNS and Caddy executables under `C:\ProgramData\OwnDomains\bin`;
- an admin-owned install manifest at `C:\ProgramData\OwnDomains\install-manifest.json`.

It does **not** use `netsh interface portproxy`, a persistent privileged helper, a Windows service, a privileged command socket, or a wildcard listener. The existing `SetupRequest`, `UninstallRequest`, IPC protocol, settings schema 1, mapping semantics, GUI, and platform-neutral runtime/installer/inspector contracts stay unchanged.

The design has two implementation phases:

1. **Named-pipe IPC.** Replace the hardcoded Unix socket in the daemon, client, CLI, and desktop with a platform endpoint abstraction. This is a transport change only. The protocol remains version 2 and every existing protocol fixture must pass over the pipe unchanged.
2. **Host integration.** Add a Windows runtime backend, inspector, installer, setup/uninstall transaction, and acceptance evidence. Direct binding is selected over portproxy because Windows does not reserve low ports for administrators and because portproxy cannot redirect UDP DNS. If direct binding or NRPT fail-closed behavior does not pass the real-host gate, this design is blocked; there is no silent fallback to portproxy or a privileged helper.

## Context

The native runtime has one implemented host adapter: macOS. On Windows, the daemon returns `runtimeUnavailable`, the CLI uses `UnsupportedHostInstaller`, and all IPC code is Unix-only:

- `crates/own-domains-daemon/src/ipc.rs` binds a `UnixListener`, removes a stale socket file, and uses Unix metadata for identity.
- `crates/own-domains-client/src/unix.rs` connects with `UnixStream` and exposes a Unix-socket event stream.
- `crates/own-domains-client/src/lifecycle.rs` treats a missing socket path as an absent daemon and waits for the socket file to disappear after stop.
- `crates/own-domains-desktop/src/bridge.rs` stores a `socket: PathBuf` in `DesktopPaths`.
- `crates/own-domains-desktop/src/shell.rs` launches the daemon with `--socket` and waits on the same path.

The shared platform contracts already exist in `crates/own-domains-daemon/src/platform/host.rs`: `HostInspector`, `HostInstaller`, `SetupRequest`, `UninstallRequest`, `SetupPreview`, `SetupApplyResult`, and `SetupFailure`. `crates/own-domains-daemon/src/platform/mod.rs` selects a runtime backend at compile time. The macOS implementation under `crates/own-domains-daemon/src/platform/macos` remains the reference for start, reconcile, stop, manifest, and rollback semantics, but its PF, resolver, trust, and Unix-socket details are not portable.

Windows cannot reuse the macOS mechanisms. PF, `/etc/resolver`, the macOS keychain, `ifconfig`, and Unix sockets are replaced by NRPT, Windows Firewall, `certutil`, `GetAdaptersAddresses`, named pipes, and Job Objects. Windows also has no single privileged configuration file analogous to a PF anchor; its privileged state is a set of machine-wide rules plus an admin-owned manifest.

There is no native Windows runtime to migrate. The legacy TypeScript product is macOS-only in this repository, and Windows currently has no IPC or host integration. A normal Windows installation starts from a fresh `%LOCALAPPDATA%\OwnDomains` state directory; existing macOS settings and CA state are not imported. Pre-existing Windows development state is handled under [Migration](#migration).

## Platform boundaries

The shared application must not learn named pipes, NRPT, Windows Firewall, `certutil`, ProgramData ACLs, or Windows interface discovery. Portable contracts stay platform-neutral:

- `RuntimeBackend` remains the daemon's apply/stop/inspect/diagnose contract.
- `HostInspector` remains the shared diagnostic contract.
- `HostInstaller` remains the shared setup/uninstall contract.
- `SetupRequest` remains exactly `state_directory`, `lan_address`, and `interface`.
- `UninstallRequest` remains exactly `state_directory` and `purge`.
- `SetupPreview`, `SetupApplyResult`, `SetupFailure`, and `SetupAction` remain shared.
- `PROTOCOL_VERSION` remains `2`; no command, result, event, or error shape changes.
- Settings remain schema version 1 with `lanAddress`, `upstreamDns`, and mappings.

The following move out of the macOS namespace when they have no macOS-specific fields:

- `SidecarManifest` and `SidecarSpec`, because macOS and Windows both verify pinned CoreDNS/Caddy executables.
- The read-only setup HTTP server, because its routes, methods, headers, and limits are platform-neutral.
- The IPC endpoint abstraction and JSON-line codec, because only the transport differs.

The following remain inside `platform/windows` and are not abstracted into shared code:

- Named-pipe creation, security descriptors, connection verification, and stale/busy handling.
- NRPT rule names, PowerShell invocations, and policy inspection.
- Windows Firewall COM rule definitions and fingerprints.
- `certutil` trust commands and CurrentUser store verification.
- `GetAdaptersAddresses`, `GetIfEntry2`, and Windows interface eligibility.
- ProgramData/state-directory ACLs, the Windows manifest schema, sidecar paths, and target triple.
- Job Object supervision, `CreateProcessW` flags, and Windows process identity.

Platform selection stays compile-time through `platform::runtime_backend` and `cli::platform::host_installer`. Shared command code must not contain `cfg!(target_os = "windows")` branches; the facade returns a Windows, macOS, Linux, or unsupported implementation. The two existing macOS guards in `execute_setup` and `execute_uninstall` are removed as part of the preparatory refactor, matching the Linux adapter work.

### Preparatory refactor

The platform boundary is a separate, behavior-preserving refactor before Windows host work:

1. Introduce an endpoint abstraction in the client and daemon. `Client`, `connect_or_launch`, `stop_and_wait`, `DesktopPaths`, and `DesktopBridge` accept an endpoint instead of a `Path`. Unix behavior and tests remain unchanged.
2. Extract the JSON-line frame reader/writer and handshake/event logic so it is independent of the stream type.
3. Extract `SidecarManifest`/`SidecarSpec` and the read-only setup server into shared modules.
4. Add a Windows platform facade, a Windows CLI installer selection, and a Windows elevation module. Keep `UnsupportedHostInstaller` for other targets.
5. Add compile-time sidecar pins for Windows and ZIP extraction to the sidecar fetch/verification script. The pins are embedded in the signed CLI/daemon, not read from user-writable resources at privilege time.
6. Add Windows-only dependencies for Win32, security descriptors, COM, and process APIs. This is a production dependency change and requires explicit approval before implementation.

The refactor changes no host artifacts, paths, manifest fields, diagnostics, or runtime behavior on macOS. Phase 1 starts after step 1 and step 2; Phase 2 starts only after all steps and the real-host gates pass.

Coordination with the Linux design (`2026-10-26-linux-host-integration-design.md`): steps 1-3 (endpoint abstraction, JSON-line codec, shared `SidecarManifest`/`SidecarSpec` and setup-server moves) are done once and shared with Linux. The Linux `platform/unix` process/config/files/runtime/paths/setup extraction builds on top of this series and must not duplicate it. Order: transport extraction first, then the Unix extraction, then the Linux and Windows adapters.

## Goals

- Make `own-domains start` and Tauri launch activate `.own` DNS, HTTP, HTTPS, and `setup.own` on Windows without UAC after setup.
- Make `own-domains stop` and tray Quit stop every owned process and listener without UAC.
- Keep CoreDNS and Caddy bound only to `127.0.0.1` plus the selected LAN IPv4 address. Do not introduce wildcard listeners or HTTP/3 UDP listeners.
- Keep the LAN address and interface out of all persistent privileged artifacts.
- Make address changes, DHCP renewals, Ethernet/Wi-Fi switches, and new physical interfaces unprivileged runtime work.
- Preserve mappings, settings schema 1, CA identity, protocol version 2, GUI behavior, and uninstall reversibility.
- Keep the no-helper boundary: no long-running privileged process, Windows service, privileged socket, or privileged command channel.
- Make diagnostics actionable for missing setup, NRPT drift, firewall profile/rule drift, port conflicts, inactive addresses, sidecar drift, and untrusted CA.
- Require exactly one UAC prompt for a normal first setup and one for a normal uninstall.

## Non-goals

- Zero-UAC installation. Initial setup and uninstall still require administrator approval.
- A privileged helper daemon, Windows service, scheduled task, privileged named pipe, or auto-elevating GUI action.
- `netsh interface portproxy`, WinNAT, ICS, WinDivert, a kernel driver, or any privileged port-forwarding layer.
- IPv6 LAN support. The settings model and sidecar bindings remain IPv4.
- Windows Server, Windows 10 versions before 22H2, or Windows 11 builds before the accepted baseline.
- arm64 runtime acceptance. arm64 is build-only in this design.
- MSI packaging. The Tauri NSIS `currentUser` package is the distribution target.
- Auto-start at login or boot. NRPT and firewall rules persist; the runtime starts when the user launches the app or CLI.
- A GUI button that runs privileged setup. Setup remains an explicit CLI action with a UAC prompt.
- Automatic modification of VPN, third-party firewall, DNS-over-HTTPS, network profile, Group Policy, or browser trust stores.
- Concurrent multi-user operation. NRPT and ports 53/80/443 are machine-wide; one user owns the installation.
- WSL, Hyper-V guests, containers, or Windows Sandbox as supported hosts.
- A custom NRPT DNS port. NRPT has no documented port parameter, so CoreDNS owns port 53.
- Changing the IPC protocol, settings schema, mapping semantics, or reserved `setup.own` behavior.

## Selected architecture

The host integration is static. The only mutable network state is the selected LAN address in the user's settings, and it is consumed only by the unprivileged runtime.

| Artifact | Location | Owner / ACL | Content | Changes after setup |
| --- | --- | --- | --- | --- |
| Daemon IPC endpoint | `\\.\pipe\own-domains-<hash>` | Current user only, protected DACL, remote clients rejected | JSON-line protocol v2 | Never |
| State directory | `%LOCALAPPDATA%\OwnDomains` | Current user full; SYSTEM and Administrators full; no other users | Settings, generated configs, logs, Caddy data, CA, runtime files | Freely changed by the daemon |
| App installation | `%LOCALAPPDATA%\Own Domains` | Current user | GUI, CLI, daemon, resource sidecars | App updates |
| Sidecar binaries | `C:\ProgramData\OwnDomains\bin\coredns.exe`, `caddy.exe` | Administrators/SYSTEM full; Users read+execute | Pinned CoreDNS and Caddy executables | Only when the pinned sidecar set changes |
| Install manifest | `C:\ProgramData\OwnDomains\install-manifest.json` | Administrators/SYSTEM full; owning user read | Schema 1: owner SID, sidecar pins, NRPT rule, firewall rules, CA fingerprints, backups | Only when setup is rerun |
| Setup backups | `C:\ProgramData\OwnDomains\backups\<timestamp>` | Administrators/SYSTEM full | Previous manifest, firewall/NRPT snapshots, replaced sidecars, transaction journal | Each setup that changes privileged state |
| NRPT rule | `Own Domains (.own)` | Machine-wide; admin-created | Namespace `.own`, name server `127.0.0.1` | Never, unless the host integration version changes the rule shape |
| Firewall rules | `OwnDomains-CoreDNS-UDP53`, `OwnDomains-CoreDNS-TCP53`, `OwnDomains-Caddy-TCP80`, `OwnDomains-Caddy-TCP443` | Machine-wide; admin-created | Inbound allow, Domain+Private, LocalSubnet, program-scoped | Only when the sidecar path or port shape changes |
| CA trust | `CurrentUser\Root` | Owning user | Public Caddy root certificate | When the CA identity changes |
| Uninstall journal | `%LOCALAPPDATA%\OwnDomains\uninstall.json` | Current user | CA thumbprint and pending uninstall state | During uninstall only |

The manifest contains no LAN address, interface name, interface GUID, DHCP state, or LAN subnet. It records only immutable installation facts plus the fixed loopback NRPT server. The selected address remains in `settings.json` and never becomes a privileged artifact.

### Runtime bindings

| Component | Bindings |
| --- | --- |
| CoreDNS | `127.0.0.1:53` and the selected LAN IPv4 address on `53`, TCP and UDP |
| Caddy HTTP | `127.0.0.1:80` and the selected LAN IPv4 address on `80` |
| Caddy HTTPS | `127.0.0.1:443` and the selected LAN IPv4 address on `443` |
| Caddy admin | `%LOCALAPPDATA%\OwnDomains\runtime\caddy-admin.sock` as an AF_UNIX socket, or disabled |
| Setup server | `127.0.0.1:4781` |
| Daemon IPC | User-scoped named pipe |

There is no `0.0.0.0`, `::`, or other wildcard binding. HTTP/3 is disabled so Caddy does not create an unplanned UDP 443 listener. Caddy admin is never a loopback TCP listener; any local process could otherwise reconfigure Caddy and proxy arbitrary local services.

The Windows backend constructs `DnsConfigOptions` with `port: 53`, `upstream_port: 53`, and `listen_addresses = [127.0.0.1, selected_lan]`, and supplies the Windows TTL (`0`) to the renderer. It constructs `ProxyConfigOptions` with `http_port: 80`, `https_port: 443`, `setup_port: 4781`, and the same listen addresses. It does not reuse the macOS 5354/8080/8443 defaults. The shared renderers keep their existing validation and framing.

### Why direct binding, not portproxy

Windows does not reserve ports below 1024 for administrators. A normal user process can bind 53, 80, and 443 when those ports are free and not in an excluded range. **[verify]** This behavior is the foundation of the design and must be tested as a standard user, after reboot, with Hyper-V/WSL/Docker installed and uninstalled.

Direct binding removes the need for a redirect layer entirely:

- CoreDNS receives phone and host DNS on 53.
- Caddy receives HTTP and HTTPS on 80 and 443.
- The selected address is used directly, so no packet translation, address-specific rule, or loopback redirect exists.
- DHCP and interface changes only require rebinding the two user processes. No elevated rule changes.

`netsh interface portproxy` was evaluated and rejected as the default:

| Property | Portproxy | Direct binding |
| --- | --- | --- |
| UDP DNS on 53 | Not supported; portproxy is TCP-only | Supported |
| Exact address scope | Requires an exact `listenaddress`; a wildcard is forbidden by this design | Binds exactly loopback plus the selected address |
| Address/DHCP changes | Require elevated rule updates for each new address | Rebind in the user daemon; no elevation |
| Listener conflict | Portproxy must own 53/80/443, preventing CoreDNS/Caddy from binding them | One listener per service |
| Dependencies | IP Helper service and `netsh` rule state | No extra service |
| Excluded port ranges | Cannot bind an excluded port either | Same limitation; setup reports it |

Portproxy also cannot redirect UDP 53, so it cannot satisfy `.own` resolution for Windows clients or phones. Using it only for TCP would leave DNS broken or force a second mechanism, adding a privileged component without removing the low-port requirement. If direct binding fails because a port is excluded, portproxy would fail to bind the same port; this design is blocked rather than silently downgraded. Do not add a privileged helper, service, WinNAT rule, or kernel driver without a new design decision.

### Why NRPT and port 53

The Windows DNS Client supports Name Resolution Policy Table (NRPT) rules. A rule for namespace `.own` with name server `127.0.0.1` sends `.own` and all subdomains to CoreDNS while leaving every other name on the existing resolver path.

NRPT has no documented custom-port parameter for `-NameServers`. It accepts IP addresses, not `IP:port`. Therefore CoreDNS must own port 53. A future Windows release that supports a per-rule port could remove this constraint; this design does not assume one.

The NRPT rule is machine-wide and static:

```powershell
$rule = Add-DnsClientNrptRule `
  -DisplayName 'Own Domains (.own)' `
  -Namespace '.own' `
  -NameServers '127.0.0.1' `
  -Comment 'Own Domains managed rule' `
  -PassThru
$rule.Name
```

`Add-DnsClientNrptRule` returns a GUID in `Name` when `-PassThru` is used. **[verify]** The manifest records that GUID, not the display name alone. `-DisplayName` is the friendly label; `-Name` is the unique rule identifier used by `Get-DnsClientNrptRule`, `Set-DnsClientNrptRule`, and `Remove-DnsClientNrptRule`.

The rule survives reboot and remains installed while the runtime is stopped. CoreDNS owns 53 only while the runtime runs. When it stops, `.own` queries go to a closed loopback port. Windows DNS Client must fail the query rather than fall back to another DNS server. **[verify]** If Windows falls back to another server, `.own` could leak to public DNS and this design is blocked. There is no unprivileged, no-service way to keep a blackhole responder on 53 at all times.

NRPT behavior has additional constraints:

- A Group Policy NRPT rule for `.own` or a more specific suffix can override the local rule. Setup inspects `Get-DnsClientNrptPolicy` and fails on a conflict. **[verify]**
- The DNS Client service (`Dnscache`) must be running. Setup does not start or reconfigure it.
- Applications that bypass the system resolver (browsers with secure DNS, `nslookup` with an explicit server, some Java resolvers) do not honor NRPT. The setup page must tell users to use the full `.own` name and, where applicable, disable secure DNS for `.own`.
- NRPT is machine-wide. A second local user can resolve and reach `.own` services while the owner's runtime runs, but cannot control the daemon. Shared-device operation is documented as a limitation, not a supported multi-user feature.

## Phase 1: Named-pipe IPC

Phase 1 replaces the hardcoded Unix socket with a platform endpoint abstraction. It changes no command, result, event, error shape, settings field, or host artifact. The protocol stays `PROTOCOL_VERSION = 2`, the frame limit stays 1 MiB, and every existing protocol fixture must pass over the pipe without modification.

### Endpoint derivation

The Windows endpoint is a user-scoped named pipe:

```text
\\.\pipe\own-domains-<128-bit-hash>
```

The hash is the first 16 bytes of SHA-256, rendered as 32 lowercase hex characters, over this byte string:

```text
"own-domains-pipe-v1\0" ||
UTF-8(ConvertSidToStringSidW(current_user)) || "\0" ||
UTF-16LE(lowercase(canonical_state_directory))
```

The canonical state directory is produced with `GetFullPathNameW` followed by `PathCchCanonicalizeEx`, with a trailing separator removed. It is not resolved through reparse points for derivation. The lowercase step uses `LCMapStringEx(LOCALE_NAME_INVARIANT, LCMAP_LOWERCASE, ...)` on the UTF-16 path. This is a derivation convention so the desktop, CLI, and daemon agree on one name; it is not a security boundary. The DACL and the client's server-identity check are the security boundaries.

The SID prevents two users with a directory at the same path from sharing an endpoint. The state-directory hash prevents two installations or test directories for the same user from sharing an endpoint. The pipe name is about 50 characters, well below the 256-character named-pipe limit. The endpoint is not secret and is safe to print in diagnostics.

If the state directory is later found to be a reparse point, a junction, or a directory not owned by the current user, the daemon refuses to start with `stateDirectoryInsecure`. It does not derive a second endpoint or fall back to a path-based socket.

### Server creation and ACL

The daemon creates the first pipe instance with:

- `CreateNamedPipeW`;
- `PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED`;
- `PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT`;
- `PIPE_REJECT_REMOTE_CLIENTS`;
- `FILE_FLAG_FIRST_PIPE_INSTANCE` on the first instance only;
- up to 64 instances;
- 64 KiB input and output buffers;
- a default timeout of zero.

Byte mode is required because the existing protocol is newline-delimited JSON, not a message protocol. The server must not use `PIPE_TYPE_MESSAGE` or `PIPE_READMODE_MESSAGE`; doing so would change frame boundaries and could break the 1 MiB limit and subscription stream.

The security descriptor is explicit and protected:

```text
O:<current-user-SID>G:<current-user-SID>D:P(A;;GA;;;<current-user-SID>)
```

The SDDL is converted with `ConvertStringSecurityDescriptorToSecurityDescriptorW` and passed through `SECURITY_ATTRIBUTES` with `bInheritHandle = FALSE`. `D:P` disables inherited ACEs, so the default named-pipe DACL that grants read access to `Everyone` is not present. Only the current user SID receives access. Administrators and SYSTEM can take ownership through normal Windows administrative authority; that is outside the application's threat model.

`FILE_FLAG_FIRST_PIPE_INSTANCE` makes creation fail if any process already owns the name. The daemon must treat that failure as `ipcEndpointBusy` and exit. It must not connect to the existing endpoint, impersonate it, kill its owner, or unlink a filesystem object. There is no filesystem object to unlink.

After the first instance is created, the accept loop creates additional instances with the same name, ACL, and flags except `FILE_FLAG_FIRST_PIPE_INSTANCE`. Every instance, including the first, accepts at most one client and then returns to the accept loop after `DisconnectNamedPipe`; `FILE_FLAG_FIRST_PIPE_INSTANCE` proves ownership of the name at creation time, but it does not make the first instance the sole listener. The server never creates a second first-instance or a second daemon for the same endpoint.

The accept loop uses overlapped `ConnectNamedPipe` and waits on the pipe event plus a manual-reset shutdown event. On shutdown it calls `CancelIoEx` for the pending connect, closes the listening instances, and lets connected clients finish or disconnect. After each client, it calls `DisconnectNamedPipe` before reusing the instance. It does not call `FlushFileBuffers`: on a connected byte-mode pipe that call can block until the client reads, and each JSON line is already written through the overlapped write path. `ERROR_PIPE_CONNECTED` means a client connected between creation and `ConnectNamedPipe`; that is not an error.

On connection, the server calls `GetNamedPipeClientProcessId` and `ProcessIdToSessionId`. A client in another session is disconnected. The client PID is recorded in the daemon log for audit. The server does not impersonate the client and does not need to: the DACL already restricts connections to the current user, and same-user processes can read the user's state directory and CA key anyway.

### Client connection and server verification

The client opens the pipe with `CreateFileW(endpoint, GENERIC_READ | GENERIC_WRITE, 0, NULL, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, NULL)`. On `ERROR_PIPE_BUSY`, it calls `WaitNamedPipeW(endpoint, remaining_timeout)` and retries. On `ERROR_FILE_NOT_FOUND`, the endpoint is absent. On `ERROR_ACCESS_DENIED`, the endpoint exists but the caller lacks access or a foreign server created a restrictive DACL; that is an error, not absence.

Named pipes do not support the Unix `set_read_timeout`/`set_write_timeout` contract directly. The Windows connection uses overlapped `ReadFile` and `WriteFile` with a per-operation wait equal to the remaining client timeout, then `CancelIoEx` on expiry. It does not rely on `SetCommTimeouts`, which is not a documented named-pipe timeout mechanism.

Before sending the handshake, the client verifies the server:

1. `GetNamedPipeServerProcessId` returns the server PID.
2. `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` opens that PID.
3. `QueryFullProcessImageNameW` returns the canonical image path. It must equal the expected `own-domains-daemon.exe` path resolved next to the CLI or supplied by the desktop.
4. `GetProcessTimes` returns a creation time. The client keeps the process handle open for the life of the connection so the PID cannot be recycled into a different process while the pipe is in use.
5. The token user SID from `OpenProcessToken`/`GetTokenInformation` must equal the current user SID.
6. `ProcessIdToSessionId` must equal the current session ID.
7. In production builds, the daemon image's Authenticode signer must match the pinned publisher. Unsigned development daemons are accepted only with the explicit development override described under [Security boundaries](#security-boundaries).

A mismatch closes the handle without sending any request and returns `ClientError::UntrustedDaemon`. It is not retried as if the daemon were absent, because a squatted or substituted endpoint must be reported, not hidden.

The client also verifies the handshake result and protocol version exactly as the Unix client does. The server identity check is additional; it does not replace the protocol handshake.

### Framing, lifecycle, and shutdown

The JSON-line codec moves out of `crates/own-domains-daemon/src/ipc.rs` and `crates/own-domains-client/src/unix.rs` into transport-neutral modules. Both transports use the same `read_frame`, `write_json_line`, `exchange`, and event-stream parsing logic. The Windows connection is a byte stream; it does not add a length prefix, message header, or pipe-specific envelope.

`stop_and_wait` becomes endpoint-aware:

- Unix: wait for the socket path to disappear, as today.
- Windows: wait until `CreateFileW` returns `ERROR_FILE_NOT_FOUND` for the endpoint, or until the bounded timeout expires. A named-pipe object disappears when the last server handle closes, so there is no stale file to remove and no filesystem race.
- A pipe that remains present after the stop acknowledgment produces `ShutdownTimeout`. `ERROR_PIPE_BUSY` is not absence; it means a live server is still accepting or busy.

`IpcServer::request_shutdown` sends the existing `Command::Stop` request over the endpoint. On Windows, the daemon installs a `SetConsoleCtrlHandler` handler that calls the same function. A daemon launched with `CREATE_NO_WINDOW` has no console, so the handler is inert there; the Job Object is the crash backstop. The Unix signal path remains unchanged.

### Transport abstraction

The preparatory refactor introduces an endpoint type rather than sprinkling `cfg(target_os)` through command code:

```text
Endpoint::Unix(PathBuf)
Endpoint::NamedPipe(String)

Endpoint::display_name()
Endpoint::is_absent(io::Error) -> bool
Endpoint::connect(timeout) -> Connection
Endpoint::server_bind(state) -> IpcServer
Endpoint::launch_argument() -> Option<(&str, OsString)>
```

`Client` stores an `Endpoint` instead of a `PathBuf`. `ClientError::DaemonUnavailable` carries the endpoint display name, not a `PathBuf`. `EventStream` is generic over a `Connection` trait or uses a small enum so the Unix and Windows implementations share the event parser. `EventStreamCancellation` is transport-specific: Unix duplicates and shuts down the stream; Windows holds a shared owned pipe handle and calls `CancelIoEx(handle, NULL)`, mapping `ERROR_OPERATION_ABORTED` to cancellation.

`DesktopPaths` becomes:

```text
state_dir: PathBuf
endpoint: Endpoint
```

`--state-dir` remains required for the desktop. `--socket` remains a Unix-only development override. `--pipe` is a Windows-only development override. The default is derived from the state directory; the desktop does not pass a socket path to the daemon. The CLI keeps `--socket` on Unix and gains `--pipe` on Windows for tests and development. Shared command code calls `Endpoint` methods and never matches on the operating system.

`connect_or_launch` takes the endpoint, the state directory, and the daemon executable. It tries the endpoint first. If absent, it launches the daemon with `--state-dir` and no socket argument, using `CREATE_NO_WINDOW` and null standard streams. It then retries the endpoint until the deadline. It must not launch a second daemon when the endpoint is busy or access-denied. `CREATE_NO_WINDOW` is `0x08000000`; the daemon is a console-subsystem executable, so this prevents a console window when launched from the GUI. The daemon's own sidecar launches use the Job Object path described under [Process supervision](#process-supervision).

The desktop bridge and shell replace every `&Path` socket parameter with `&Endpoint`. `DesktopBridge::connect`, `daemon_subscribe`, `daemon_unsubscribe`, `Host::start`, `Host::stop`, and the `RunEvent::Exit` cleanup all use the endpoint. The CLI's `stop`, `status`, `doctor`, and `start` paths do the same. No shared code inspects `\\.\pipe` or calls Win32 directly.

### Phase 1 tests

Automated tests must cover:

- endpoint derivation for two SIDs, two state directories, case variants, trailing separators, forward versus backward separators, and long paths;
- rejection of relative, empty, non-canonical, reparse-point, and wrong-owner state directories;
- SDDL construction and a created pipe's DACL containing exactly the current-user ACE and no inherited `Everyone` ACE;
- a second `CreateNamedPipeW` with `FILE_FLAG_FIRST_PIPE_INSTANCE` failing while the first server is alive;
- a foreign server squatting the name; the client refusing it without sending a handshake;
- a same-user but wrong-image server; a server in another session; a server whose PID is reused after exit;
- `ERROR_FILE_NOT_FOUND` mapping to absence, `ERROR_PIPE_BUSY` mapping to a live but busy endpoint, and `ERROR_ACCESS_DENIED` mapping to an error;
- overlapped read/write timeouts and cancellation unblocking `recv` promptly;
- 1 MiB frame acceptance, 1 MiB plus one rejection, missing newline rejection, and mid-stream disconnect;
- all existing handshake, command, subscription, and protocol-version fixtures over a real named pipe on Windows;
- multiple concurrent clients, a subscription stream, and a stop that closes the pipe;
- Unix tests passing unchanged after the abstraction.

Phase 1 is complete when the macOS suite still passes, the Windows pipe suite passes on a real Windows runner, and the protocol fixtures are byte-identical over both transports.

### Phase 1 real-Windows gate

Gate C in [Real Windows verification gates](#real-windows-verification-gates) must prove the ACL, first-instance, remote-client rejection, server-identity, cancellation, and shutdown behavior on Windows 10 22H2 x64 and Windows 11 x64. A mocked pipe test is not acceptance evidence.

## Phase 2: Host integration

Phase 2 adds the Windows runtime backend, inspector, installer, setup/uninstall transaction, and acceptance evidence. It starts only after Phase 1 and the real-host gates that do not require host integration have passed.

### DNS and NRPT behavior

The runtime serves `.own` through CoreDNS on `127.0.0.1:53` and the selected LAN IPv4 address on `53`, TCP and UDP. NRPT sends `.own` and every subdomain to `127.0.0.1`. Other names continue to use the existing resolver path.

NRPT has no documented custom-port parameter. `-NameServers` accepts IP addresses, not `IP:port`. CoreDNS therefore owns port 53. A future Windows release with a per-rule port could remove this constraint; this design does not assume one.

The rule is created with `Add-DnsClientNrptRule` using `-DisplayName`, `-Namespace '.own'`, `-NameServers '127.0.0.1'`, and `-Comment 'Own Domains managed rule'`, with `-PassThru`. The returned GUID is the rule identity stored in the manifest. Setup does not use a fixed display name as the removal key.

Before creating the rule, setup checks:

- the DNS Client service (`Dnscache`) is running;
- `Get-DnsClientNrptRule` has no local rule with the same display name but different namespace, name servers, or comment;
- `Get-DnsClientNrptPolicy -Effective` has no other rule for `.own`, a child of `.own`, or a catch-all namespace that would take precedence;
- no Group Policy NRPT rule for `.own` or a more specific suffix overrides the local rule;
- the `DnsClient` PowerShell module is available.

A conflicting effective policy is `nrptPolicyConflict`. Setup does not remove or edit a GPO rule, and it does not edit a foreign local rule. The user or domain administrator must resolve the conflict. **[verify]**

The runtime does not add, remove, or modify NRPT. The rule is static host integration. When CoreDNS is stopped, `.own` queries go to `127.0.0.1:53`, where nothing listens. Windows DNS Client must fail the query rather than fall back to another server. **[verify]** If Windows falls back to a public resolver, `.own` can leak and this design is blocked. There is no unprivileged, no-service way to keep a blackhole responder on 53 at all times.

Applications that bypass the system resolver do not honor NRPT: browsers with secure DNS enabled, `nslookup` with an explicit server, some Java resolvers, and applications with their own DNS stack. The setup page and documentation must tell users to use the full `.own` name and, where applicable, disable secure DNS for `.own`. The design does not modify browser settings.

NRPT is machine-wide. A second local user can resolve and reach `.own` services while the owner's runtime runs, but cannot control the daemon. Shared-device operation is a documented limitation, not a supported multi-user feature.

### DNS TTL, cache, and stop behavior

The existing DNS renderer in `crates/own-domains-core/src/dns.rs` emits a fixed 60-second TTL. The Windows renderer must take a TTL parameter and use `0` for `.own` A records. The template remains:

```text
template IN A {
    match ^(?:[a-z0-9-]+\.)+own\.$
    answer "{{ .Name }} 0 IN A <selected-lan-address>"
}
```

TTL 0 is intended to prevent the Windows DNS Client cache and browsers from serving a stale `.own` answer after CoreDNS stops or the selected address changes. **[verify]** Windows DNS Client behavior for zero-TTL records is not assumed. Gate B must test:

- resolve `setup.own`, stop the runtime, and resolve again without flushing the cache;
- change the LAN address, apply, and resolve again;
- stop, restart, and resolve again, recording any negative-cache delay;
- check `Get-DnsClientCache` and browser behavior.

If Windows caches zero-TTL records, the fallback is the shortest verified TTL, for example 5 seconds, with a documented bounded stale window. The design must not claim immediate failure after stop unless Gate B proves it. If Windows caches the record indefinitely or falls back to another resolver, the design is blocked.

The runtime does not call `Clear-DnsClientCache` on start or stop. It is a machine-wide side effect, may require elevation, and would not be scoped to `.own`. Negative caching after a failed query is a Gate B item; the remedy is to retry after the observed negative-cache interval or to document it.

### Direct low-port binding

Windows does not reserve ports below 1024 for administrators. A normal user process can bind 53, 80, and 443 when the ports are free and not in an excluded range. **[verify]** This is the foundation of the design and must be tested as a standard user, after reboot, with Hyper-V, WSL, and Docker installed and uninstalled.

CoreDNS binds exactly:

```text
127.0.0.1:53
<selected-lan-ipv4>:53
```

Caddy binds exactly:

```text
127.0.0.1:80
<selected-lan-ipv4>:80
127.0.0.1:443
<selected-lan-ipv4>:443
```

There is no `0.0.0.0`, `::`, or other wildcard listener. The setup server binds `127.0.0.1:4781`. Caddy's admin endpoint is an AF_UNIX socket or disabled; it is never a loopback TCP listener.

Caddy's HTTP app must set `protocols` to `["h1", "h2"]` on the Windows servers so HTTP/3 does not create a UDP 443 listener. **[verify]** The JSON field is `apps.http.servers.<name>.protocols`. If Caddy still opens UDP 443, the no-unplanned-listener requirement fails and the design is blocked until the configuration is corrected.

Setup and the runtime inspect listeners with `GetExtendedTcpTable` and `GetExtendedUdpTable`. They reject:

- any listener on 53, 80, 443, or 4781 that is not owned by the expected CoreDNS, Caddy, or setup server process;
- any owned listener whose local address is wildcard or is not loopback or the selected LAN address;
- any Caddy-owned UDP 443 listener;
- a selected address that is not assigned to an eligible interface.

The check is address-aware. A listener on `127.0.0.1:53` is not a conflict with a listener on the selected LAN address on 53; both are required. A wildcard listener on either port is a conflict even if it is owned by one of our processes, because it violates the binding contract.

### Excluded port ranges and conflicts

Hyper-V, WSL, Docker, and other components can reserve dynamic port ranges. Setup and doctor run:

```text
netsh interface ipv4 show excludedportrange protocol=tcp
netsh interface ipv4 show excludedportrange protocol=udp
```

If 53, 80, 443, or 4781 falls inside an excluded range, setup fails with `portExcluded` and reports the exact range. It does not change the dynamic port range, stop `winnat`, restart Hyper-V, uninstall Docker, or reboot the machine. Those are administrator actions outside this design. The remedy is to free the range or move it, then rerun setup.

Setup also checks for existing portproxy rules on 53, 80, 443, or 4781:

```text
netsh interface portproxy show all
```

A portproxy listener on a required port is a conflict and is reported as `portproxyConflict`. The design does not use portproxy, WinNAT, ICS, WinDivert, a kernel driver, or any privileged forwarding layer.

### Windows Firewall

Setup creates four inbound allow rules through the Windows Firewall COM API (`INetFwPolicy2` and `INetFwRule`), not by editing registry keys or calling `netsh advfirewall`:

| Rule name | Program | Protocol | Local port | Profile | Remote address |
| --- | --- | --- | --- | --- | --- |
| `OwnDomains-CoreDNS-UDP53` | `C:\ProgramData\OwnDomains\bin\coredns.exe` | UDP | 53 | Domain, Private | LocalSubnet |
| `OwnDomains-CoreDNS-TCP53` | `C:\ProgramData\OwnDomains\bin\coredns.exe` | TCP | 53 | Domain, Private | LocalSubnet |
| `OwnDomains-Caddy-TCP80` | `C:\ProgramData\OwnDomains\bin\caddy.exe` | TCP | 80 | Domain, Private | LocalSubnet |
| `OwnDomains-Caddy-TCP443` | `C:\ProgramData\OwnDomains\bin\caddy.exe` | TCP | 443 | Domain, Private | LocalSubnet |

All four are enabled, inbound, allow, and have edge traversal blocked. The display names are the same as the rule names. The group is `Own Domains`.

The rules are program-scoped, not address-scoped. Embedding the selected LAN address or interface in a firewall rule would require an elevated update on every DHCP renewal, address change, or Wi-Fi/Ethernet switch. Program scope is the safety boundary: only the admin-owned CoreDNS and Caddy executables can use the rules. The listeners remain loopback plus the selected address, so the rules do not create a wildcard service.

`LocalSubnet` means the local subnet(s) of the machine, not only the selected interface's subnet. Traffic from another local subnet that can route to the selected address is allowed. That is the intended LAN boundary. A narrower remote-address rule would need the DHCP-assigned subnet and would reintroduce elevation on address changes.

The Public profile is deliberately excluded. If the user marks the LAN as Public, LAN clients cannot reach `.own`; host-originated `.own` still works. The inspector reports `networkProfilePublic` with the remedy to set the network to Private. It does not call `Set-NetConnectionProfile`, which requires elevation and changes a user-visible security setting.

Setup verifies the Windows Firewall service (`mpssvc`) is running and reads the profile state. It does not enable a disabled profile, change a third-party firewall, or add a Public rule. If a third-party firewall replaces Windows Firewall, the rules may be inert; that is outside this design.

Each rule's canonical properties are serialized to a stable JSON object and hashed with SHA-256. The manifest stores the fingerprint. The inspector compares the current rule's properties with the fingerprint. If the rule is missing, `firewallRuleMissing`; if it differs, `firewallRuleMismatch`; if the firewall rules cannot be read as the current user, `firewallUnverifiable` (warning). Setup runs elevated and is authoritative.

Uninstall removes only rules whose name, program, protocol, port, profile, remote address, direction, action, enabled state, edge-traversal state, and group match the manifest. A name collision with different properties is a conflict; uninstall does not remove the foreign rule. A missing rule is already removed.

### Caddy admin

Caddy's admin endpoint is a local AF_UNIX socket inside the owner-only runtime directory:

```text
%LOCALAPPDATA%\OwnDomains\runtime\caddy-admin.sock
```

The Caddy JSON uses a relative admin address, for example `"listen": "unix/caddy-admin.sock"`, with the Caddy process working directory set to `%LOCALAPPDATA%\OwnDomains\runtime`. A relative AF_UNIX path avoids the `unix/C:\...` drive-letter parsing question. The shared `ProxyConfigOptions::for_runtime` currently requires an absolute admin socket; the Windows backend adds a Windows-only constructor or an optional relative admin name that renders this value, while macOS keeps its absolute path. **[verify]** Caddy 2.11.7 on Windows must accept a relative AF_UNIX admin address and create the socket. If it does not, the fallback is to disable the admin API entirely:

```json
"admin": { "disabled": true }
```

With admin disabled, configuration changes stop and restart Caddy with the previous configuration retained for rollback. Product decision (2026-10-26): this restart fallback is not accepted as a shipping mode; Gate F must prove the AF_UNIX live-`/load` path, otherwise the design is blocked and requires a new decision. The runtime reports `caddyAdminUnavailable` as an error, not a warning, until Gate F passes. Caddy admin is never exposed on TCP, because any local process could otherwise reconfigure Caddy and proxy arbitrary local services.

The admin socket is inside a directory whose ACL is current user full, SYSTEM and Administrators full, and no other users. Same-user processes can use it; they can already read the CA key and settings. The design does not add a second authentication layer to the AF_UNIX socket.

Before starting Caddy, the runtime removes a stale `caddy-admin.sock` only after verifying that no recorded Caddy process is alive and the socket does not accept a connection. It never unlinks a live admin socket. Caddy's behavior when the socket file already exists is a Gate F item. **[verify]**

### Certificate trust

Setup reuses the Caddy local CA generated in the target user's state directory. Only the public root certificate is installed. The private key stays at:

```text
%LOCALAPPDATA%\OwnDomains\caddy-data\pki\authorities\local\root.key
```

with an ACL that grants only the current user, SYSTEM, and Administrators. The certificate is:

```text
%LOCALAPPDATA%\OwnDomains\caddy-data\pki\authorities\local\root.crt
```

Trust is installed in the current user's root store, not the machine store:

```text
certutil -addstore -user Root "<state-dir>\caddy-data\pki\authorities\local\root.crt"
```

If the store already contains the same SHA-1 thumbprint, setup verifies the DER fingerprint and skips the add instead of treating an existing certificate as a failure.

Removal uses the certificate's SHA-1 thumbprint:

```text
certutil -delstore -user Root "<sha1-thumbprint>"
```

The manifest records both `caThumbprintSha1` (the 40-hex certificate-store thumbprint used by `certutil`) and `caFingerprintSha256` (the 64-hex DER digest used for integrity checks). Uninstall recomputes both from the state-directory certificate and the store before removing anything. A mismatch is `caFingerprintMismatch`; uninstall refuses to remove a certificate it does not own.

`certutil -addstore -user Root` may display the standard Windows root-certificate confirmation dialog. It is not a UAC prompt, but it is an additional consent dialog. **[verify]** Gate G must record whether it appears, whether it can be suppressed without broadening trust, and whether removal prompts. Product decision (2026-10-26): up to two dialogs are accepted (one UAC prompt plus the certificate confirmation). The alternative of installing the CA in `LocalMachine\Root` during the elevated phase is rejected: it would trust the CA for every user and weaken least privilege. This design keeps `CurrentUser\Root`.

Trust installation happens in the unprivileged user phase, after the elevated phase. The elevated process may run under a different administrator account when a standard user enters credentials, so writing `CurrentUser\Root` from the elevated process could trust the administrator's profile instead of the target user's. The elevated phase never touches user trust.

`certutil` updates the Windows certificate store. Firefox may use its own NSS store, and some Java runtimes use their own `cacerts`. Those applications may need a manual CA import from the setup page. Setup and doctor do not modify browser or Java trust stores.

### Setup server

The setup server remains the existing read-only HTTP server on `127.0.0.1:4781`. Caddy proxies `setup.own` to it. It serves only the public CA certificate and fixed setup guidance; it exposes no management route, settings, logs, private keys, or IPC. There is no firewall rule for 4781, so LAN clients reach it only through Caddy on 443 and never directly.

The setup server is not a privileged service. It runs in the user daemon, binds loopback only, and stops with the runtime.

### Address and interface changes

The selected LAN address is stored in `settings.json` (schema 1) and is never written to ProgramData, NRPT, firewall rules, or the manifest. Interface discovery uses `GetAdaptersAddresses(AF_INET, ...)` and `GetIfEntry2`.

An eligible interface is:

- Ethernet (`IF_TYPE_ETHERNET_CSMACD`) or IEEE 802.11 (`IF_TYPE_IEEE80211`);
- operationally up;
- not loopback, tunnel, PPP, WWAN, or another non-LAN type;
- marked `HardwareInterface` and `ConnectorPresent` by `GetIfEntry2`;
- not in the virtual-adapter denylist: Hyper-V, VMware, VirtualBox, Docker, WSL, VPN, WireGuard, Tailscale, ZeroTier, TAP, Bluetooth, and similar.

The denylist is explicit and matched against the adapter description and friendly name. A new virtual-adapter class requires a design update, not a silent wildcard. USB Ethernet and Wi-Fi adapters must pass the hardware flags; Gate E records the behavior. **[verify]**

The selected address must be a global unicast IPv4 address and must be the primary IPv4 address of an eligible interface. Aliases, link-local addresses, loopback, multicast, unspecified addresses, and addresses on ineligible adapters are rejected. The runtime finds the interface by address; settings do not store an interface name.

Address changes are unprivileged:

1. The daemon saves the new value at the expected revision.
2. Runtime reconciliation rebuilds the CoreDNS and Caddy configurations with the new address.
3. Caddy reloads through the admin socket or restarts with the new config; CoreDNS restarts.
4. Readiness probes the new address on 53, 80, and 443.
5. NRPT, firewall rules, trust, and the manifest are unchanged.

If the new address is active on an eligible interface, reconciliation succeeds. If it is not active, the settings remain persisted, the runtime reports `lanAddressInactive`, and the previous applied configuration is retained where the existing reconciliation semantics allow it. The GUI remedy is to select an active address, not to rerun setup.

A DHCP renewal that changes the address makes the saved value stale until the user saves the new one. The runtime does not silently rewrite settings. It reports `lanAddressInactive` and the GUI can offer the new active address.

Switching between Ethernet and Wi-Fi, plugging in a new USB Ethernet adapter, or connecting a new physical interface requires no setup. The firewall rules are program-scoped and NRPT is machine-wide; only the settings address changes. This is the main reason direct binding and program-scoped firewall rules were selected over interface-scoped or address-scoped artifacts.

A new address on a virtual, VPN, or tunnel adapter is `lanAddressIneligible`. A new address on an interface marked Public is still usable on the host but not from the LAN; the inspector reports `networkProfilePublic`. A new address on a physical adapter that is not in the denylist is accepted without setup.

## SetupRequest reuse

`SetupRequest` stays exactly as it is:

```text
state_directory
lan_address
interface
```

No platform field, SID field, firewall flag, or trust flag is added. `UninstallRequest` stays exactly:

```text
state_directory
purge
```

The fields are initial selection inputs, not persistent privileged state:

- `state_directory` is the target user's state root.
- `lan_address` is the initial selected LAN IPv4 address.
- `interface` is the initial selected LAN interface.

Setup validates that `interface` is present, eligible, and has `lan_address` as its primary IPv4 address. It then discards both values from the privileged request. The selected address is persisted in `settings.json` by the unprivileged phase for the first runtime launch. It is never written to the NRPT rule, a firewall rule, the sidecar path, or the manifest.

Adding `target_user_sid` to `SetupRequest` would move the Windows token model into the shared contract. The Windows installer derives the target SID from the request file owner and the state-directory ACL, then records it in its own manifest. A future platform that needs an interface index can add an optional field behind the facade; this design does not.

## Windows install manifest

The Windows manifest is platform-specific and independent of the macOS schema 2 manifest and the Linux schema 1 manifest. It is stored at:

```text
C:\ProgramData\OwnDomains\install-manifest.json
```

It is admin-owned, admin-writable, and readable only by the owning user in addition to Administrators and SYSTEM. It contains no LAN address, interface name, interface GUID, DHCP state, subnet, or other mutable network value.

```json
{
  "schemaVersion": 1,
  "platform": "windows",
  "hostIntegrationVersion": 1,
  "ownerSid": "S-1-5-21-...",
  "ownerAccount": "DESKTOP-1234\\alice",
  "installRoot": "C:\\ProgramData\\OwnDomains",
  "backupDirectory": "C:\\ProgramData\\OwnDomains\\backups\\20261026T101500Z-<request-id>",
  "sidecars": {
    "schemaVersion": 1,
    "coredns": {
      "version": "1.14.7",
      "sha256": "<64 lowercase hex>",
      "path": "C:\\ProgramData\\OwnDomains\\bin\\coredns.exe"
    },
    "caddy": {
      "version": "2.11.7",
      "sha256": "<64 lowercase hex>",
      "path": "C:\\ProgramData\\OwnDomains\\bin\\caddy.exe"
    }
  },
  "nrpt": {
    "name": "{00000000-0000-0000-0000-000000000000}",
    "displayName": "Own Domains (.own)",
    "namespace": ".own",
    "nameServers": ["127.0.0.1"],
    "comment": "Own Domains managed rule"
  },
  "firewallRules": [
    {
      "name": "OwnDomains-CoreDNS-UDP53",
      "program": "C:\\ProgramData\\OwnDomains\\bin\\coredns.exe",
      "protocol": "UDP",
      "localPort": 53,
      "profiles": ["Domain", "Private"],
      "remoteAddress": "LocalSubnet",
      "direction": "Inbound",
      "action": "Allow",
      "enabled": true,
      "edgeTraversal": false,
      "group": "Own Domains",
      "fingerprintSha256": "<64 lowercase hex>"
    },
    {
      "name": "OwnDomains-CoreDNS-TCP53",
      "program": "C:\\ProgramData\\OwnDomains\\bin\\coredns.exe",
      "protocol": "TCP",
      "localPort": 53,
      "profiles": ["Domain", "Private"],
      "remoteAddress": "LocalSubnet",
      "direction": "Inbound",
      "action": "Allow",
      "enabled": true,
      "edgeTraversal": false,
      "group": "Own Domains",
      "fingerprintSha256": "<64 lowercase hex>"
    },
    {
      "name": "OwnDomains-Caddy-TCP80",
      "program": "C:\\ProgramData\\OwnDomains\\bin\\caddy.exe",
      "protocol": "TCP",
      "localPort": 80,
      "profiles": ["Domain", "Private"],
      "remoteAddress": "LocalSubnet",
      "direction": "Inbound",
      "action": "Allow",
      "enabled": true,
      "edgeTraversal": false,
      "group": "Own Domains",
      "fingerprintSha256": "<64 lowercase hex>"
    },
    {
      "name": "OwnDomains-Caddy-TCP443",
      "program": "C:\\ProgramData\\OwnDomains\\bin\\caddy.exe",
      "protocol": "TCP",
      "localPort": 443,
      "profiles": ["Domain", "Private"],
      "remoteAddress": "LocalSubnet",
      "direction": "Inbound",
      "action": "Allow",
      "enabled": true,
      "edgeTraversal": false,
      "group": "Own Domains",
      "fingerprintSha256": "<64 lowercase hex>"
    }
  ],
  "caThumbprintSha1": "<40 lowercase hex>",
  "caFingerprintSha256": "<64 lowercase hex>",
  "createdAt": "2026-10-26T10:15:00Z",
  "updatedAt": "2026-10-26T10:15:00Z"
}
```

Validation rules:

- `schemaVersion` must be `1`, `platform` must be `windows`, and `hostIntegrationVersion` must match the app build's expected version.
- `ownerSid` must be a valid SID string and must equal the target user SID. `ownerAccount` must equal `LookupAccountSidW(ownerSid)` in `DOMAIN\name` form. A mismatch is `manifestOwnerMismatch`.
- `installRoot` must be exactly `C:\ProgramData\OwnDomains`. `backupDirectory` must be absolute, inside `C:\ProgramData\OwnDomains\backups`, free of `..` and reparse points, and owned by Administrators/SYSTEM.
- `sidecars` must contain exactly `coredns` and `caddy`, with the fixed paths above, the pinned versions, and lowercase 64-hex SHA-256 digests. The actual files must match both the manifest and the compile-time pins.
- `nrpt.name` must be a valid GUID; it is normalized to the braced form used by `Get-DnsClientNrptRule` and `Remove-DnsClientNrptRule`. `displayName`, `namespace`, `nameServers`, and `comment` must be exactly the values above. No additional namespaces or name servers are allowed.
- `firewallRules` must contain exactly the four rule names above, each with the fixed program, protocol, port, profiles, remote address, direction, action, enabled state, edge-traversal state, and group. Each `fingerprintSha256` must match the canonical rule properties when the rule is readable. Every rule must be enabled, have edge traversal blocked, and belong to the `Own Domains` group.
- `caThumbprintSha1` must be 40 lowercase hex characters; `caFingerprintSha256` must be 64 lowercase hex characters.
- `createdAt` and `updatedAt` must be RFC 3339 UTC timestamps. Unknown fields are rejected.

The manifest cannot widen the uninstall path set. Uninstall validates the fixed install root, the fixed sidecar paths, the fixed rule names, and the recorded NRPT GUID and properties before removing anything. A tampered manifest that adds a path or rule name is invalid, not authoritative.

### ACLs

The ACLs are explicit and protected, not inherited from `C:\ProgramData` or `%LOCALAPPDATA%`:

| Path | DACL |
| --- | --- |
| `C:\ProgramData\OwnDomains` | Administrators/SYSTEM full; `BU` read and traverse |
| `C:\ProgramData\OwnDomains\bin` | Administrators/SYSTEM full; `BU` read and execute |
| `C:\ProgramData\OwnDomains\backups` | Administrators/SYSTEM full only |
| `C:\ProgramData\OwnDomains\install-manifest.json` | Administrators/SYSTEM full; owner SID read |
| `%LOCALAPPDATA%\OwnDomains` | owner SID full; SYSTEM and Administrators full; no other users |

The install root is created by elevated setup only if it does not exist. If it exists with a different owner or a DACL that grants write access to a non-administrator, setup fails with `programDataInsecure`. It does not take ownership of a directory another user created in ProgramData.

The state directory is created and hardened by the unprivileged user phase before the elevated phase. If it is a reparse point, a junction, or owned by another SID, setup fails with `stateDirectoryInsecure`. The daemon repeats the owner and reparse-point check at start.

## Setup changes

`own-domains setup` remains dry-run by default. `own-domains setup --apply` is the only privileged setup path. The CLI keeps `--lan-address` and `--interface` as initial selection inputs. On Windows the command is:

```text
own-domains.exe setup --apply --lan-address 192.168.1.42 --interface "Ethernet"
```

The GUI never runs setup. It shows the diagnostic and the exact command.

### Preview

Preview is unprivileged and read-only. It validates the request and reports create, update, or unchanged actions for:

- `C:\ProgramData\OwnDomains\bin\coredns.exe`
- `C:\ProgramData\OwnDomains\bin\caddy.exe`
- `C:\ProgramData\OwnDomains\install-manifest.json`
- the NRPT rule
- the four firewall rules
- the CurrentUser root certificate

It also reports whether UAC is expected (a fully matching readable installation skips it), the CA generation or reuse, and the final probes. Preview must not create the state directory, generate the CA, write any file, add an NRPT rule, create a firewall rule, change trust, or launch an elevated process.

If preview cannot read a privileged artifact as the current user, it reports that item as unverifiable and still does not claim it is correct. Setup runs elevated and is authoritative.

### Preflight

The user phase rejects the operation before staging a request when any of these checks fail:

- The state directory is not absolute, contains `..`, is a reparse point, is not owned by the current user, or grants write access to another non-administrator SID.
- The CA pair is inconsistent and cannot be safely repaired as the current user. Existing CA files must be owned by the current user, and the private key must not be readable by other non-administrator SIDs. A repairable pair is backed up under the state directory before regeneration; the old files are never silently deleted.
- The CA cannot be generated or repaired as the current user.
- The selected interface is absent, ineligible, or its primary IPv4 address differs from `lan_address`.
- The runtime cannot be stopped. The user phase sends `Command::Stop` and waits for the endpoint to disappear before staging the request; if the endpoint remains, setup aborts.
- The app resources do not contain both Windows sidecars and the runtime sidecar manifest, or the sidecars do not match the compile-time pins.
- The setup binary is unsigned in a production build, or its Authenticode signer does not match the pinned publisher.
- The current process is already elevated. The user phase refuses with `alreadyElevated`; it does not skip UAC, treat the elevated token as a privileged path, or invoke the hidden elevated entry point implicitly. The remedy is to run `setup --apply` from a non-elevated terminal. The hidden `--elevated-setup` entry point is the only elevated mode and requires a staged request.

The elevated phase repeats every check that can be affected by a race and adds:

- The request file is owned by `targetUserSid`, is not a reparse point, is at most 64 KiB, and has a valid UUID and action.
- `lan_address` is a parsed IPv4 address; `interface` is a validated eligible adapter name; `state_directory` canonicalizes to a path owned by `targetUserSid`.
- The ProgramData install root is absent or owned by Administrators/SYSTEM with the expected DACL.
- The sidecar source files in the app resource directory are regular files with the pinned SHA-256. The elevated process copies them to ProgramData and verifies the copied bytes again. It never executes a sidecar from the user-writable app directory.
- Ports 53 TCP/UDP, 80, 443, and 4781 are free on the exact bind addresses, except for a stopped Own Domains runtime that has already released them.
- `netsh interface ipv4 show excludedportrange protocol=tcp` and `protocol=udp` do not cover 53, 80, 443, or 4781.
- `netsh interface portproxy show all` has no rule on 53, 80, 443, or 4781.
- `Dnscache` and `mpssvc` are running.
- The `DnsClient` PowerShell module and `certutil.exe` are present.
- No conflicting local or effective NRPT rule for `.own` exists.
- No foreign firewall rule with one of the four fixed names exists. An existing owned rule is backed up and updated; a same-name rule with different program, protocol, port, profile, remote address, direction, action, enabled state, edge-traversal state, or group is a conflict.
- The backup directory can be created and written, and the volume has enough free space for the sidecars, backups, and logs.
- No incomplete setup transaction exists. If one does, setup rolls it back before proceeding or stops with `setupInterrupted` if rollback fails.
- No other elevated setup is running. A global named mutex `Global\OwnDomainsSetup` with an Administrators/SYSTEM-only DACL serializes setup. It is a one-shot mutex, not a command channel.

The selected network profile is checked but is not a hard failure. A Public profile produces a `networkProfilePublic` warning because LAN clients will not reach `.own`; host `.own` still works. Setup does not change the profile.

### Elevation exchange

Setup uses exactly one UAC prompt through `ShellExecuteExW` with `lpVerb = "runas"` and `SEE_MASK_NOCLOSEPROCESS`. The unprivileged process stages a request file and launches the same executable with a hidden entry point:

```text
own-domains.exe --elevated-setup "<request-path>"
```

The request path is under:

```text
%LOCALAPPDATA%\OwnDomains\setup\request-<uuid>.json
```

The request contains only:

```json
{
  "schemaVersion": 1,
  "requestId": "<uuid>",
  "action": "setup",
  "targetUserSid": "S-1-5-21-...",
  "stateDirectory": "C:\\Users\\alice\\AppData\\Local\\OwnDomains",
  "lanAddress": "192.168.1.42",
  "interface": "Ethernet"
}
```

The unprivileged process creates the `setup` directory with the state-directory ACL and writes the request atomically. The elevated process opens the request with `FILE_FLAG_OPEN_REPARSE_POINT`, verifies its owner and DACL, reads at most 64 KiB, and treats every field as untrusted input. It never interpolates a field into a shell command. Fixed PowerShell scripts read a validated JSON file and pass values to `Add-DnsClientNrptRule`, `Set-DnsClientNrptRule`, or `Remove-DnsClientNrptRule` as parameters.

The elevated process writes a result to:

```text
%LOCALAPPDATA%\OwnDomains\setup\result-<uuid>.json
```

with a DACL that grants the target user full control plus Administrators/SYSTEM. The result contains the request ID, outcome, actions, failure code and message, unresolved items, and the manifest SHA-256. The unprivileged process verifies the request ID and then re-inspects the manifest and artifacts; it does not trust the result file as proof of success.

The unprivileged process waits on the elevated process handle with a bounded timeout (five minutes for setup, two minutes for uninstall). `ERROR_CANCELLED` from `ShellExecuteExW` is `elevationCancelled` and changes nothing. Any other `ShellExecuteExW` failure is `elevationFailed`; it is not retried automatically. A timeout is `elevationTimeout`; the elevated process may still be running, so the user must not start a second setup until the mutex is released.

A fully matching installation skips UAC. The user phase compares the manifest owner, host integration version, sidecar versions and hashes, NRPT GUID and properties, firewall rule fingerprints, and CA fingerprints. If every privileged item matches, it does not stage a request. If any item is missing, drifted, or unreadable, it stages one request. The elevated phase is idempotent and reports unchanged items without rewriting them.

`CREATE_NO_WINDOW` cannot be passed through `ShellExecuteExW`. The elevated entry point uses a hidden window and `SW_HIDE`; if a console window still flashes, the implementation may add a small GUI-subsystem elevation helper that calls the same library. That helper is not a service or a persistent process. **[verify]**

If the user phase detects that its own token is already elevated, it does not call `ShellExecuteExW`. It fails with `alreadyElevated` before staging a request, so the target user identity stays unambiguous and the one-UAC contract is not replaced by an implicit privileged path. The same check applies to `uninstall --apply`.

### Apply

The unprivileged phase runs first:

1. Validate the state directory and request.
2. Stop any running runtime and wait for the named pipe to disappear.
3. Create or reuse the Caddy local CA with `install_trust: false`, a temporary owner-only admin socket, and the target user's storage root. On Windows the child environment retains `SystemRoot`, `WINDIR`, `TEMP`, `TMP`, `USERPROFILE`, `APPDATA`, and `LOCALAPPDATA`; it is not cleared to an empty environment. `HOME` points at the user profile. Caddy runs as the current user, never elevated.
4. Compute the CA SHA-1 thumbprint and DER SHA-256 fingerprint.
5. Inspect the privileged state. If it fully matches, skip elevation.
6. Otherwise stage the request and run the elevated phase.
7. Re-inspect the manifest and privileged artifacts.
8. Install the public CA certificate into `CurrentUser\Root` with `certutil -addstore -user Root`, unless the same thumbprint is already present and verified.
9. Verify the certificate is present and matches the manifest fingerprint.
10. Persist the selected LAN address in `settings.json` atomically, without resetting mappings or upstream DNS.
11. Remove the request and result files. Keep the user-phase journal until the trust and settings steps succeed.

The elevated phase performs these steps transactionally:

1. Verify the Authenticode signature and publisher of its own image in production builds. The development override is described under [Security boundaries](#security-boundaries).
2. Acquire `Global\OwnDomainsSetup`.
3. Validate the request, target SID, state directory, ProgramData root, and all preflight conditions.
4. Roll back an incomplete prior transaction if one exists.
5. Create `C:\ProgramData\OwnDomains\backups\<timestamp>-<request-id>` and the transaction journal. Back up every owned artifact that will change.
6. Copy `coredns.exe` and `caddy.exe` from the app resources to a temporary file in `C:\ProgramData\OwnDomains\bin`, verify SHA-256 against the compile-time pins, set the bin ACL, and replace the destination with `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`. Back up the previous executable when it changes.
7. Create, update, or verify the NRPT rule. If the rule is absent, add it and capture the returned GUID. If the manifest owns a rule with the same GUID, use `Set-DnsClientNrptRule` to restore the exact namespace, name servers, display name, and comment. If a foreign rule conflicts, stop and roll back.
8. Create, update, or verify the four firewall rules through `INetFwPolicy2`. Remove and recreate an owned rule whose properties drifted; back up its previous definition first. A foreign same-name rule is a conflict.
9. Write the manifest atomically with `ReplaceFileW` or `MoveFileExW`, then apply the manifest ACL. The manifest is written last, after all host artifacts and sidecars.
10. Run final probes: sidecar hashes, NRPT rule by GUID, effective NRPT policy, firewall rule fingerprints, ProgramData ACLs, CA certificate and key presence, and the absence of portproxy rules on the required ports.
11. Write the result file, release the mutex, and exit 0.

Setup does not bind 53, 80, or 443 itself. It runs as the administrator, not the target user, and a bind test would not prove the target user's runtime can bind. The first unprivileged `start` is the real bind test. If it fails, the runtime reports `portConflict` or `portExcluded`; setup is still installed and can be uninstalled. This is intentional: setup does not start a privileged listener to prove a user listener will work.

Setup is idempotent. Re-running with the same sidecars, NRPT rule, firewall rules, and host integration version reports the artifacts unchanged, revalidates them, and does not rewrite them. Re-running after a sidecar or host integration version change updates the manifest and changed artifacts. Re-running after a firewall rule drift repairs the owned rule and records a backup. A foreign artifact remains a conflict.

### Rollback

The transaction journal is an admin-only JSON file in the backup directory. It records each step with its kind, target, previous value or backup path, and completion state. Writes are atomic and flushed before the next step.

On failure, rollback runs in reverse order:

- remove firewall rules created by this transaction;
- restore firewall rules whose definitions were replaced;
- restore the NRPT rule to its previous properties or remove it if it was created;
- restore replaced sidecars from the backup;
- restore the previous manifest or remove it if it was created;
- remove files and directories created by this transaction that are empty;
- leave the backup directory and journal for diagnosis, or remove them if rollback fully succeeds.

Rollback continues after an individual failure and collects every unresolved item. `SetupFailure.unresolved` lists the exact path, rule name, NRPT GUID, or trust action. The CLI exits with code 4. A failed setup must not leave a partially trusted CA, a firewall rule pointing at a missing sidecar, or a manifest that claims artifacts that are not installed.

If the elevated process is terminated mid-transaction, the journal remains `inProgress`. The next elevated setup detects it and runs rollback before any new work. If rollback cannot complete, setup stops with `setupInterrupted` and lists the unresolved items. It does not overwrite the journal.

The unprivileged phase also has a small journal at:

```text
%LOCALAPPDATA%\OwnDomains\setup\user-journal.json
```

It records the request ID, CA thumbprints, and whether trust and settings were written. It is user-only. If the process exits after the elevated phase but before trust or settings, rerunning setup sees a fully matching privileged installation, skips UAC, and retries only the user-phase steps.

## Uninstall changes

`own-domains uninstall` remains dry-run by default. `own-domains uninstall --apply` is the only privileged uninstall path. `--purge` requires `--apply`.

Uninstall requires the runtime to be stopped. The user phase connects to the named pipe, sends `Command::Stop`, and waits for the endpoint to disappear. If a listener on 53, 80, 443, or 4781 remains owned by an Own Domains process, uninstall returns `runtimeStillRunning` instead of removing host integration under a live runtime.

### Preview

Preview reads the manifest, validates its owner and fixed paths, and lists, in reverse install order:

- stop the runtime;
- remove the four firewall rules after fingerprint verification;
- remove the NRPT rule after GUID and property verification;
- remove `coredns.exe` and `caddy.exe` after hash verification;
- remove the backup directory;
- remove `install-manifest.json` last;
- remove the CurrentUser root certificate after thumbprint and fingerprint verification;
- preserve or purge the state directory.

Preview does not remove anything. If the manifest is missing or corrupt, it reports the fixed paths for manual inspection and does not guess.

### Apply

The user phase:

1. Refuses with `alreadyElevated` before doing anything if its own token is elevated.
2. Stops the runtime and verifies the endpoint and owned listeners are gone.
3. Reads the manifest and computes the CA thumbprints from the state-directory certificate.
4. Writes the uninstall journal at `%LOCALAPPDATA%\OwnDomains\uninstall.json` with the CA thumbprints, request ID, and state.
5. If the manifest is absent and the uninstall journal records that the elevated phase already completed, skip the elevated phase and continue with trust removal. Otherwise stage an uninstall request in the same request/result exchange used by setup, with `action: "uninstall"` and no LAN address or interface.
6. Runs the elevated phase once.
7. Re-inspects that the privileged artifacts are gone.
8. Removes the CurrentUser root certificate with `certutil -delstore -user Root "<sha1-thumbprint>"` and verifies it is absent.
9. Removes the uninstall journal.
10. With `--purge`, removes the state directory only after trust removal is verified. Without `--purge`, settings, mappings, logs, and the CA remain, but the CA is no longer trusted.

The elevated phase:

1. Verifies the request, target SID, state directory, and manifest owner.
2. Acquires `Global\OwnDomainsSetup`.
3. Validates the manifest against the fixed allowlist before removing anything.
4. Removes the four firewall rules by name after verifying their properties and fingerprints. A missing rule is already removed. A same-name rule with different properties is a conflict; it is not removed.
5. Removes the NRPT rule by GUID after verifying namespace, name servers, display name, and comment. A missing rule is already removed. A different rule with the same GUID is a conflict.
6. Removes `coredns.exe` and `caddy.exe` after verifying their hashes match the manifest. A hash mismatch is `sidecarDigestMismatch`; the file is not removed.
7. Removes the backup directory and any transaction journals.
8. Removes `install-manifest.json` last.
9. Writes the result file and exits 0.

Uninstall is resumable and idempotent:

- It removes the manifest last, after all host artifacts are gone.
- If a step fails, it stops and reports the unresolved item; rerunning uninstall resumes from the manifest.
- If the manifest is missing or corrupt, the elevated phase refuses and reports the fixed paths for manual inspection. It does not delete arbitrary files.
- If only the CurrentUser trust remains because the elevated phase already succeeded, the user phase can remove it using the uninstall journal or the state-directory CA certificate without another UAC prompt.
- If trust removal fails, the journal remains and the command reports `caTrustRemovalFailed`. A later `uninstall --apply` sees no manifest, skips UAC, and retries trust removal.
- `--purge` refuses if trust removal is unresolved or the CA is still present in the CurrentUser store. It removes settings, mappings, logs, and the CA only after host integration is gone.

A missing NRPT rule, missing firewall rule, or missing sidecar is not an error during uninstall. A present artifact with the wrong properties, wrong hash, or wrong owner is a conflict. This prevents uninstall from removing another administrator's rule or a replaced executable.

## Runtime and process supervision

`WindowsRuntimeBackend` implements the existing `RuntimeBackend` contract. It does not add a protocol command or GUI surface.

### Paths

```text
%LOCALAPPDATA%\OwnDomains\settings.json
%LOCALAPPDATA%\OwnDomains\runtime\
%LOCALAPPDATA%\OwnDomains\runtime\candidate\
%LOCALAPPDATA%\OwnDomains\runtime\candidate\Corefile
%LOCALAPPDATA%\OwnDomains\runtime\candidate\caddy.json
%LOCALAPPDATA%\OwnDomains\runtime\candidate\revision
%LOCALAPPDATA%\OwnDomains\runtime\applied\
%LOCALAPPDATA%\OwnDomains\runtime\applied\Corefile
%LOCALAPPDATA%\OwnDomains\runtime\applied\caddy.json
%LOCALAPPDATA%\OwnDomains\runtime\applied\revision
%LOCALAPPDATA%\OwnDomains\runtime\previous\
%LOCALAPPDATA%\OwnDomains\runtime\previous\Corefile
%LOCALAPPDATA%\OwnDomains\runtime\previous\caddy.json
%LOCALAPPDATA%\OwnDomains\runtime\processes\
%LOCALAPPDATA%\OwnDomains\runtime\caddy-admin.sock
%LOCALAPPDATA%\OwnDomains\logs\coredns.log
%LOCALAPPDATA%\OwnDomains\logs\coredns.err.log
%LOCALAPPDATA%\OwnDomains\logs\caddy.log
%LOCALAPPDATA%\OwnDomains\logs\caddy.err.log
%LOCALAPPDATA%\OwnDomains\caddy-data\
%LOCALAPPDATA%\OwnDomains\caddy-config\
C:\ProgramData\OwnDomains\bin\coredns.exe
C:\ProgramData\OwnDomains\bin\caddy.exe
```

The daemon process lock remains `%LOCALAPPDATA%\OwnDomains\own-domains.lock` and uses `LockFileEx` through `std::fs::File::try_lock`. The state directory and its children use the owner-only DACL described under [ACLs](#acls).

### Start

`start` runs as the target user and performs the existing sequence:

1. Load settings and require a valid LAN address.
2. Inspect the manifest, sidecars, NRPT, firewall, trust, ports, excluded ranges, listeners, and surviving process records.
3. Stop verified stale Own Domains children. Never kill an ambiguous PID.
4. Stage and validate candidate CoreDNS and Caddy configurations with the Windows ports and TTL.
5. Start the setup server on `127.0.0.1:4781`.
6. Create the Job Object and start CoreDNS and Caddy.
7. Wait for readiness.
8. Promote the candidate and persist child ownership records and the applied revision.

The runtime does not add or remove NRPT, firewall rules, or trust. Those are static host artifacts. A missing or drifted NRPT rule, firewall rule, or sidecar is an error diagnostic that blocks start and points at setup. It does not attempt an unprivileged repair. Missing CA trust is a warning; the runtime can start, and the setup page explains how to install it.

### Process supervision

The runtime creates one Job Object per runtime with:

- `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`;
- no breakaway limit;
- an active-process limit high enough for the sidecars and any Caddy helper processes, for example 16;
- a non-inheritable job handle.

CoreDNS and Caddy are assigned to the job. When the daemon exits, crashes, or is terminated, the last job handle closes and Windows terminates every process in the job. This is the crash backstop that replaces the Unix process group and `SIGKILL` sweep.

The preferred launch path is `CreateProcessW` with `EXTENDED_STARTUPINFO_PRESENT` and `PROC_THREAD_ATTRIBUTE_JOB_LIST`, so the child is in the job before it runs user code. **[verify]** The job handles in the attribute list must be valid for the child and must not be inherited in a way that keeps the job alive after the daemon exits. The implementation creates the job handle non-inheritable and verifies `IsProcessInJob` after creation.

If `PROC_THREAD_ATTRIBUTE_JOB_LIST` is unavailable or fails, the fallback is:

1. `CreateProcessW` with `CREATE_SUSPENDED | CREATE_NO_WINDOW`.
2. `AssignProcessToJobObject(job, process)`.
3. `ResumeThread`.

If assignment fails, terminate the suspended child and fail start with `processSupervisionUnavailable`. The runtime never leaves a sidecar running outside the job.

`CREATE_NO_WINDOW` is used for every sidecar launch. The daemon redirects the child's stdout and stderr to the log files above, sets the working directory to the runtime directory, and passes an explicit environment. It does not use a shell, a `.bat` file, or `cmd.exe`. The child is a console-subsystem executable, so `CREATE_NO_WINDOW` prevents a console window from appearing when the GUI starts the runtime.

If the daemon is already inside a job that forbids assignment or has `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK`, nested assignment may fail or the child may escape. The runtime calls `IsProcessInJob(GetCurrentProcess(), NULL)` and queries the current job's limits. If supervision cannot be guaranteed, start fails with `processSupervisionUnavailable`; it does not run sidecars unsupervised. **[verify]**

### Process identity and stop

Windows has no Unix process group or `SIGTERM`. The Windows `ProcessIdentity` is:

```text
pid
creation_time (FILETIME)
executable canonical path
arguments excluding argv[0]
```

`process_group` is recorded as `0` and is not used. The process supervisor verifies all four identity fields with `OpenProcess`, `QueryFullProcessImageNameW`, and `GetProcessTimes` before terminating a recovered process. A PID that has been reused with a different creation time is not an Own Domains child and is never killed.

Stop behavior:

- Caddy: if the admin socket is available, request a graceful stop through the admin API. If it is not available, use `TerminateProcess` and wait.
- CoreDNS: `TerminateProcess`; CoreDNS has no graceful stop channel in this design.
- Wait for both children with a bounded timeout.
- Close the Job Object handle, which terminates any surviving descendants.
- Stop the setup server.
- Remove the owned process records.
- Verify that no owned listener remains on 53, 80, 443, or 4781.

`TerminateProcess` does not run Caddy or CoreDNS shutdown handlers. Caddy's certificate cache and config files are written atomically, so a forced stop should not corrupt them, but in-flight requests are dropped. This is a documented limitation. **[verify]** The Job Object kill is the final backstop and is not a substitute for graceful Caddy stop when the admin socket is available.

### Reconcile and Caddy admin fallback

`reconcile` stages and validates a new configuration, then:

- if the Caddy admin socket is available, `POST /load` the new configuration; if the load fails, keep the previous configuration and report the error;
- if the admin socket is disabled, stop Caddy, write the new configuration, start Caddy with the new config, and wait for readiness; if start or readiness fails, restore the previous applied `caddy.json`, start Caddy with it, and report the failure;
- always stop and restart CoreDNS with the new Corefile, because CoreDNS has no reload;
- if the new configuration fails, restore the previous applied Corefile and restart CoreDNS.

The fallback path is slower and briefly interrupts HTTPS. It is the only supported path when Gate F shows that Caddy cannot use a Windows AF_UNIX admin socket.

### Readiness

Readiness must prove all of:

- direct UDP and TCP DNS queries to `127.0.0.1:53` and the selected LAN address on 53 return the selected LAN address for `setup.own` with the expected TTL;
- the system resolver, using `DnsQuery_W` with standard NRPT processing, resolves `setup.own` to the selected LAN address;
- the setup server answers on `127.0.0.1:4781`;
- Caddy answers HTTP on `127.0.0.1:80` with `Host: setup.own` and HTTPS on `127.0.0.1:443` with SNI `setup.own`, and the certificate chains to the state-directory CA;
- the Caddy admin socket connects if admin is enabled; if admin is disabled, the Caddy process is alive and the HTTPS probe passes;
- listener inspection shows no wildcard listener and no Caddy-owned UDP 443;
- the selected LAN address is active on an eligible interface.

The system resolver probe is the real NRPT test. A direct query to `127.0.0.1:53` proves CoreDNS is alive but not that Windows sends `.own` to it. If the system resolver probe fails, the runtime reports `nrptNotEffective` and blocks start. It retries with bounded backoff for a short window to allow NRPT and DNS Client state to settle. **[verify]**

The HTTPS probe verifies the certificate chain against the CA file, not the Windows trust store. Trust is a separate user-facing concern; a user who has not installed the CA can still start the runtime, and the setup page explains how to install it. The inspector reports `caNotTrusted` as a warning until the CurrentUser store contains the CA.

## Inspection and diagnostics

`WindowsHostInspector` validates:

- the manifest structure, platform, host integration version, owner SID, owner account, install root, backup path, and ACL;
- the two sidecar files: presence, version, path, SHA-256, and compile-time pin;
- the NRPT rule by GUID and properties, and the effective NRPT policy for `.own` and its children;
- the four firewall rules by name and fingerprint, and the Windows Firewall service and profile state;
- the CurrentUser root certificate by SHA-1 thumbprint and DER SHA-256;
- the selected address is the primary IPv4 address of an eligible interface;
- ports 53 TCP/UDP, 80, 443, and 4781 on the exact bind addresses;
- excluded port ranges and portproxy rules on the required ports;
- owned listeners: no wildcard, no unselected address, no Caddy UDP 443;
- owned process records and Job Object membership;
- the Caddy admin socket state and the selected reload strategy;
- the state-directory ACL, reparse-point state, and owner.

The inspector runs as the target user. NRPT, firewall, and some listener state may be unreadable without elevation. When a source is unreadable, it reports `nrptUnverifiable`, `firewallUnverifiable`, or `listenerUnverifiable` as a warning and does not claim the policy is safe. Setup runs elevated and is authoritative. The direct DNS, system-resolver, and HTTPS probes remain the runtime's authoritative evidence.

New diagnostics:

| Code | Meaning | Remedy |
| --- | --- | --- |
| `setupRequired` | The manifest is missing or incompatible with this build | Run `setup --apply` once |
| `setupManifestInvalid` | The manifest is corrupt, has an unknown field, or fails schema validation | Inspect the manifest and rerun setup |
| `manifestOwnerMismatch` | The installation is owned by another user SID | Only the owner can manage it; an administrator must remove it manually |
| `sidecarMissing` | A pinned sidecar is absent | Rerun setup |
| `sidecarDigestMismatch` | A sidecar's SHA-256 differs from the manifest or pin | Rerun setup; do not execute the file |
| `sidecarVersionMismatch` | A sidecar's version differs from the manifest | Rerun setup |
| `nrptRuleMissing` | The manifest's NRPT GUID is absent | Rerun setup |
| `nrptRuleMismatch` | The rule's namespace, name servers, display name, or comment differs | Rerun setup |
| `nrptPolicyConflict` | A GPO or other effective rule overrides `.own` | Resolve the policy conflict; setup does not edit it |
| `nrptNotEffective` | The system resolver did not return the selected address for `setup.own` | Check the rule, Dnscache, and GPO; rerun setup |
| `nrptUnverifiable` | NRPT state cannot be read as the target user (warning) | Check with an elevated PowerShell session; setup is authoritative |
| `firewallRuleMissing` | A required firewall rule is absent | Rerun setup |
| `firewallRuleMismatch` | A rule's program, port, protocol, profile, remote address, direction, action, enabled state, or edge-traversal state differs | Rerun setup |
| `firewallUnverifiable` | Firewall rules cannot be read as the target user (warning) | Check with an elevated PowerShell session; setup is authoritative |
| `listenerUnverifiable` | Listener ownership or binding cannot be read as the target user (warning) | Check with an elevated PowerShell session; the runtime probes remain authoritative |
| `networkProfilePublic` | The selected network is on the Public profile, so LAN `.own` is blocked (warning) | Set the network to Private |
| `portConflict` | A required port is occupied on a conflicting address | Stop the conflicting application |
| `portproxyConflict` | A portproxy rule owns a required port | Remove the portproxy rule; this design does not use portproxy |
| `portExcluded` | A required port is inside a Hyper-V/WSL/Docker excluded range | Free or move the range, then rerun setup; no portproxy fallback |
| `unexpectedListener` | An Own Domains process bound a wildcard or unselected address | Stop the runtime and report a bug; do not use the listener |
| `http3ListenerDetected` | Caddy owns UDP 443 | Disable HTTP/3 and restart; the no-unplanned-listener contract is violated |
| `lanAddressInactive` | The selected address is not assigned to any interface | Select an active LAN address |
| `lanAddressIneligible` | The selected address belongs to a virtual, VPN, tunnel, or otherwise ineligible adapter | Select a physical Ethernet or Wi-Fi address |
| `caNotTrusted` | The CA is absent from `CurrentUser\Root` (warning until HTTPS is used) | Rerun setup or import the CA from the setup page |
| `caFingerprintMismatch` | The stored certificate does not match the manifest or state-directory CA | Remove and reinstall trust; rerun setup |
| `caTrustInstallFailed` | The CurrentUser root certificate could not be installed | Check the certificate file and `certutil` output, then rerun setup |
| `caTrustRemovalFailed` | Uninstall could not remove the CurrentUser certificate | Rerun `uninstall --apply`; the journal resumes trust removal |
| `processSupervisionUnavailable` | A Job Object could not be created or assigned | Stop; do not run sidecars unsupervised; report the job-in-job state |
| `staleOwnedProcess` | A recorded child survived a daemon exit | Start terminates it only after identity verification; ambiguous PIDs are reported |
| `caddyAdminUnavailable` | The AF_UNIX admin socket is unavailable; live reload is unproven (error until Gate F passes) | Block shipping until Gate F proves AF_UNIX live reload; restart fallback requires a new product decision |
| `ipcEndpointBusy` | Another process owns the named pipe | Stop the conflicting process; do not connect to it |
| `ipcServerIdentityMismatch` | The pipe server is not the expected daemon | Close the connection and investigate the endpoint |
| `elevationCancelled` | The user cancelled the UAC prompt | Rerun setup or uninstall |
| `elevationFailed` | The elevated process could not be started | Check the UAC policy and Authenticode signature, then retry |
| `alreadyElevated` | `setup --apply` or `uninstall --apply` was started from an already-elevated process | Rerun the command from a non-elevated terminal; use the UAC prompt |
| `elevationTimeout` | The elevated process did not finish in the bounded time | Wait for the mutex, inspect the result, then retry |
| `setupInProgress` | Another elevated setup holds the global mutex | Wait for it to finish |
| `setupInterrupted` | An incomplete transaction journal exists and rollback failed | Inspect the listed paths and rerun setup |
| `setupResultUnavailable` | The elevated result file is missing or unreadable | Inspect the manifest and rerun setup |
| `uninstallPending` | An uninstall journal remains from a partial uninstall | Rerun uninstall; it resumes trust removal |
| `programDataInsecure` | The install root has a foreign owner or an unsafe DACL | Fix the ACL manually or remove the foreign directory |
| `stateDirectoryInsecure` | The state directory is a reparse point, has a foreign owner, or grants write access to another user | Fix the ACL or recreate the directory |
| `authenticodeUntrusted` | The setup binary's signature or publisher is not trusted | Use a signed production build |
| `runtimeStillRunning` | Uninstall found a live endpoint or owned listener | Stop the runtime and retry |
| `inspectionFailed` | A host inspection command or API failed | Read the message and retry; setup is authoritative |
| `runtimeUnavailable` | This build has no Windows runtime backend | Install a build with Windows support |

Existing codes keep their meaning: `setupRequired`, `setupManifestInvalid`, `sidecarMissing`, `sidecarDigestMismatch`, `sidecarVersionMismatch`, `caNotTrusted`, `portConflict`, `lanAddressInactive`, `inspectionFailed`, and `runtimeUnavailable`.

Diagnostic precedence:

- A missing, corrupt, or wrong-owner manifest produces `setupRequired`, `setupManifestInvalid`, or `manifestOwnerMismatch` and stops further artifact checks.
- A sidecar hash mismatch produces `sidecarDigestMismatch` and blocks start. It is not downgraded to a warning.
- NRPT and firewall drift are separate errors, one per failed condition, never duplicated.
- `lanAddressIneligible` wins over `lanAddressInactive` when the address exists on an ineligible adapter. `lanAddressInactive` is used when no interface owns the address.
- `portExcluded` wins over `portConflict` when the port is reserved rather than held by a process.
- `http3ListenerDetected` and `unexpectedListener` are errors, not warnings. They violate the binding contract even if HTTP still works.
- `networkProfilePublic`, `caNotTrusted`, `caddyAdminUnavailable`, `nrptUnverifiable`, `firewallUnverifiable`, and `listenerUnverifiable` are warnings. They do not block start unless the corresponding readiness probe fails.
- A stopped runtime may save an address that is not active. Start fails with `lanAddressInactive` or `lanAddressIneligible` until the address is active on an eligible interface.

The GUI displays the diagnostic message and remedy. It must not instruct the user to rerun setup for an ordinary address change or an interface switch. It must not offer a button that elevates.

## Security boundaries

- Only `own-domains setup --apply` and `own-domains uninstall --apply` run elevated, each through one UAC prompt. An already-elevated user invocation is refused with `alreadyElevated`; the hidden `--elevated-setup` entry point is the only elevated mode and requires a staged request. Start, stop, status, mapping edits, address changes, interface switches, doctor, and GUI launch run as the target user.
- There is no privileged helper daemon, Windows service, scheduled task, privileged named pipe, privileged TCP listener, or auto-elevating GUI action.
- The named pipe DACL grants only the current user. `PIPE_REJECT_REMOTE_CLIENTS` rejects remote clients. The server rejects clients in another session. The client verifies the server image, SID, session, creation time, and, in production, Authenticode publisher before sending a request.
- The daemon and sidecars run as the target user, not as an administrator. The Job Object is a supervision mechanism, not a privilege escalation.
- ProgramData sidecars are admin-owned and read-only to users. Firewall rules are program-scoped to those exact paths. The runtime never executes a sidecar from `%LOCALAPPDATA%\Own Domains` or from `PATH`.
- The elevated phase verifies its own Authenticode signature and publisher in production. A development build may set `OWN_DOMAINS_ALLOW_UNSIGNED_SETUP=1`; the override prints a warning and is rejected when the variable is absent. The override is not a production path.
- The elevated phase treats the request file as untrusted. It verifies the file owner, DACL, reparse state, size, action, UUID, SID, state-directory owner, address, and interface. It never interpolates request fields into a shell command.
- PowerShell is invoked with a fixed app-owned script and a validated JSON request. It is not invoked with user-controlled command text. The `DnsClient` module is used only for NRPT; firewall rules use COM.
- The install root and state directory use protected DACLs. Setup rejects a foreign ProgramData directory or an insecure state directory instead of taking ownership.
- Only the public CA certificate is installed. The private key stays in the owner-only state directory. Trust is per-user in `CurrentUser\Root`. Uninstall verifies both thumbprints before removal.
- The setup page is read-only, loopback-only, and exposes only the public certificate and fixed guidance. Caddy proxies it; no management route, settings, logs, private keys, or IPC are exposed.
- Caddy admin is AF_UNIX only and lives in the owner-only runtime directory. It is never a loopback TCP listener. If AF_UNIX is unavailable, admin is disabled rather than exposed on TCP.
- CoreDNS and Caddy bind only to loopback and the selected LAN address. HTTP/3 is disabled. No wildcard listener, portproxy, WinNAT, ICS, WinDivert, or kernel driver is used.
- NRPT is machine-wide. A second local user can resolve `.own` while the owner's runtime runs but cannot control the daemon. The manifest records the owner SID and setup refuses takeover by another user.
- The Windows Firewall rules are inbound, program-scoped, Domain/Private only, and LocalSubnet only. The design does not modify firewall policy, network profiles, VPNs, browser trust stores, or Java trust stores.
- Uninstall validates the manifest against a fixed allowlist and removes only exact owned artifacts. A tampered manifest cannot widen the removal set.
- Administrator access can always modify or remove these artifacts. This design protects against other standard users and unprivileged processes, not against a malicious administrator.

## Sidecars and packaging

Windows uses the same pinned CoreDNS and Caddy versions as macOS and Linux, with Windows release assets:

| Component | amd64 target | arm64 target | Asset |
| --- | --- | --- | --- |
| CoreDNS | `x86_64-pc-windows-msvc` | `aarch64-pc-windows-msvc` | `coredns_1.14.7_windows_amd64.zip`; arm64 only if `coredns_1.14.7_windows_arm64.zip` exists |
| Caddy | `x86_64-pc-windows-msvc` | `aarch64-pc-windows-msvc` | `caddy_2.11.7_windows_amd64.zip`; arm64 only if `caddy_2.11.7_windows_arm64.zip` exists |

The exact pinned versions are the versions already used by the macOS and Linux sidecar manifests. This design does not invent hashes. The build must download the published assets, verify the published checksums, extract the `.exe`, and pin the extracted executable SHA-256.

The build manifest gains an additive `windowsReleases` map under each sidecar:

```json
"windowsReleases": {
  "amd64": {
    "target": "x86_64-pc-windows-msvc",
    "url": "...",
    "archiveMember": "coredns.exe",
    "checksum": { "algorithm": "sha256", "url": "...", "value": "..." },
    "executableSha256": "..."
  },
  "arm64": { "...": "..." }
}
```

The macOS and Linux release maps are unchanged. The fetch and verification scripts gain:

- `--platform windows`, defaulting to the host platform;
- ZIP extraction with an archive-member allowlist;
- `.exe` executable names and the Windows target triples;
- the same URL allowlist, redirect limit, size limits, checksum verification, and Caddy checksum-signature verification as macOS;
- per-target runtime manifests: `manifest-<target>.json` with the existing `schemaVersion: 1` runtime shape.

The Windows executable hashes are pinned from the published release checksums and the extracted PE binaries. macOS and Linux hashes are never reused. arm64 is build-only only if both upstream projects publish Windows arm64 assets and the fetch script can verify them; if CoreDNS does not publish a Windows arm64 asset, arm64 is not built and the support matrix says so. **[verify]**

The app resources contain:

```text
$INSTDIR\resources\sidecars\coredns-x86_64-pc-windows-msvc.exe
$INSTDIR\resources\sidecars\caddy-x86_64-pc-windows-msvc.exe
$INSTDIR\resources\sidecars\manifest-x86_64-pc-windows-msvc.json
```

The Tauri NSIS `currentUser` package installs the app under:

```text
%LOCALAPPDATA%\Own Domains
```

The CLI, daemon, and desktop executable are in `$INSTDIR`; the sidecars are resources under `$INSTDIR\resources\sidecars`. The exact `$INSTDIR` expansion and resource mapping must be verified against the Tauri version in use. **[verify]** The app installation is user-writable, so elevated setup never trusts these files. It verifies the sidecar SHA-256 against compile-time pins embedded in the signed CLI/daemon, copies the bytes to ProgramData, and verifies the copied bytes again. The runtime executes only the ProgramData copies.

A sidecar version or hash change requires one elevated setup. The runtime never downloads or updates sidecars. A missing or mismatched ProgramData sidecar is `sidecarMissing` or `sidecarDigestMismatch`, not a reason to search `PATH` or fall back to the app resources.

## Persistence and upgrades

### Reboot

NRPT and the four firewall rules persist across reboot. The runtime does not auto-start at login or boot. Before the user starts the app or CLI, `.own` queries go to a closed `127.0.0.1:53` and fail while other names resolve normally. LAN clients cannot resolve `.own`; ports 53, 80, and 443 on the selected address are not listening, so `.own` fails and no wildcard service is exposed.

After reboot, the user launches the GUI or runs `own-domains start`. No UAC prompt appears. The runtime re-inspects the manifest, sidecars, NRPT, firewall, trust, ports, and selected address, then starts.

### Windows Fast Startup and sleep

Fast Startup and sleep do not change the design. NRPT and firewall rules persist. The runtime process and sidecars are terminated by the OS on shutdown; the Job Object also closes. After resume or a fast-startup boot, the user starts the runtime again. The selected address may have changed; the inspector reports `lanAddressInactive` and the GUI offers the new address. No elevation is required.

### App upgrade

Host artifacts live in ProgramData, outside the app package. A normal app upgrade does not remove or rewrite them.

- If the sidecar versions, host integration version, NRPT shape, firewall shape, and manifest schema are unchanged, no setup is required. The new app validates the existing manifest and starts normally.
- If the sidecars or host integration version change, the inspector reports `setupRequired`. The user runs `setup --apply` once. The fully matching fast path does not apply because the pins changed.
- If the app resource layout changes, the setup source path changes, but the ProgramData destination and firewall program paths do not. The manifest records the destination, not the app resource path.
- Uninstall uses the manifest written by the last successful setup, not the current app's defaults, except for the fixed allowlist validation.

### OS upgrade

This design targets Windows 10 22H2 x64 and Windows 11 x64. A feature update may change NRPT, Windows Firewall, named-pipe behavior, Caddy AF_UNIX support, or adapter metadata. The inspector detects drift, but support is not claimed for a new build until the acceptance suite is rerun. A setup rerun may be required after a major OS upgrade.

Windows 10 22H2 is the minimum baseline. It is out of mainstream support; this design still targets it because the product requirement includes Windows 10. Security updates and the exact build number are recorded in the acceptance evidence.

### Multi-user and multiple installations

NRPT and ports 53/80/443 are machine-wide. Only one Own Domains runtime can run at a time, regardless of which user starts it. The manifest records the owning user SID. Setup by another user is `manifestOwnerMismatch`; it does not take over the installation. Uninstall by another user is also refused. An administrator can remove the artifacts manually after stopping the runtime.

Two different state directories for the same user produce two different named pipes, but the second daemon fails to bind 53/80/443 and reports `portConflict`. The process lock prevents two daemons for the same state directory. The design does not support concurrent multi-user or multi-installation operation.

### Uninstall and reinstall

Without `--purge`, uninstall removes host integration and trust but keeps settings, mappings, logs, and the CA. A later setup reuses the CA identity and trusts it again. With `--purge`, the state directory is removed after trust removal is verified. A reinstall after purge generates a new CA.

## Migration

There is no supported Windows installation to migrate. The current Windows build has no runtime, no IPC endpoint, and no host artifacts; the first supported build starts from a fresh state directory and one setup run. macOS and Linux host state is not imported.

### From the unsupported Windows build

- Install the supported app over the existing per-user package. The NSIS `currentUser` installer replaces `%LOCALAPPDATA%\Own Domains`; it does not create or modify ProgramData artifacts.
- Run `own-domains.exe setup --apply --lan-address <address> --interface "<name>"` once. There is no manifest to preserve, so setup creates the sidecars, NRPT rule, firewall rules, manifest, and CurrentUser trust.
- If `%LOCALAPPDATA%\OwnDomains` already exists, setup validates its owner, reparse state, and CA pair. Valid schema-1 settings and a matching CA may be reused; an invalid CA is not trusted and must be repaired or removed before setup. Setup never treats a ProgramData path as user state.
- A pre-existing `.own` NRPT rule, same-name firewall rule, portproxy/WinNAT rule, or listener on 53/80/443/4781 is a conflict. Setup reports it and does not adopt, edit, or remove it. The user removes the foreign artifact manually, then reruns setup.

### From macOS or Linux

- Host integration is not portable. PF anchors, `/etc/resolver`, launchd state, keychain trust, Linux resolver files, and Linux trust stores are not imported.
- Settings schema 1 is unchanged, but `settings.json` is not a supported migration artifact because it contains the platform-specific LAN address and paths. Recreate mappings through the GUI or CLI on Windows; the `.own` names and mapping semantics are unchanged.
- The Caddy CA is per state directory. A CA copied from another platform is not trusted automatically. The supported path is to let Windows setup generate a CA, or to validate an existing Windows CA pair before trusting it.

### Downgrade and rollback

- A manifest with a higher `hostIntegrationVersion`, sidecar pin, or schema than the running app supports produces `setupRequired`; the older app does not edit or remove it.
- To downgrade, stop the runtime and run `uninstall --apply` with the newer build before installing the older build. The older build then runs its own setup. Uninstall with the older build is refused if the manifest is outside its fixed allowlist.
- A failed setup rolls back to the previous manifest and artifacts. The backup directory retains the previous manifest and journal for diagnosis; it is not a migration mechanism.
- An app upgrade with unchanged host integration and sidecar pins needs no setup. A sidecar or host-integration change needs one elevated setup; user settings and mappings are preserved.

## Testing and acceptance

Automated coverage must include:

### Windows IPC and transport

- endpoint derivation and canonicalization cases;
- pipe DACL, first-instance, remote-client rejection, session rejection, busy, access-denied, and absence;
- client server-identity verification, including wrong image, wrong SID, wrong session, and PID reuse;
- overlapped timeouts, cancellation, concurrent clients, subscriptions, and stop;
- all protocol fixtures over a real pipe;
- Unix tests passing unchanged after the abstraction;
- desktop and CLI endpoint plumbing, including `--socket` on Unix and `--pipe` on Windows.

### Host rendering and manifest

- CoreDNS Windows rendering with ports 53, TTL 0, loopback plus selected address, and no wildcard;
- Caddy Windows rendering with ports 80/443, `protocols: ["h1", "h2"]`, AF_UNIX admin or disabled, and storage under the state directory;
- manifest serialization, validation, owner SID, fixed paths, extra-field rejection, and fingerprint formats;
- firewall rule canonical JSON and fingerprint stability;
- NRPT rule property parsing, GUID validation, and conflict detection;
- adapter eligibility fixtures for Ethernet, Wi-Fi, USB Ethernet, Hyper-V, VMware, VirtualBox, WSL, VPN, WireGuard, Tailscale, ZeroTier, Bluetooth, loopback, and tunnel;
- excluded-port-range parsing and portproxy detection, including `portproxyConflict`;
- listener table fixtures for loopback, selected address, wildcard, and Caddy UDP 443.

### Transaction and uninstall

- preview changes nothing and does not elevate;
- idempotent setup for the same sidecars and artifacts;
- setup skipping UAC when everything matches;
- one UAC prompt when a privileged artifact is missing or changed;
- already-elevated `setup --apply` and `uninstall --apply` refuse with `alreadyElevated` and stage nothing;
- request file owner, DACL, reparse, size, UUID, action, SID, address, and interface validation;
- result file request ID and manifest re-inspection;
- elevation start failure, cancellation, timeout, and result-file absence;
- CA trust install failure and resumable trust removal failure;
- sidecar copy, hash verification, and atomic replacement;
- NRPT add, set, remove, and rollback;
- firewall create, update, remove, fingerprint mismatch, and foreign-name conflict;
- manifest write last and ACL enforcement;
- rollback after sidecar failure, NRPT failure, firewall failure, manifest failure, and final-probe failure;
- incomplete transaction rollback on the next setup;
- uninstall preview/apply, manifest allowlist, sidecar hash refusal, NRPT GUID mismatch, firewall property mismatch, trust removal, resumable trust removal, and `--purge`;
- migration from an unsupported Windows build: pre-existing state directory, valid settings/CA reuse, invalid CA refusal, and foreign NRPT/firewall/portproxy conflict reporting;
- uninstall refusal while the runtime is live;
- Authenticode rejection and the development override.

### Inspector and runtime

- missing, corrupt, wrong-owner, wrong-platform, wrong-version, and extra-field manifests;
- sidecar, NRPT, firewall, trust, port, excluded-range, listener, adapter, and profile diagnostics;
- `nrptUnverifiable`, `firewallUnverifiable`, and `listenerUnverifiable` warnings;
- no duplicate diagnostics and the documented precedence;
- address change without setup, Ethernet/Wi-Fi switch, new USB adapter, inactive address, ineligible address, and Public profile;
- runtime start/reconcile/stop with fake process, file, registry, firewall, and readiness adapters;
- failed candidate retaining the previous applied state;
- Caddy admin load and restart fallback;
- Job Object child-tree termination, kill-on-close, no console window, assignment fallback, and job-in-job refusal;
- stale process record identity verification and ambiguous-PID refusal;
- system resolver probe and `nrptNotEffective`;
- frontend save behavior proving an address change does not produce `setupRequired`;
- platform-facade contract tests with fake Windows/macOS/Linux installers and inspectors;
- the existing macOS and Linux suites passing unchanged.

### Packaging and release

- Windows sidecar fetch and verify for amd64 and, if available, arm64;
- ZIP member allowlist, archive checksum, executable SHA-256, and version checks on matching hosts;
- Caddy checksum-signature verification;
- NSIS `currentUser` layout and `$INSTDIR\resources\sidecars` resolution;
- no execution of arm64 binaries on an x86_64 host;
- macOS and Linux release checks still passing.

Run the repository's normal gates: `pnpm test`, `pnpm run lint`, `pnpm run typecheck`, `pnpm run build`, `cargo fmt --check`, `cargo clippy`, and the release checks. Windows-specific tests must run on Windows 10 22H2 x64 and Windows 11 x64, not only on macOS or Linux.

### Manual acceptance

On physical Windows 10 22H2 x64 and Windows 11 x64 hosts:

1. Preview setup and confirm no files, rules, trust, or elevation.
2. Apply setup and confirm exactly one UAC prompt, plus any root-certificate confirmation dialog. Record the dialog count.
3. Confirm `coredns.exe`, `caddy.exe`, `install-manifest.json`, the NRPT rule, the four firewall rules, and the CurrentUser certificate.
4. Start from the GUI and CLI. Confirm `.own` DNS over UDP and TCP, HTTP, HTTPS, `setup.own`, and a phone on the same LAN.
5. Stop; confirm `.own` fails on the host and phone while other names resolve, and record any stale-cache or negative-cache delay. Restart and confirm recovery.
6. Change the LAN address on the same interface; save it; confirm rebinding without UAC and without setup.
7. Switch between Ethernet and Wi-Fi; confirm rebinding without UAC.
8. Plug in a new USB Ethernet adapter; select its address; confirm it works without setup.
9. Reboot; confirm NRPT and firewall persist and `.own` fails before start. Start without UAC and confirm recovery.
10. Set the network profile to Public; confirm the LAN diagnostic and that host `.own` still works. Set it back to Private; confirm LAN works without setup.
11. Install Hyper-V, WSL, and Docker. Confirm excluded port ranges are reported. If 53/80/443/4781 are excluded, confirm setup fails with the exact remedy and does not fall back to portproxy.
12. Confirm no wildcard listener and no UDP 443 listener owned by Caddy. Confirm Caddy admin is AF_UNIX with working live `/load`; a disabled-admin restart fallback does not satisfy this step.
13. Install and remove the CA with `certutil`; confirm the setup page, Firefox, Chrome, and Java behavior. Record the trust prompt behavior.
14. Run uninstall; confirm one UAC prompt, removal of NRPT, firewall rules, sidecars, manifest, and trust, and preservation of settings without `--purge`.
15. Run `--purge`; confirm settings and CA are removed after trust removal.
16. Kill the daemon; confirm the sidecars die with it and no orphan listener remains. Restart and confirm recovery.
17. Attempt to connect to the pipe from another user, another session, and a squatted server; confirm refusal.
18. Upgrade the app with unchanged sidecars and confirm no setup; change a sidecar and confirm one setup.
19. Record the exact Windows builds, UAC state, Hyper-V/WSL/Docker versions, adapter names, network profiles, phone model, browser versions, and command outputs in `docs/windows-acceptance.md`.
20. Install over a development build with a pre-existing `%LOCALAPPDATA%\OwnDomains` directory; confirm valid settings/CA reuse or the invalid-CA diagnostic, and confirm a foreign NRPT, firewall, or portproxy artifact is reported as a conflict rather than adopted.

arm64 is build-only until an arm64 host or runner records the same evidence. No runtime support is claimed before that record exists.

## Real Windows verification gates

No gate below has been run. The design is not implementation-ready until the gates pass and the results are recorded in `docs/windows-acceptance.md`. A failed gate blocks the affected part of the design; it must not be worked around with a privileged helper, service, portproxy, or wildcard listener.

| Gate | What must be proved | Status |
| --- | --- | --- |
| A. Low-port binding | A standard user can bind 53 TCP/UDP, 80, and 443 on loopback and the selected LAN address, after reboot, with Hyper-V/WSL/Docker installed and uninstalled; excluded ranges are detected and reported; no elevation is required | Not run |
| B. NRPT and DNS | The local NRPT rule sends `.own` to `127.0.0.1`; effective policy conflicts are detected; stopping CoreDNS fails `.own` without public fallback; TTL 0 cache/stop/restart behavior is recorded; negative caching is bounded; `Dnscache` is required | Not run |
| C. Named pipe | The protected DACL excludes `Everyone`; `PIPE_REJECT_REMOTE_CLIENTS` rejects remote clients; first-instance squatting fails; server identity verification rejects wrong image/SID/session; cancellation and shutdown work; protocol fixtures pass | Not run |
| D. Job Objects | Sidecars are assigned before running or via suspend/assign/resume; kill-on-close kills the child tree when the daemon exits; no console window appears; job-in-job and silent-breakaway cases fail closed; no orphan survives a crash | Not run |
| E. Firewall and adapters | The four COM rules are program-scoped, Domain/Private, LocalSubnet, inbound, enabled, edge-traversal blocked, and grouped under `Own Domains`; Public profile blocks LAN `.own`; unprivileged inspection is accurate or reports unverifiable; adapter eligibility accepts USB Ethernet/Wi-Fi and rejects virtual/VPN/tunnel adapters | Not run |
| F. Caddy admin and HTTP/3 | Caddy accepts a relative AF_UNIX admin address on Windows, handles a pre-existing stale socket without blocking start, and serves live `/load`; `protocols: ["h1","h2"]` prevents a UDP 443 listener. Restart fallback alone does not pass this gate | Not run |
| G. Trust | `certutil -addstore -user Root` installs the CA; the confirmation dialog behavior is recorded; `certutil -delstore -user Root` removes it; fingerprint mismatch is refused; Firefox/Java behavior is recorded | Not run |
| H. Setup and uninstall | Exactly one UAC prompt for a normal setup and one for a normal uninstall; fully matching setup skips UAC; an already-elevated invocation is refused; request/result validation; Authenticode rejection; rollback and resumability; manifest ACL and owner; `--purge` ordering | Not run |
| I. Reboot, upgrade, and network changes | NRPT/firewall survive reboot; no auto-start; address/DHCP/interface changes need no UAC; app upgrade with unchanged artifacts needs no setup; sidecar change needs one setup | Not run |
| J. Physical phone | DNS, HTTP, HTTPS, CA installation through `setup.own`, and loss of `.own` after stop work on a real phone; no management endpoint is exposed | Not run |
| K. Packaging | NSIS `currentUser` layout, `$INSTDIR\resources\sidecars`, sidecar ZIP fetch/verify, and desktop launch work on a clean Windows install | Not run |
| L. arm64 | Build and package for `aarch64-pc-windows-msvc` only if both upstream assets exist; run the same gates on arm64 hardware before claiming runtime support | Not run |

Specific syntax and behavior that must be verified, not assumed:

- non-administrator `bind` on 53/80/443 after reboot and with Hyper-V/WSL/Docker installed;
- `netsh interface ipv4 show excludedportrange` output and `netsh interface portproxy show all` parsing;
- `Add-DnsClientNrptRule -DisplayName ... -PassThru` returning the GUID in `Name` and the exact brace format; `Set-DnsClientNrptRule` preserving the GUID; `Remove-DnsClientNrptRule -Name` removing exactly that rule;
- `Get-DnsClientNrptRule` readability as a standard user; `Get-DnsClientNrptPolicy -Effective` conflict detection; GPO precedence;
- Windows DNS Client zero-TTL caching, negative caching, and stop/restart timing;
- named-pipe default ACLs, explicit DACL application, `PIPE_REJECT_REMOTE_CLIENTS`, `FILE_FLAG_FIRST_PIPE_INSTANCE`, `ConnectNamedPipe` overlapped cancellation, and `WaitNamedPipe` busy semantics;
- `GetNamedPipeServerProcessId`/`GetNamedPipeClientProcessId`, token SID, session ID, and process creation time behavior;
- `PROC_THREAD_ATTRIBUTE_JOB_LIST` on Windows 10 22H2; job-handle inheritance; nested jobs; `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK`; `CREATE_NO_WINDOW`;
- `INetFwPolicy2` rule properties, `LocalSubnet` semantics, profile bitmask, and unprivileged read access;
- Caddy 2.11.7 relative AF_UNIX admin address on Windows and stale-socket behavior; `apps.http.servers.<name>.protocols`; HTTP/3 listener behavior;
- `certutil -addstore -user Root` and `-delstore -user Root` output, prompt behavior, and thumbprint format;
- `GetAdaptersAddresses`/`GetIfEntry2` `HardwareInterface` and `ConnectorPresent` values for USB Ethernet, Wi-Fi, Hyper-V, VMware, VirtualBox, WSL, VPN, and tunnel adapters;
- `GetExtendedTcpTable`/`GetExtendedUdpTable` listener ownership and wildcard detection;
- Tauri NSIS `currentUser` `$INSTDIR` and resource layout;
- Windows amd64/arm64 sidecar asset availability and checksums;
- Caddy CA generation on Windows with the restricted environment and explicit storage root;
- `certutil` CurrentUser root prompt and browser/Java trust behavior;
- Authenticode verification and publisher pinning;
- `File::try_lock` using `LockFileEx` on Windows;
- the `DnsClient` PowerShell module's availability and PowerShell execution policy.

## Risks and fallbacks

- **Low-port binding fails.** If a standard user cannot bind 53/80/443, the design is blocked. The fallback is not portproxy, a service, a helper, or a driver; it is a new design decision with a different privilege model.
- **NRPT does not fail closed.** If Windows falls back to another resolver when CoreDNS is stopped, `.own` can leak. The design is blocked; a blackhole responder would require a persistent process or service.
- **TTL 0 is cached.** Use the shortest verified TTL and document the stale window. If the cache is indefinite or stop behavior is unacceptable, the design is blocked.
- **Named-pipe squatting or ACL defaults differ.** The explicit DACL and client server-identity check are mandatory. If they cannot be made reliable, Phase 1 is blocked; there is no fallback to a TCP loopback port, because that would expose a local command channel.
- **Job Object assignment fails.** If `PROC_THREAD_ATTRIBUTE_JOB_LIST` and suspend/assign/resume both fail, or the daemon is in a job that permits silent breakaway, start fails with `processSupervisionUnavailable`. Running unsupervised sidecars is not acceptable.
- **Caddy AF_UNIX admin fails.** Disable admin and use stop/start with previous-config rollback. This is a supported degraded mode with brief HTTPS downtime. Exposing admin on TCP is not acceptable.
- **CurrentUser trust prompts.** The standard root-certificate confirmation may add a second dialog. If the product requires exactly one dialog, machine-wide trust in the elevated phase is the alternative, but it trusts the CA for every user. That is a design change.
- **Excluded port ranges.** Setup fails with the exact range. The user must adjust Hyper-V/WSL/Docker or the dynamic port range. The design does not modify them.
- **Public network profile.** LAN `.own` is blocked until the user sets the network to Private. The design does not change the profile automatically.
- **Third-party firewall, VPN, or EDR.** These can block 53/80/443 or named pipes. The design detects what it can and documents the rest as unsupported.
- **Windows 10 22H2 lifecycle.** The baseline may receive fewer updates or change behavior after a feature update. Acceptance must be rerun for each claimed Windows build.
- **Sidecar asset availability.** If CoreDNS or Caddy does not publish a Windows arm64 asset, arm64 is not built. x86_64 acceptance does not prove arm64.
- **Authenticode signing.** Production setup must be signed. An unsigned development build is not a production path.

## Known limitations

- IPv4 only. IPv6-only LANs and IPv6 `.own` records are unsupported.
- Windows 10 22H2 x64 and Windows 11 x64 only. Windows Server, older Windows 10 builds, Windows Sandbox, WSL guests, Hyper-V guests, and containers are unsupported.
- arm64 is build-only until hardware acceptance.
- Setup and uninstall require one UAC prompt each. Setup may also show the Windows root-certificate confirmation dialog.
- No persistent service or helper. The runtime does not auto-start at login or boot.
- NRPT is machine-wide. A second local user can resolve `.own` while the owner's runtime runs but cannot control the daemon. Concurrent multi-user operation is unsupported.
- Ports 53, 80, and 443 must be free, and 4781 must be free on loopback. Another service on those ports, or a Hyper-V/WSL/Docker excluded range, blocks the design.
- `.own` fails while the runtime is stopped. DNS Client negative caching may delay the first successful query after a restart; TTL 0 may be cached by Windows or the browser. The acceptance record captures the observed delay.
- A LAN client that uses the host as its only DNS server loses all DNS while the runtime is stopped, not just `.own`. Clients that use the host only for `.own` need their own split-DNS configuration.
- Applications that bypass NRPT (browser secure DNS, `nslookup` with an explicit server, some Java resolvers) do not resolve `.own`. The design does not change their settings.
- Firefox and Java may not use the Windows certificate store. Manual CA import may be required.
- The Public network profile blocks LAN `.own`. The user must set the network to Private; the design does not change it.
- Third-party firewalls, VPN clients, and EDR products may block ports or named pipes. They are outside the supported matrix.
- HTTP/3 is disabled. Browsers use HTTP/1.1 or HTTP/2. A Caddy-owned UDP 443 listener is a contract violation.
- Caddy admin is AF_UNIX or disabled. The disabled fallback causes brief HTTPS downtime on configuration changes and does not support live reload.
- `TerminateProcess` is forceful for CoreDNS and for Caddy when admin is disabled. In-flight requests are dropped; Caddy's atomic files are expected to survive, but this is not graceful shutdown.
- A Job Object kill is the crash backstop. If the daemon is in a job that permits silent breakaway or assignment fails, start is refused.
- No wildcard listener. LAN clients must reach the selected LAN address; another interface's address is not served.
- A new address on a virtual, VPN, or tunnel adapter is rejected. `.own` over VPN is unsupported.
- A sidecar update requires one elevated setup. The runtime never downloads or updates sidecars.
- The app installation is per-user and user-writable. Elevated setup relies on Authenticode and compile-time hashes; a user who runs an unsigned development binary is outside the production security boundary.
- The CA private key is protected by the user's Windows profile. A compromised user account can sign certificates trusted by that user's store. This is inherent to per-user trust.
- The setup page is read-only. It cannot fix NRPT, firewall, port, or trust problems; those require the documented remedies.
- A machine-wide manifest owned by one user means a second user cannot set up or uninstall the installation without an administrator removing it first.
- The design does not rotate logs. Long-running verbose logs can grow in the state directory.
- The design does not support Windows Store/MSIX distribution, automatic updates of ProgramData sidecars, or enterprise deployment via Group Policy.
- The design does not add Windows Defender/EDR exclusions. Organizations that require them must add them separately.

## Documentation

Update:

- `README.md`: Windows support status, the one-time setup boundary, and the no-service model.
- `docs/platform-support.md`: Windows implementation and acceptance status, x86_64 accepted versus arm64 build-only.
- `docs/desktop.md`: NSIS `currentUser` layout, `%LOCALAPPDATA%\Own Domains`, state directory, setup command, and GUI behavior.
- `docs/cli.md`: Windows `setup --apply`, `uninstall --apply`, `doctor`, named-pipe behavior, and the UAC boundary.
- `docs/setup.md`: Windows prerequisites, ports, NRPT, firewall profiles, CA trust, and the exact commands.
- `docs/troubleshooting.md`: the diagnostics table, port conflicts, excluded ranges, Public profile, NRPT conflicts, and trust failures.
- `docs/windows-acceptance.md`: gates, commands, outputs, environment versions, and pass/fail evidence.

Documentation must state:

- setup and uninstall are the only elevated commands;
- address changes, DHCP renewals, Ethernet/Wi-Fi switches, and new physical adapters do not require setup or UAC;
- a new address on a virtual, VPN, or tunnel adapter is rejected;
- ports 53, 80, and 443 must be free and are machine-wide; 4781 must be free on loopback;
- Hyper-V/WSL/Docker excluded ranges can block setup;
- the Public network profile blocks LAN `.own`;
- `.own` fails while the runtime is stopped and may have a cache delay;
- Firefox and Java may need manual CA import;
- setup may show a root-certificate confirmation dialog in addition to UAC;
- HTTP/3 is disabled;
- no service, helper, portproxy, or wildcard listener is installed;
- the exact diagnostics and remedies;
- the arm64 support status.

## Implementation order

1. Land the behavior-preserving transport extraction: endpoint abstraction, JSON-line codec, client/desktop/CLI plumbing, and Unix tests unchanged.
2. Add the Windows named-pipe server and client, ACL and identity checks, cancellation, and protocol fixtures. Run Gate C on a real Windows host.
3. Add Windows sidecar targets, build manifest entries, ZIP fetch/verify, NSIS resource layout, and packaging checks.
4. Build a disposable feasibility harness for Gates A, B, D, and F: standard-user bind on 53/80/443, NRPT/TTL/stop behavior, Job Object supervision, and Caddy AF_UNIX/HTTP3 behavior. Run it on Windows 10 22H2 x64 and Windows 11 x64. Stop and revise the design if any gate fails. The harness must not ship or install host integration.
5. Add Windows host artifact rendering: CoreDNS TTL 0, Caddy ports 80/443, HTTP/3 disabled, AF_UNIX admin with live `/load` (disabled-admin mode is not a shippable fallback), manifest, NRPT, firewall, and trust.
6. Add the Windows inspector, setup/uninstall transaction, elevation exchange, backups, rollback, and diagnostics.
7. Add `WindowsRuntimeBackend`, Job Object supervision, process identity, Caddy admin live reload with previous-config rollback, setup server, and readiness probes.
8. Run Gates E, G, H, and I on disposable Windows 10 22H2 x64 and Windows 11 x64 hosts.
9. Run Gate J (physical phone) and Gate K (packaging).
10. Record acceptance evidence, update the support matrix, and mark x86_64 supported. Leave arm64 build-only until Gate L passes.

No Windows host behavior may be mixed into the transport extraction commit series. Phase 1 must land and pass before Phase 2 begins.

## Out of scope

- A privileged helper daemon, Windows service, scheduled task, privileged named pipe, privileged TCP listener, or auto-elevating GUI action.
- `netsh interface portproxy`, WinNAT, ICS, WinDivert, a kernel driver, or any privileged forwarding layer.
- IPv6 LAN support.
- Windows Server, Windows 10 versions before 22H2, Windows Sandbox, WSL guests, Hyper-V guests, or containers as supported hosts.
- arm64 runtime support before acceptance.
- MSI, Windows Store/MSIX, or enterprise Group Policy deployment.
- Auto-start at login or boot.
- Concurrent multi-user or multi-installation operation.
- Automatic modification of VPN, third-party firewall, DNS-over-HTTPS, network profile, Group Policy, browser trust stores, or Java trust stores.
- A custom NRPT DNS port.
- A GUI button that runs setup or uninstall.
- Changing the IPC protocol, settings schema, mapping semantics, or reserved `setup.own` behavior.
- Sidecar auto-update or runtime download of sidecars.
- Windows Defender/EDR exclusions.
- Log rotation and retention policy beyond the existing behavior.

## Source constraints

- Named pipes, byte mode, security descriptors, `PIPE_REJECT_REMOTE_CLIENTS`, and `FILE_FLAG_FIRST_PIPE_INSTANCE`: https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipes, https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-createnamedpipew, and https://learn.microsoft.com/en-us/windows/win32/ipc/pipe-security
- Named-pipe client/server process identity: https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-getnamedpipeserverprocessid and https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-getnamedpipeclientprocessid
- Job Objects, kill-on-close, and process assignment: https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects and https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_extended_limit_information
- `PROC_THREAD_ATTRIBUTE_JOB_LIST` and process creation attributes: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute
- `CREATE_NO_WINDOW` and process creation flags: https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags
- NRPT cmdlets: https://learn.microsoft.com/en-us/powershell/module/dnsclient/add-dnsclientnrptrule, https://learn.microsoft.com/en-us/powershell/module/dnsclient/get-dnsclientnrptrule, https://learn.microsoft.com/en-us/powershell/module/dnsclient/set-dnsclientnrptrule, https://learn.microsoft.com/en-us/powershell/module/dnsclient/remove-dnsclientnrptrule, and https://learn.microsoft.com/en-us/powershell/module/dnsclient/get-dnsclientnrptpolicy
- Windows Firewall COM API and `LocalSubnet`: https://learn.microsoft.com/en-us/windows/win32/api/netfw/nn-netfw-inetfwpolicy2 and https://learn.microsoft.com/en-us/windows/win32/api/netfw/nn-netfw-inetfwrule
- `certutil` store commands: https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/certutil
- Adapter discovery and interface metadata: https://learn.microsoft.com/en-us/windows/win32/api/iphlpapi/nf-iphlpapi-getadaptersaddresses and https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-getifentry2
- Listener tables: https://learn.microsoft.com/en-us/windows/win32/api/iphlpapi/nf-iphlpapi-getextendedtcptable and https://learn.microsoft.com/en-us/windows/win32/api/iphlpapi/nf-iphlpapi-getextendedudptable
- Portproxy is TCP-only and uses an exact listen address: https://learn.microsoft.com/en-us/windows-server/networking/technologies/netsh/netsh-interface-portproxy
- Windows excluded port ranges and dynamic port behavior: https://learn.microsoft.com/en-us/troubleshoot/windows-server/networking/default-dynamic-port-range-tcpip-chang
- Windows AF_UNIX sockets: https://learn.microsoft.com/en-us/windows/win32/winsock/af-unix-socket-addresses
- `DnsQuery_W` and standard NRPT processing: https://learn.microsoft.com/en-us/windows/win32/api/windns/nf-windns-dnsquery_w
- Security descriptors and ACLs: https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-definition-language and https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-setnamedsecurityinfow
- Authenticode verification: https://learn.microsoft.com/en-us/windows/win32/seccrypto/winverifytrust
- Caddy admin API, AF_UNIX, and protocols: https://caddyserver.com/docs/api, https://caddyserver.com/docs/json/apps/http/servers/protocols/, and https://caddyserver.com/docs/caddyfile/options
- Tauri NSIS `currentUser` packaging and resources: https://v2.tauri.app/distribute/windows-installer/ and https://v2.tauri.app/reference/config/
- CoreDNS and Caddy release assets: https://github.com/coredns/coredns/releases and https://github.com/caddyserver/caddy/releases
