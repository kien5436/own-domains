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
  runtime reports unavailable as expected (no Windows backend yet).
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
