# Windows feasibility record (ARM64)

> **Scope:** feasibility only. Host is Windows 11 **ARM64** in UTM, not the x64 acceptance target.
> Nothing here claims x86_64 support. Results inform the design but do not pass any gate.

## Environment

- Guest: Windows 11 Home Single Language, build 26200, ARM 64-bit, UTM host-only network.
- Guest IP `192.168.128.2`, gateway unreachable from macOS host except via host-only `bridge100`.
- Accounts: `a` (administrator, SSH key auth), `odtest` (standard user, SSH key auth).
- No Hyper-V (Home edition has none), no WSL2/Docker installed, no portproxy rules.
- Disposable PowerShell DNS stub used for NRPT tests; removed after. No host integration installed.

## Gate A: low-port binding (partial)

Standard user `odtest` bound successfully, first try, no elevation:

- TCP `127.0.0.1:53`, `192.168.128.2:53`, `:80`, `:443` — OK.
- UDP `127.0.0.1:53`, `192.168.128.2:53` — OK.
- TCP `127.0.0.1:4781` — OK.

Preconditions observed:

- No listener on 53/80/443/4781 before the test.
- `netsh` excluded ranges: TCP only `5357-5357`; UDP none. No conflict with 53/80/443/4781.
- `netsh interface portproxy show all`: empty.

Still pending: reboot retest, WSL2/Docker-installed variants, x86_64 hardware.

## Gate B: NRPT and DNS (partial)

- `Add-DnsClientNrptRule -DisplayName OwnDomainsTest -Namespace .own -NameServers 127.0.0.1`
  returned GUID `{34FAE2F8-BF57-413E-9A56-BEB29D4B1543}`; effective policy showed `.own -> 127.0.0.1`.
- Direct UDP and TCP queries to `127.0.0.1:53` answered `setup.own -> 192.168.128.2`.
- System resolver (`Resolve-DnsName setup.own`, no `-Server`) returned `192.168.128.2`.
  NRPT routing works.
- `Get-DnsClientCache` showed no `setup.own` entry after the query: TTL 0 record was not cached.
- Stub stopped, system resolver queried twice:
  - Attempt 1 failed in ~287 ms (connection refused, nothing listening).
  - Attempt 2 failed in ~5 ms (negative cache).
  - Cache remained empty; no public-DNS answer leaked. Fail-closed holds for this case.
- Unrelated-name split path (e.g. `example.com` beside `.own`) was **not testable**: host-only
  networking has no upstream DNS, so the query timed out for lack of route, not policy.

Still pending: split-path proof with upstream DNS, GPO-conflict detection, secure-DNS-bypass
behavior, reboot persistence, x86_64 hardware.

## Gate G: trust add/remove (feasibility, interactive console)

Throwaway self-signed cert (`CN=OwnDomains Gate Test`, thumbprint `C4865404...FA2D`):

- `certutil -addstore -user Root` and `certutil -delstore -user Root <thumbprint>` both succeed
  from the interactive console logon. Removal verified: store no longer lists the cert.
- From a non-interactive SSH session both `certutil` and the PowerShell `X509Store` write fail
  with `0x80070032` / "The request is not supported", while reads work. Trust writes require
  the interactive user session — consistent with the design placing trust install/removal in
  the unprivileged interactive user phase, never in the elevated phase or a remote session.
- Dialog behavior: the operator confirmed a confirmation dialog appears on both add and
  remove in the interactive console. Combined with the UAC prompt, setup can show two dialogs,
  matching the accepted product decision (per-user trust kept; machine-wide trust rejected).

All harness artifacts removed (test cert in `My` and `Root`, `.cer` file, helper scripts).

## Gate F: Caddy admin and HTTP/3 (feasibility pass on ARM64)

Caddy 2.11.7 `windows_arm64` (zip SHA-512 matches the published `caddy_2.11.7_checksums.txt`
entry `5e232f5f...`) runs on this host. Disposable harness, since removed:

- Relative AF_UNIX admin `"listen": "unix/caddy-admin.sock"` with CWD set to the runtime dir:
  socket file created, no drive-letter parsing problem.
- Live config load works: `caddy reload --config caddyB.json --address unix/caddy-admin.sock`
  switched the served body from `config-a` to `config-b` with no restart.
