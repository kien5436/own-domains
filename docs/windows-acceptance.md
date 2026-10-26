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
- Dialog behavior on add/remove: observed by operator during the interactive run (see below).

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
