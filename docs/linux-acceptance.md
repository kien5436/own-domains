# Linux acceptance record (Milestone 1: rendering + backend skeleton)

> **Scope:** artifact rendering and the runtime-backend skeleton only.
> Nothing here applies host changes: no systemd units enabled, no nftables
> table loaded, no resolver settings touched, no certificates installed, no
> processes started. Those land in Milestone 2.

## Environment

- Guest: Ubuntu 24.04 x86_64 on UTM. Verification ran natively in the
  guest; the cross-compile target check (`x86_64-unknown-linux-gnu`) on
  the host covers only compilation of the `cfg(target_os = "linux")`
  facade branch.
- Repo state: `main` at the Milestone 1 squash commit; no uncommitted
  changes during the runs.

## Verified

All suites run with `cargo test` in the guest (native target), plus the
host-side full workspace suite, both clippies (`-D warnings`), `fmt`,
and target checks for `x86_64-pc-windows-msvc`,
`x86_64-unknown-linux-gnu`, and `aarch64-unknown-linux-gnu` — all clean.

| Suite | Tests | What it proves |
| --- | --- | --- |
| `linux_config` | 3 | TTL 0, ports 5354/8080/8443, h1/h2, loopback+LAN listeners; macOS defaults unchanged (TTL 60, no protocols) |
| `linux_interfaces` | 3 | Allowlist/denylist with Docker `br-` + 12 hex checked before `br`, 15-byte cap, sorted unique output capped at 32 |
| `linux_ip` | 3 | `ip -j` fixtures produce `["bond0","br0","enp3s0","vlan100","wlp2s0"]` with primaries `None`/`10.0.0.5`/`192.168.1.20`/`10.0.0.100`/`None`; slaves and virtual interfaces excluded; secondary alias never selected |
| `linux_nftables` | 2 | Ruleset byte-exact against the checked-in fixture (dedicated `own_domains` table, prerouting + output chains, no LAN address, no filter chain) |
| `linux_systemd` | 2 | Both units byte-exact against fixtures copied verbatim from the design (oneshot, full hardening set, no substitution) |
| `linux_trust` | 2 | PEM and DER inputs share the fingerprint `15a63910…e013be6f`; malformed/empty/multi-block/upper-case inputs rejected |
| `linux_manifest` | 4 | Install manifest byte-exact (digests produced independently with `shasum -a 256`); parse round-trips; every invalid shape in the plan matrix rejected |
| `linux_platform` | 2 | Facade skeleton: `setupRequired` on an empty state dir, `setupManifestInvalid` on corrupt or foreign-platform manifests, silence on a valid one; inspect stopped, stop idempotent, apply unavailable; Linux-only facade-selection assertions execute on the guest |
| `sidecar_manifest` | 1 | Shared `SidecarManifest` contract both platforms build against |

Bugs caught by verification, not by review (all fixed on
`feat/linux-host-rendering` before the matrix re-ran green):

- The Linux facade called `linux::runtime_backend` without a re-export;
  invisible on every host build because the call site is cfg-gated out
  there, and only the `x86_64-unknown-linux-gnu` target check failed.
  The re-export now mirrors the Windows factory pattern.
- The NRPT remove command interpolated brace-wrapped rule names
  unquoted (Windows, same branch window).
- `validate_covered_interfaces` and `render_nftables` took `&[&str]`,
  which fails type inference on empty literals; both are generic over
  `AsRef<str>` with turbofish call sites in tests.

## Not claimed

Live `ip`, `nft`, `resolvectl`, `systemctl`, `update-ca-certificates`,
or `openssl` execution; setup/uninstall transactions; backup, rollback,
or trust installation; process supervision, listener inspection, or
sidecar loading; `.deb`/AppImage layout; sidecar fetch/verify on Linux;
the shared Unix extraction; the full runtime backend. Those are
Milestone 2, which lands on top of this skeleton.