- `"protocols": ["h1","h2"]` on the test server: `Get-NetUDPEndpoint -LocalPort 443` empty.
  No Caddy-owned UDP 443 listener.
- Stale socket restart: after `Stop-Process`, the dead `caddy-admin.sock` file remained and the
  next `caddy run` started normally and served traffic.
- Listeners were exactly `127.0.0.1:80` as configured; no wildcard.

The restart-fallback path was not exercised and remains rejected as a shipping mode per the
product decision. Still pending: same pass on x86_64 hardware.

## Cleanup

NRPT test rule removed by GUID; no `OwnDomains*` rules remain. Stub process stopped; all
harness scripts, log, and pidfile deleted from both profiles. Verified via re-listing rules
and the absence of `:53` listeners owned by the harness.

## Gate D: Job Objects (blocked on this host)

Probed with compiled C# harnesses (`csc.exe`, Framework64) plus a class/length sweep binary,
run as admin and standard user, over SSH and interactively. All harness files removed after.

What works on this host:

- `CreateJobObject` succeeds (named and unnamed).
- `PROC_THREAD_ATTRIBUTE_JOB_LIST` launch works: `IsProcessInJob` confirms the child is in
  the job before it runs user code.
- The suspend/assign/resume fallback path works: `AssignProcessToJobObject` succeeds.
- `CREATE_NO_WINDOW` launches show no window.

What fails on this host:

- Every `SetInformationJobObject` limit set fails with `ERROR_BAD_LENGTH` (24), including
  `BasicLimit` (class 2, 56 bytes) and `ExtendedLimit` (class 9, 136 bytes) with
  byte-exact buffers, from compiled code, in all four combinations of
  admin/standard x SSH/interactive.
- Behavioral proof: with the set failing, closing the job handle leaves the child tree
  (`cmd` + `ping`) alive — kill-on-close genuinely not applied, not a reporting artifact.
- Length validation itself works (classes 1/3/5/10/11 fail later on content with 87/50),
  so the rejection targets the limit-setting classes specifically.

Context findings relevant to the design:

- SSH session processes report `IsProcessInJob == True` (both accounts); the interactive
  console reports `False`. The daemon runs in the interactive user context, so the normal
  case is not nested — but any launcher running inside a job must hit
  `processSupervisionUnavailable`, as designed.
- No stray test processes or listeners remained after the sessions ended; harness files
  removed from both profiles and `C:\Users\Public`.

Status: Gate D is blocked on this host by an unexplained kernel rejection, not by the
design. The API is fundamental to Windows (browsers and services rely on it), so the
next validation is the same behavioral test on x86_64 hardware and, in parallel, the
Rust implementation using the `windows` crate. If x86_64 also rejects, the design's
supervision section must be rewritten.

## Gate C: Named pipe (transport passes on ARM64)

Proven with the Rust implementation (`feat/platform-transport-abstraction` up to
`83e2a6d`), on the same ARM64 VM as the other gates:

- `cargo test -p own-domains-client --test pipe`: 5/5 pass against a live server —
  first-instance rejection is `AddrInUse`, handshake + `Snapshot` roundtrip through
  the verified client, subscription streams a published event until cancel, `stop`
  waits for shutdown and releases the name (re-bind succeeds), and a missing pipe
  classifies as absence.
- Pipe-name derivation is deterministic per state directory and distinct across
  directories (`cargo test -p own-domains-protocol`).
- End to end with separate binaries: `own-domains.exe start/status/stop` against a
  real `own-domains-daemon.exe` over `--pipe`, with the full server-identity check
  (image path, SID, session) against the real daemon image — no errors, and the
  runtime reports unavailable as expected (no Windows backend yet at that time;
  since landed — see Backend milestone below).
- Two implementation bugs were caught by the tests, not by review: the client
  rejected in-process test servers (fixed by trusting a kernel-reported server PID
  equal to our own), and clients paired with the never-listened first handle while
  the accept loop waited elsewhere (fixed by serving the first client on the first
  handle).

DACL readback passes: the live pipe reports `O:<sid>G:<sid>D:P(A;;FA;;;<sid>)` —
protected, owner-only, no `Everyone` (`GA` requested, `FA` stored; the test accepts
both spellings). Still open for full Gate C: remote-client rejection (needs a second
machine) and Authenticode publisher pinning (lands with signed packaging). The cancel path documents a deliberate deviation: cross-thread
`CancelIoEx(handle, NULL)` only cancels the calling thread's I/O, so the waiter
slices at 50 ms and checks a shared flag instead.

## Backend milestone: WindowsRuntimeBackend (passes on ARM64 VM)

Same VM as the gates above; guest LAN observed as `192.168.1.21` during these
runs. Sidecars staged once in the user profile: CoreDNS 1.14.7
`windows_amd64` (no ARM64 upstream asset — 404 — so x64 under emulation;
published zip SHA-256 `6b718a5e…c6fdd56`) and Caddy 2.11.7 `windows_arm64`
(published zip SHA-512 `5e232f5f…a3fe982`).

Guest results, all green:

- `cargo test -p own-domains-daemon --test windows_admin` — 3/3: config
  load + readiness roundtrip over Winsock AF_UNIX with path assertions,
  500-status handling, relative-socket rejection.
- `cargo test -p own-domains-daemon --test windows_backend` — 2/2:
  `cmd /c exit 42` returns 42; the `ping` sleeper shows no console window,
  reports running, then terminates with a non-still-active code.
- `cargo test -p own-domains-daemon --test windows_backend_e2e` — full
  `apply` (render, write, spawn, setup server, DNS + HTTP readiness,
  admin live-load) then DNS answers `app.own -> 192.168.1.21`, Caddy serves
  the mapped app (`200`, body `ok`), the setup page serves `200` on
  `127.0.0.1:4781`, `stop` leaves ports 53/80/443/4781 closed — twice.
- `cargo test --workspace --locked` on the guest — green. One failure on the
  way was a test bug (desktop bridge test hardcoded the Unix `daemon.sock`
  default; Windows defaults to a named pipe by design), fixed and re-run.
- Manual: `own-domains-daemon.exe --pipe` + Ctrl+C exits promptly with no
  listeners left behind (operator-confirmed).

Bugs caught by the tests, not by review: the admin stub discarded
already-coalesced body bytes (mutual hang) and raced `listen`; the first
single-shot DNS probe met Windows `WSAECONNRESET` (10054) when the packet
landed before emulated CoreDNS bound (fixed by 30 s polling); a failed
`apply` leaked sidecars holding ports (now terminated on failure); the HTTP
probe targeted `setup.own`, which 502s without a setup server (now probes
the first plain-HTTP mapping and starts the setup server first).

Not claimed: the backend is not yet wired into the daemon `runtime_backend`
facade (the daemon still reports an unavailable runtime on Windows); the
`/ca.crt`-style setup routes await the trust milestone; TLS-only configs
skip HTTP readiness; supervision is plain spawn/terminate (Gate D still
blocked). Rendered listeners are loopback plus the LAN address only; the
observed Corefile binds `127.0.0.1` and the LAN address on port 53.

## x64 hardware acceptance (PTKIEN-TNN, Windows 11 build 26200, AMD64)

Host: physical x64 Windows 11 (build 26200), administrator account, Rust
1.99.1 + VS Build Tools installed for the run, WebView2 runtime 156 present,
LAN `10.23.34.20` (Ethernet; the `172.30.96.1` vEthernet is the WSL switch,
not the LAN). IIS Default Web Site initially held port 80; its `*:80:`
binding was removed for the run (IIS kept installed). All results below are
from real hardware; the ARM64 emulation notes above are superseded where
they conflict.

- `cargo test --workspace --locked` — 50 suites, 109 tests, 0 failures
  (includes the elevated e2e; env-gated staging via
  `OWN_DOMAINS_TEST_{COREDNS,CADDY}_EXE` + `OWN_DOMAINS_TEST_LAN`).
- `windows_backend_e2e` — full loop passes in ~22 s: DNS answers
  `app.own -> 10.23.34.20`, Caddy serves the mapped app (`200`, body `ok`),
  setup page `200` on `127.0.0.1:4781`, admin unreachable on the LAN
  address, `stop` leaves 53/80/443/4781 closed — twice.
- Elevated setup runbook — `RUNBOOK PASS`: preview changes nothing;
  apply installs exactly one NRPT rule (`.own -> 127.0.0.1`), one
  `OwnDomains-LAN` firewall rule, and `CN=OwnDomains Local CA` in per-user
  Root; `add` + `start` serves `runbook.own` over HTTP with system-resolver
  DNS; `stop` closes all ports; `uninstall --apply --purge` leaves zero
  NRPT rules, zero firewall rules, zero certs, no state dir.
- NSIS bundle `Own Domains_0.1.0-beta.3_x64-setup.exe` (81 MiB) builds,
  installs per-user, and the app launches (`Own Domains` window, daemon
  auto-spawns) with `--state-dir` (required by design; bare launch exits 2
  asking for it). No SmartScreen block on the local build.
- `pnpm test` on Windows: 115 pass; 29 fail, all pre-existing legacy
  TypeScript gaps unrelated to this release (Windows file modes
  `438 vs 493`, `install.sh` lifecycle scripts, dnsmasq/launchd/uid-501
  fixtures). Rust suites, lint, typecheck, clippy, and fmt are green.
- Gate D retest: Job Object create/set/assign/query/terminate all succeed
  on x64 — the ARM64 `ERROR_BAD_LENGTH` block was emulation-only
  (it also fires on x64 with a malformed 4-byte struct; the full 64-byte
  `JOBOBJECT_BASIC_LIMIT_INFORMATION` / 144-byte extended form works).
  Caveat: `KILL_ON_JOB_CLOSE` did not terminate the member on handle close
  in the interop shell (which itself runs inside a job); explicit
  `TerminateJobObject` works. Supervision stays out of scope for beta.3.

Bugs caught by the x64 run, not by review (all fixed on
`feat/windows-release` before the matrix re-ran green):

- Caddy on Windows dies silently (no log, no exit code) with a backslash
  `unix/` admin path — masked at first because IIS answered port 80 with
  `200` for any Host, so HTTP readiness passed spuriously. Admin is now
  loopback TCP (`127.0.0.1:2019`) per plan, with a port-carrying Host
  header (Caddy 403s bare `localhost`).
- The Rust AF_UNIX client passed the POSIX exact-length `addrlen`; WinSock
  requires the full 110-byte `SOCKADDR_UN` (was `WSAECONNREFUSED` 10061),
  and Winsock errors were reported via `GetLastError` (always 0) instead of
  `WSAGetLastError`.
- `setup --apply` never persisted the LAN address, so a later `start`
  failed validation; fixed to match the macOS persist step.
- The trust receipt stored `Export-Certificate`'s whole directory listing,
  so `certutil -delstore` exited 0 while deleting nothing — a silent CA
  leak on every uninstall. The receipt now holds just the thumbprint.
- The GUI/CLI-launched daemon never received sidecar locations; the
  Windows factory now auto-derives the installed bundle layout.
- Two stale gate tests: `--interface` optional off macOS, and the
  unsupported-platform test still listing Windows.

Not claimed: signed packaging, auto-updater, MSI, Linux, encrypted-DNS
coexistence, physical-phone checks against the x64 host, Intel macOS.

Reboot persistence (second host reboot, verified 2026-10-28): setup left
installed across the reboot — NRPT rule, firewall rule, per-user CA
cert, and settings all present; no autostart (by design). Manual `start`
serves traffic, `stop` closes all ports, `uninstall --apply --purge`
returns to zero. Two cold-boot findings fixed during the check: `start`
timed out at 30 s while the daemon kept applying in the background (CLI
budget raised to 120 s on both the connect and launch paths), and a
timed-out retry tore down the working sidecars (Windows backend is now
idempotent for the applied revision and stops existing sidecars before
a fresh apply, mirroring macOS; repeat `start` returns Applied and
traffic is uninterrupted).

Post-matrix packaging additions (verified on the same host): the NSIS
bundle now ships `own-domains.exe` beside the daemon (`externalBin`;
`stage-desktop.ts` stages both, all `desktop:build` variants compile
both). Bare `own-domains status` from any directory works against the
platform-default state dir, and the GUI falls back to the same default
instead of exiting 2 when launched without `--state-dir` — bare
double-click starts the app with its window and auto-spawns the daemon.
Explicit `--state-dir` and `OWN_DOMAINS_STATE_DIR` still override.
