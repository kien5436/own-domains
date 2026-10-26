# Linux Host Integration Design

## Summary

Own Domains targets Ubuntu 24.04 LTS on x86_64 and arm64. Linux needs the same product boundary as macOS: a one-time privileged setup installs narrowly scoped host integration, and normal GUI launches, CLI start/stop/status, mapping edits, LAN address changes, and switching between covered interfaces never request root.

The Linux adapter uses:

- systemd-resolved with a dedicated dummy link and a route-only `~own` domain for split DNS;
- a dedicated `ip own_domains` nftables table for redirecting ports 53, 80, and 443 to the unprivileged CoreDNS and Caddy ports;
- `/usr/local/share/ca-certificates/own-domains.crt` plus `update-ca-certificates` for system trust;
- two static systemd oneshot units for boot persistence, not a privileged helper daemon or command channel.

The existing `SetupRequest`, IPC protocol, settings model, GUI, and platform-neutral runtime/installer/inspector contracts stay unchanged. Linux discovers and stores covered interface names, not the selected LAN address. Address and covered-interface changes remain unprivileged runtime work.

No item in this design has been executed on a real Ubuntu host. The verification gates in [Real Ubuntu verification gates](#real-ubuntu-verification-gates) must pass before Linux can be declared supported.

## Context

The macOS runtime establishes the host-integration model:

- a one-time privileged `setup --apply` writes resolver, packet-filter, and trust artifacts;
- the runtime runs as the signed-in user and supervises bundled CoreDNS and Caddy sidecars on high ports;
- the install manifest records owned artifacts, sidecar hashes, and covered interfaces but no selected LAN address;
- the runtime inspector gates start and reports actionable diagnostics;
- uninstall removes only exact owned artifacts and restores prior state.

Linux cannot reuse the macOS mechanisms. PF, `/etc/resolver`, the System keychain, and `ifconfig` are replaced by nftables, systemd-resolved, `update-ca-certificates`, and `ip`. Linux also has no equivalent of a single PF anchor loaded from `/etc/pf.conf`; it has independent nftables tables, systemd units, and network managers.

The portable boundary already exists. `crates/own-domains-daemon/src/platform/host.rs` defines `HostInspector`, `HostInstaller`, `SetupRequest`, and `UninstallRequest`. `crates/own-domains-daemon/src/platform/mod.rs` selects a runtime backend at compile time. `crates/own-domains-cli/src/platform/mod.rs` selects an installer. The CLI still contains two hardcoded `cfg!(target_os = "macos")` guards in `execute_setup` and `execute_uninstall`; those guards move behind the platform facade as part of this work.

The Linux runtime currently returns `UnavailableRuntimeBackend`, and the CLI uses `UnsupportedHostInstaller`. `process.rs` already has a Linux `/proc` identity implementation, but listener inspection and all runtime orchestration remain macOS-only.

## Platform boundaries

The shared application must not learn nftables, systemd-resolved, systemd unit names, `update-ca-certificates`, or Linux interface-discovery rules. Portable contracts stay platform-neutral:

- `RuntimeBackend` remains the daemon's apply/stop/inspect/diagnose contract.
- `HostInspector` remains the shared diagnostic contract.
- `HostInstaller` remains the shared setup/uninstall contract.
- `SetupRequest`, `UninstallRequest`, `SetupPreview`, `SetupApplyResult`, and `SetupFailure` remain shared.
- `SidecarManifest` moves out of the macOS namespace because both Unix platforms use the same runtime sidecar identity.
- Platform selection stays compile-time through `platform` and `cli::platform`.

The following remain platform-specific:

- macOS: PF anchor rendering and `/etc/pf.conf` composition, `/etc/resolver/own`, the `security` trust commands, `ifconfig` discovery, the schema 2 macOS manifest, and macOS target triples.
- Linux: nftables ruleset rendering, systemd unit contents, the resolved dummy link, `update-ca-certificates` and `openssl` trust checks, `ip` discovery, the Linux manifest, and Linux target triples.

The CLI must not branch on `target_os` inside shared command code. `host_installer()` returns a Linux, macOS, or unsupported implementation. The unsupported implementation already returns an actionable error, so the macOS-only guards are removed rather than replaced with Linux-specific guards.

## Goals

- Make `own-domains start` and GUI launch activate `.own` DNS, HTTP, HTTPS, and `setup.own` on Ubuntu 24.04 without root after setup.
- Make `own-domains stop` and tray Quit stop every owned process and listener without root.
- Keep CoreDNS and Caddy bound only to `127.0.0.1` plus the selected LAN IPv4 address. Do not introduce wildcard listeners.
- Route only `.own` through CoreDNS on the host; leave all other names on the existing resolver path.
- Redirect LAN traffic addressed to a covered interface's own address on ports 53, 80, and 443, and host-originated HTTP/HTTPS to local non-loopback addresses, to the unprivileged ports 5354, 8080, and 8443. The output chain is bounded by UID and loopback/link-local exclusions.
- Remove the LAN address from all persistent privileged artifacts. Address changes and switching between covered interfaces must not require root.
- Preserve mappings, settings schema version 1, CA identity, IPC protocol version, GUI behavior, and uninstall reversibility.
- Keep the no-helper boundary: no long-running privileged process, privileged socket, or privileged command channel.
- Make diagnostics actionable when the selected address is inactive, an interface is uncovered, a unit is inactive, or trust/resolver state has drifted.

## Non-goals

- Zero-root installation. Initial setup and uninstall still require `sudo`.
- A privileged helper daemon, root IPC socket, or polkit action for normal runtime operations.
- Automatic modification of NetworkManager connections, netplan, `/etc/resolv.conf`, Docker tables, UFW policy, firewalld policy, or browser trust stores.
- IPv6 LAN support. The settings model and sidecar bindings remain IPv4.
- Supporting resolvers other than systemd-resolved.
- Changing the IPC protocol, settings schema, mapping semantics, or reserved `setup.own` behavior.
- Supporting Ubuntu versions other than 24.04 LTS in this design.
- Replacing nftables with privileged-port listeners, socket activation, or file capabilities.
- A GUI button that runs privileged setup.

## Selected architecture

The privileged host integration is static except for the covered interface list:

| Artifact | Content | Changes after setup |
| --- | --- | --- |
| `/etc/own-domains/nftables.conf` | Dedicated `ip own_domains` table and redirect rules | Only when covered interfaces or rule shape change |
| `/etc/systemd/system/own-domains-nftables.service` | Oneshot unit that loads the table at boot | Only when the unit shape changes |
| `/etc/systemd/system/own-domains-resolver.service` | Oneshot unit that creates the dummy link and applies resolved settings at boot | Only when the unit shape changes |
| `/usr/local/share/ca-certificates/own-domains.crt` | Public Caddy root certificate | Only when the CA identity changes |
| `install-manifest.json` | Linux schema 1: owned artifact hashes, sidecars, covered interfaces, CA fingerprint | Only when setup is rerun |
| `settings.json` | Selected LAN address, upstream DNS, mappings | Freely changed by the GUI/CLI |

The runtime continues to run as the target user. It starts CoreDNS on `127.0.0.1:5354` and the selected LAN address on `5354`, Caddy HTTP on `127.0.0.1:8080` and the selected LAN address on `8080`, Caddy HTTPS on `127.0.0.1:8443` and the selected LAN address on `8443`, the setup server on `127.0.0.1:4781`, and the Caddy admin endpoint on an owner-only Unix socket in the state directory. The setup server is loopback-only; Caddy proxies `setup.own` to it over `127.0.0.1:4781`, so LAN clients reach the proxy on 8443 and never the setup server directly.

### DNS split behavior

The host resolver path is:

```text
application
  -> NSS / libc
  -> /etc/resolv.conf
  -> systemd-resolved stub at 127.0.0.53:53
  -> route-only domain ~own on link own-domains0
  -> 127.0.0.1:5354 (CoreDNS)
```

Setup creates a dummy link named `own-domains0` and applies per-link resolved settings:

- `DNS=127.0.0.1:5354`
- `Domains=~own`
- `DNSDefaultRoute=no`
- `DNSSEC=no`
- `DNSOverTLS=no`

The dummy link has no routable address, no default route, and no forwarding. It exists only to attach the route-only `~own` domain to a DNS server that is independent of the physical LAN interface. Queries for `own` and every subdomain go to CoreDNS. Queries for every other name continue to use the physical link's DNS servers. The dummy link cannot become the general resolver because `DNSDefaultRoute=no` is set and no other domain is attached to it.

This is deliberately not a global `DNS=` setting. A global `DNS=127.0.0.1:5354` would become a fallback for unrelated names and would break host DNS when the runtime is stopped. A per-physical-link setting would need root or a network-manager hook whenever the user switches between Wi-Fi and Ethernet. The dummy link is independent of both.

CoreDNS serves `.own` A records pointing at the selected LAN address and forwards other names upstream for LAN clients that use the host as DNS. The host's resolved sends only `.own` to CoreDNS, so host queries for unrelated names do not traverse CoreDNS.

When the runtime is stopped, resolved still routes `.own` to `127.0.0.1:5354`, where nothing listens. On the host, `.own` fails while all other names continue to resolve. This is the desired stop behavior and requires no root.

This stop behavior is host-only. A LAN client that uses the host as its only DNS server sends every query to CoreDNS, so stopping the runtime also stops that client's other DNS resolution. A client that keeps its normal resolver and uses the host only for `.own` needs split-DNS support of its own. The phone acceptance test records which mode was used.

When the runtime is running, the host's browser resolves `shop.own` to the LAN address. Host-originated HTTP and HTTPS to that address are redirected to the high ports by the output chain. The phone resolves `shop.own` through the host's LAN address and is redirected by the prerouting chain to the same LAN address on the high ports.

### nftables redirect shape

Setup writes one dedicated table. It never edits `/etc/nftables.conf`, Docker tables, UFW chains, firewalld zones, or NetworkManager profiles.

```nft
destroy table ip own_domains

table ip own_domains {
  chain prerouting {
    type nat hook prerouting priority -110; policy accept;

    iifname "enp3s0" fib daddr . iif type local ip daddr != 169.254.0.0/16 udp dport 53 redirect to :5354
    iifname "enp3s0" fib daddr . iif type local ip daddr != 169.254.0.0/16 tcp dport 53 redirect to :5354
    iifname "enp3s0" fib daddr . iif type local ip daddr != 169.254.0.0/16 tcp dport 80 redirect to :8080
    iifname "enp3s0" fib daddr . iif type local ip daddr != 169.254.0.0/16 tcp dport 443 redirect to :8443

    iifname "wlp2s0" fib daddr . iif type local ip daddr != 169.254.0.0/16 udp dport 53 redirect to :5354
    iifname "wlp2s0" fib daddr . iif type local ip daddr != 169.254.0.0/16 tcp dport 53 redirect to :5354
    iifname "wlp2s0" fib daddr . iif type local ip daddr != 169.254.0.0/16 tcp dport 80 redirect to :8080
    iifname "wlp2s0" fib daddr . iif type local ip daddr != 169.254.0.0/16 tcp dport 443 redirect to :8443
  }

  chain output {
    type nat hook output priority -100; policy accept;

    meta skuid 1000 fib daddr type local ip daddr != 127.0.0.0/8 ip daddr != 169.254.0.0/16 tcp dport 80 redirect to :8080
    meta skuid 1000 fib daddr type local ip daddr != 127.0.0.0/8 ip daddr != 169.254.0.0/16 tcp dport 443 redirect to :8443
  }
}
```

The rules repeat for every covered interface. The interface list is sorted, validated, and capped at 32 entries to bound the file size.

The `prerouting` chain runs at priority `-110`, before Docker's `dstnat` chain at `-100`. A packet arriving on a covered interface and addressed to that interface's own local address is redirected to the same interface's primary address on the high port. `fib daddr . iif type local` is dynamic: it follows DHCP changes and interface switches without reloading the table. `redirect to :port` uses the input interface's local address as the redirect target. Input rules exclude link-local destinations, matching the output chain; an addressless interface's link-local address is not redirected.

The `output` chain handles host-originated traffic to a local non-loopback address. It is scoped to the target user's UID and to local destinations, and it excludes loopback and link-local addresses. This avoids redirecting CoreDNS upstream queries or unrelated external traffic. It is broader than the input rule because nftables has no output-hook equivalent of "the destination address belongs to a covered interface"; the limitation is documented in [Known limitations](#known-limitations).

Only the selected interface is served at a time. CoreDNS and Caddy bind only the selected LAN address, so a redirect on another covered interface targets a high port with no listener. Those rules are prepared for an unprivileged switch; they do not make every covered interface simultaneously reachable.

No output redirect is added for port 53. The host resolver talks directly to `127.0.0.1:5354`; redirecting host DNS would add a loop and interception risk without serving the split-DNS design.

The table contains no filter chain. A NAT redirect does not bypass the host firewall: after translation, the input filter sees the high port. If UFW or firewalld is active with a default-deny policy, the high ports must be allowed. Setup detects this and reports the exact required rules; it does not modify the firewall. See [Docker, UFW, and firewall coexistence](#docker-ufw-and-firewall-coexistence).

The selected address must be the primary IPv4 address of the selected interface. `redirect to :port` targets the interface's primary address, so accepting an alias as the selected address would redirect clients to a different address than the one Caddy and CoreDNS bind to. Alias selection is rejected, matching the macOS behavior.

### Trust

Setup reuses the Caddy local CA generated in the target user's state directory. Only the public certificate is installed system-wide:

```text
/usr/local/share/ca-certificates/own-domains.crt
```

The file is root-owned with mode `0644`. Setup records the lowercase SHA-256 fingerprint of the DER certificate in the Linux manifest and runs:

```text
/usr/sbin/update-ca-certificates
```

The final probe runs:

```text
/usr/bin/openssl verify -CAfile /etc/ssl/certs/ca-certificates.crt \
  /usr/local/share/ca-certificates/own-domains.crt
```

The private key stays in the user's state directory under `caddy-data/pki/authorities/local/root.key`, mode `0600`, inside a `0700` directory. It is never copied to a system path and never served by the setup page.

Uninstall reads the installed certificate, computes the DER SHA-256 fingerprint, and compares it with the manifest before removing anything. A mismatch is a conflict: uninstall refuses to remove the file and reports `caFingerprintMismatch`. This prevents removing a certificate the user installed independently at the same path. After removal, uninstall runs `update-ca-certificates` again and verifies that the Own Domains certificate is no longer in the system bundle.

`update-ca-certificates` updates the system OpenSSL store. It does not update Firefox's NSS store or Chrome's local trust store. Users who browse `.own` in those browsers may need to import `ca.crt` from `setup.own`. Setup and doctor must not modify browser stores.

### Runtime and process supervision

The Linux runtime reuses the shared Unix orchestration described in [Shared Unix extraction](#shared-unix-extraction). It implements the same start, reconcile, stop, inspect, and diagnose sequence as macOS:

1. Load settings and require a valid LAN address.
2. Inspect host integration, sidecar identity, ports, ownership records, and surviving owned processes.
3. Stop verified stale Own Domains children. Never kill an ambiguous PID.
4. Stage and validate candidate CoreDNS and Caddy configurations.
5. Start the setup server, CoreDNS, and Caddy in order.
6. Wait for readiness: direct `.own` DNS over UDP and TCP, Caddy admin socket, setup endpoint, and HTTP routing.
7. Promote the candidate and persist child ownership records and the applied revision.

The runtime does not reload nftables, restart resolved, or change trust. Those are static host artifacts. A missing or inactive artifact is an error diagnostic that blocks start and points at setup or the unit.

Linux listener inspection uses `/usr/bin/ss -H -ltnup` (iproute2) for the required ports. The parser is bounded, ignores unrelated lines, and extracts the port and PID for owned children. `ss` is part of the base Ubuntu installation; setup verifies it exists. The existing `/proc` identity implementation is reused unchanged. If `ss` cannot see another user's PID, the port is still reported as occupied, which is sufficient for `portConflict`; owned children are always visible because they run as the target user.

## SetupRequest reuse

`SetupRequest` stays exactly as it is:

```text
state_directory
lan_address
interface
```

No platform field, covered-interface list, resolver flag, or trust flag is added.

The fields are initial selection inputs, not persistent privileged state:

- `state_directory` is the target user's state root.
- `lan_address` is the initial selected LAN IPv4 address.
- `interface` is the initial selected LAN interface.

Linux validates that `interface` is present, eligible, and has `lan_address` as its primary IPv4 address. It then discovers every eligible interface present at setup and stores only their names in the Linux manifest. The selected address is persisted in `settings.json` for the first runtime launch, exactly as macOS does. It is never written to the nftables table, the systemd units, the trust certificate, or the manifest.

This is the smallest contract that supports both platforms. Renaming `lan_address` to a generic `address` would break the shared contract and the macOS adapter for no benefit. Adding `covered_interfaces` would move platform discovery into the caller and let the GUI supply privileged rule inputs. Adding a `platform` field would duplicate compile-time selection. A future platform that needs a network namespace or interface index can add an optional request field behind the facade; this design does not.

`UninstallRequest` also stays unchanged: `state_directory` and `purge`.

## Covered interfaces and address changes

### Discovery

Setup discovers interfaces with fixed `/usr/sbin/ip` commands and no shell:

```text
/usr/sbin/ip -j link show
/usr/sbin/ip -j -4 addr show
```

A present eligible interface is a LAN interface whose name matches the allowlist and whose link metadata does not mark it as a bridge or bond slave:

- `en*`, `eth*` for Ethernet;
- `wl*`, `wlan*` for Wi-Fi;
- `br*` for bridges, except Docker's `br-` plus 12 hex digit names;
- `bond*` for bonds;
- `vlan*` for VLAN interfaces.

The exclusion list is applied before the allowlist. Loopback, Docker (`docker*` and `br-` plus 12 hex digits), veth, bridge or bond slaves (any link with a `master`), libvirt, VPN, tunnel, WireGuard, Tailscale, ZeroTier, Podman, CNI, LXC, LXD, Incus, VMware, VirtualBox, dummy, and other virtual interfaces are excluded. An interface with a `master` is represented by its bridge or bond master, never by the slave, so traffic is not matched twice. The allowlist is explicit; a new interface class requires a design update, not a silent wildcard.

VLAN subinterfaces named `<parent>.<id>` (for example `enp3s0.100`) are not covered unless they are renamed to `vlan*`; this is recorded as a limitation.

Names must be nonempty, at most 15 bytes, and contain only ASCII alphanumeric characters plus `.`, `_`, and `-`. This is stricter than the kernel and prevents nftables quoting or injection issues. Duplicate names are rejected.

The selected interface must be present and eligible. The selected address must be a global unicast IPv4 address and must equal the interface's primary IPv4 address. The primary address is the first non-secondary global IPv4 address reported by `ip -j -4 addr show`. Aliases, link-local addresses, loopback, multicast, and unspecified addresses are rejected.

Setup covers every eligible interface present at setup, including interfaces that currently have no IPv4 address. The nftables rule for an addressless interface matches nothing until the interface gains an address. This behavior requires real Ubuntu verification; if a rule does not begin matching when an address appears, addressless interfaces must be excluded and the limitation must be recorded.

A covered interface name remains in the manifest when the interface is absent, so a temporarily unplugged USB Ethernet or disconnected Wi-Fi adapter works again without setup. Manifest validation checks covered names against the allowlist and denylist but does not require the interface to be present. The inspector reports `lanAddressInactive` when the selected address is not active, and `interfaceCoverageMissing` when the selected address belongs to a present interface that is not covered, including an ineligible interface.

The covered-interface count is capped at 32. An empty eligible list is an error. Absent covered names are retained and count toward the cap; a host that accumulates more than 32 distinct interface names must uninstall and reinstall rather than editing the manifest.

### Address changes

The selected address remains unprivileged settings. When it changes:

1. The daemon saves the new value at the expected revision.
2. Runtime reconciliation rebuilds CoreDNS and Caddy with the new LAN address.
3. The nftables rules continue to reference the interface, not the address. `fib daddr . iif type local` and `redirect to :port` follow the new primary address without a reload.
4. The resolved dummy link is independent of the LAN address, so split DNS is unchanged.
5. Trust is independent of the address.

If the new address is active on a covered interface, reconciliation succeeds. If it is not active, the settings remain persisted, the runtime reports `lanAddressInactive`, and the previous applied configuration is retained where the existing reconciliation semantics allow it. The GUI remedy is to select an active LAN address, not to rerun setup.

An ordinary DHCP renewal that changes the primary address makes the saved address stale until the user saves the new one. The runtime does not silently rewrite settings. The inspector reports `lanAddressInactive` and the GUI can offer the new active address.

### Interface switches and new hardware

Switching between covered interfaces requires only a settings update. Both interfaces already have nftables rules; the rule for the newly selected interface now has a listener, and CoreDNS/Caddy rebind to its address. No root.

An interface that did not exist at setup is not covered. If the user selects its address, the inspector reports `interfaceCoverageMissing` and the remedy is one additional `setup --apply`. This is the only recurring-root exception. The same applies when an interface is renamed, because the nftables rule references the old name.

A covered interface that disappears leaves an inert rule. It becomes usable again when the interface returns. No setup is required.

## Linux install manifest

The Linux manifest is platform-specific and independent of the macOS schema 2 manifest. It is stored at the same relative path, `install-manifest.json`, and is owned by the target user with mode `0600`.

```json
{
  "schemaVersion": 1,
  "platform": "linux",
  "hostIntegrationVersion": 1,
  "userName": "alice",
  "userId": 1000,
  "backupDirectory": "/etc/own-domains/backups",
  "interfaces": ["enp3s0", "wlp2s0"],
  "artifacts": [
    {
      "path": "/etc/own-domains/nftables.conf",
      "sha256": "...",
      "mode": 420,
      "ownerId": 0,
      "groupId": 0,
      "kind": "nftables"
    },
    {
      "path": "/etc/systemd/system/own-domains-nftables.service",
      "sha256": "...",
      "mode": 420,
      "ownerId": 0,
      "groupId": 0,
      "kind": "systemdUnit"
    },
    {
      "path": "/etc/systemd/system/own-domains-resolver.service",
      "sha256": "...",
      "mode": 420,
      "ownerId": 0,
      "groupId": 0,
      "kind": "systemdUnit"
    },
    {
      "path": "/usr/local/share/ca-certificates/own-domains.crt",
      "sha256": "...",
      "mode": 420,
      "ownerId": 0,
      "groupId": 0,
      "kind": "trustCertificate"
    }
  ],
  "sidecars": {
    "schemaVersion": 1,
    "coredns": { "version": "1.14.7", "sha256": "..." },
    "caddy": { "version": "2.11.7", "sha256": "..." }
  },
  "caFingerprintSha256": "...",
  "nftablesTable": "ip own_domains",
  "resolverLink": "own-domains0",
  "systemdUnits": [
    "own-domains-nftables.service",
    "own-domains-resolver.service"
  ]
}
```

Validation rules:

- `schemaVersion` must be 1 and `platform` must be `linux`.
- `hostIntegrationVersion` must match the app build's expected version.
- `userName` and `userId` must match the target user; `userId` must be nonzero.
- `backupDirectory` must be exactly `/etc/own-domains/backups`, with no symlink in any component and a `root:root` `0700` directory.
- `interfaces` must be nonempty, unique, sorted, at most 32 entries, and valid eligible names under the allowlist and denylist. Presence is not required; a covered interface may be temporarily absent.
- `artifacts` must contain exactly the four expected paths, with the expected kinds, modes, owner `0`, group `0`, and lowercase 64-character SHA-256 digests. No extra paths are allowed.
- `sidecars` must match the packaged runtime sidecar manifest.
- `caFingerprintSha256` must be a lowercase 64-character hex digest.
- `nftablesTable` must be exactly `ip own_domains`; `resolverLink` must be exactly `own-domains0`.
- `systemdUnits` must contain exactly the two expected unit names.

The manifest contains no LAN address, interface address, UID of a network manager, or other mutable network state. It is a record of immutable installation facts. A tampered manifest cannot widen the uninstall path set because uninstall validates against the fixed allowlist, not only against the manifest's list.

## Setup changes

`own-domains setup` remains dry-run by default. `sudo own-domains setup --apply` is the only privileged setup path. The CLI keeps `--lan-address` and `--interface` as initial selection inputs.

### Preview

Preview performs read-only inspection and reports create, update, or unchanged actions for:

- `/etc/own-domains/nftables.conf`
- `/etc/systemd/system/own-domains-nftables.service`
- `/etc/systemd/system/own-domains-resolver.service`
- `/usr/local/share/ca-certificates/own-domains.crt`
- `install-manifest.json`

It also reports the planned unit enable/start actions, the nftables load, the trust update, and the final probes. Preview must not create the state directory, generate the CA, write files, enable units, load nftables rules, or change trust.

### Preflight

Setup rejects the operation before writing anything when any of these checks fail:

- The command is not running as root through `sudo`, or the target user cannot be resolved from `SUDO_USER`/`SUDO_UID`.
- The state directory is not absolute, contains `..`, is not owned by the target user when it already exists, is group- or world-writable, or resolves to a directory not owned by the target user. `$XDG_DATA_HOME` paths outside the home directory are allowed when these conditions hold. The CLI resolves a relative development path to an absolute path before invoking setup; setup never resolves a path relative to root's home.
- The selected interface is absent, ineligible, or its primary IPv4 address differs from `lan_address`.
- No eligible interface is present, or more than 32 are present.
- `systemd-resolved` is not active, `/etc/resolv.conf` does not point at the resolved stub, or resolved has a global `Domains=~own` or another link already claims `~own`. Setup does not enable or reconfigure resolved.
- Required fixed executables are missing: `/usr/sbin/ip`, `/usr/sbin/nft`, `/usr/bin/resolvectl`, `/usr/bin/systemctl`, `/usr/sbin/update-ca-certificates`, `/usr/bin/openssl`, `/usr/bin/ss`.
- The nftables kernel API is unavailable, or `nft -c -f` rejects the rendered ruleset.
- `/etc/own-domains` exists but is a symlink or is not owned by root, or a foreign `ip own_domains` table, `own-domains0` link, unit file, certificate file, or install manifest exists without being owned by this installation.
- A required port is occupied on a conflicting bind address. The check is address-aware: the resolved stub loopback listener on port 53 is expected; a wildcard listener, a loopback listener on 5354, 8080, or 8443, a listener on any IPv4 address of a covered interface on 53, 80, or 443, or a listener on any covered interface's primary IPv4 address on 5354, 8080, or 8443 is a conflict. A loopback or wildcard listener on 4781 is also a conflict. A port held by the same user's previous Own Domains runtime is reported and must be stopped first.
- Docker publishes 53, 80, 443, 5354, 8080, or 8443 on a covered interface, including through a wildcard publication that includes a covered interface.
- UFW or firewalld is active and the required high ports are not allowed. Setup reports the exact commands; it does not change the firewall.
- The sidecars are missing, not executable, have the wrong version, or fail the pinned SHA-256 digest.
- The CA private key exists without the certificate, or the certificate exists without the private key, and the state cannot be repaired as the target user. Existing CA files must be owned by the target user; the private key must not be group- or world-readable.
- The root-owned backup directory cannot be created, is a symlink, or is not `root:root` `0700`.

`nftables.service` with a `flush ruleset` file is a warning, not a rejection. The Own Domains unit loads after it at boot, but a later `systemctl reload nftables` can delete the table. Setup reports `nftablesServiceConflict` and does not edit `/etc/nftables.conf`.

### Apply

Setup performs these steps transactionally:

1. Validate the request and all preflight conditions.
2. Create or tighten the state directory as the target user, mode `0700`. Create `/etc/own-domains/backups` as `root:root` mode `0700`.
3. Generate the Caddy local CA as the target user when it is missing. The shared `ensure_caddy_ca` helper runs the packaged Caddy with `install_trust: false`, a temporary owner-only admin Unix socket, and the target user's `HOME`/XDG paths, then waits with a bounded timeout for `root.crt` and `root.key`. Reuse the existing CA when both files are present and owned by the target user.
4. Render the nftables ruleset, both systemd units, and the Linux manifest. Render the certificate artifact from the public CA certificate.
5. Inspect existing files. Reject unowned files. Back up owned files that will change.
6. Create `/etc/own-domains` if missing, `root:root` `0755`, and `/etc/own-domains/backups` `root:root` `0700`. Write the nftables configuration, units, certificate, and privileged backups as root with temporary-file, flush, and atomic-rename semantics under root-owned directories. Backups preserve the original owner and mode and are never writable by the target user. Drop to the target UID to write the manifest; never follow a symlink in a user-owned path. Root-owned artifacts use `root:root` and `0644`; the manifest uses the target user and `0600`.
7. Run `systemctl daemon-reload`.
8. Validate the nftables file with `nft -c -f`, then enable `own-domains-nftables.service` and start or restart it so the rendered ruleset is loaded.
9. Enable `own-domains-resolver.service` and start or restart it so the dummy link and resolved settings are applied.
10. Run `update-ca-certificates`.
11. Run final probes:
    - `nft list table ip own_domains` succeeds and contains both chains.
    - Both units are enabled and active.
    - `resolvectl dns own-domains0` reports `127.0.0.1:5354`.
    - `resolvectl domain own-domains0` reports `~own`.
    - `resolvectl default-route own-domains0` reports `no`.
    - `openssl verify` succeeds for the installed certificate.
    - The certificate fingerprint matches the manifest.
    - The sidecar digests still match.
12. Persist the selected LAN address in `settings.json` as the target user, without resetting mappings or upstream DNS. If this write fails, setup rolls back the host integration through the same journal and reports the failure; the previous settings are preserved.

The nftables unit is:

```ini
[Unit]
Description=Own Domains nftables redirect
After=network-pre.target nftables.service
Wants=network-pre.target
Before=network.target

[Service]
Type=oneshot
RemainAfterExit=yes
NoNewPrivileges=yes
PrivateTmp=yes
ProtectHome=yes
ProtectSystem=full
ProtectKernelTunables=yes
ProtectControlGroups=yes
CapabilityBoundingSet=CAP_NET_ADMIN
ExecStart=/usr/sbin/nft -f /etc/own-domains/nftables.conf
ExecStop=-/usr/sbin/nft delete table ip own_domains

[Install]
WantedBy=multi-user.target
```

The resolver unit is:

```ini
[Unit]
Description=Own Domains split DNS for .own
After=systemd-resolved.service
Wants=systemd-resolved.service
PartOf=systemd-resolved.service

[Service]
Type=oneshot
RemainAfterExit=yes
NoNewPrivileges=yes
PrivateTmp=yes
ProtectHome=yes
ProtectSystem=full
ProtectKernelTunables=yes
ProtectControlGroups=yes
CapabilityBoundingSet=CAP_NET_ADMIN
ExecStart=-/usr/sbin/ip link add own-domains0 type dummy
ExecStart=/usr/sbin/ip link set own-domains0 up
ExecStart=/usr/bin/resolvectl dns own-domains0 127.0.0.1:5354
ExecStart=/usr/bin/resolvectl domain own-domains0 "~own"
ExecStart=/usr/bin/resolvectl dnssec own-domains0 no
ExecStart=/usr/bin/resolvectl dnsovertls own-domains0 no
ExecStart=/usr/bin/resolvectl default-route own-domains0 no
ExecStop=-/usr/bin/resolvectl revert own-domains0
ExecStop=-/usr/sbin/ip link delete own-domains0

[Install]
WantedBy=multi-user.target
```

Both units use the same systemd hardening: no new privileges, private `/tmp`, no home access, read-only system paths, protected kernel tunables and cgroups, and only `CAP_NET_ADMIN`. Gate H verifies these directives do not block `nft`, `ip`, or `resolvectl`.

`PartOf=systemd-resolved.service` reapplies the dummy-link settings when resolved restarts. The unit contains only fixed commands and fixed arguments. It has no IPC, no socket, no command channel, and no user-controlled input.

The `nftables.service` ordering makes the Own Domains table load after an enabled distribution nftables unit at boot. A later `systemctl reload nftables` can still delete the table if that unit's file contains `flush ruleset`; doctor reports `nftablesServiceConflict` or `nftablesTableMissing`. Setup does not edit `/etc/nftables.conf`.

Setup is idempotent. Re-running with the same interface set, sidecars, and host integration version reports the artifacts unchanged and revalidates the units, table, resolved settings, and trust. Re-running after a new eligible interface appears adds it to the manifest and ruleset, creates a backup of the previous owned ruleset, and updates the manifest. Previously covered interfaces that are absent remain covered. Re-running after a sidecar or host integration version change updates the manifest and any changed artifacts. Existing files not owned by the installation remain conflicts.

### Rollback

The transaction journal records every file written and every backup created. Commands that mutate host state are rolled back in reverse order:

- disable and stop units that were newly enabled;
- delete the nftables table if it was newly loaded; if an existing owned ruleset changed, restore it and reload the nftables unit;
- revert and delete the dummy link if it was newly created;
- remove the certificate and run `update-ca-certificates` again if trust was newly installed; if a previously trusted certificate was replaced, restore it and run `update-ca-certificates` again;
- restore root-owned backed-up files after revalidating each backup's owner, mode, and SHA-256 against the journal;
- remove newly created files and directories that are empty;
- run `systemctl daemon-reload`, then restart any unit that was active before setup and whose file changed.

Backups are read only from the root-owned `/etc/own-domains/backups` directory. A backup with an unexpected owner, mode, or hash is an unresolved rollback item, not a file to restore.

If any rollback step fails, `SetupFailure.unresolved` lists the exact path, unit, table, or trust action. The command exits with code 4. The manifest is removed last, after the table, link, and trust rollback succeed; if any of those steps fail, the manifest remains so uninstall can clean up. A failed setup must not leave a partially trusted certificate or a table that shadows ports without the manifest to uninstall it.

## Uninstall changes

`own-domains uninstall` remains dry-run by default. `sudo own-domains uninstall --apply` is the only privileged uninstall path. `--purge` requires `--apply`.

Uninstall requires the runtime to be stopped. If the daemon socket is live or owned listeners remain, uninstall returns an actionable error instead of removing host integration under a running runtime. The user stops the runtime first through the GUI or `own-domains stop`.

Preview reads and validates the manifest, then lists, in reverse install order:

- stop and disable `own-domains-resolver.service`; revert and delete `own-domains0` if it still exists;
- stop and disable `own-domains-nftables.service`; delete `ip own_domains` if it still exists;
- remove the two unit files, the nftables configuration, and the root-owned backups, then `/etc/own-domains` if it is empty;
- run `systemctl daemon-reload` after the unit files are removed;
- remove the trust certificate after fingerprint verification and run `update-ca-certificates`;
- remove `install-manifest.json`;
- preserve or purge the state directory.

Apply performs the same steps. Before removing anything, it validates the manifest structure, every artifact path against the fixed allowlist, every artifact owner, mode, and hash, and the certificate fingerprint. An absent artifact is treated as already removed; a present artifact must match the manifest. A conflict leaves host state untouched. It then disables both units so their `multi-user.target.wants` symlinks are removed and runs `systemctl daemon-reload` after the unit files are removed. A path outside the allowlist, an unexpected owner or mode, a hash mismatch, or a remaining enablement symlink is a conflict. Uninstall removes only exact owned artifacts.

Linux does not modify a shared PF-style file. The dedicated table and unit files are either owned by the installation or rejected as foreign, so uninstall removes them and their root-owned backups rather than restoring an earlier Own Domains configuration. The root-owned backups are used only for setup rollback.

Uninstall is resumable and idempotent:

- It removes the manifest last, after all host artifacts are gone.
- If a step fails, it stops and reports the unresolved item; rerunning uninstall resumes from the manifest.
- If the manifest is missing or corrupt, uninstall refuses and reports the fixed paths for manual inspection. It does not guess or delete arbitrary files. The one exception is `--purge` after all fixed host artifacts are confirmed absent; it may then remove only the state directory.
- If the certificate fingerprint differs, it leaves the certificate and reports `caFingerprintMismatch`.
- If the system bundle still trusts the certificate after removal because the user installed another copy, it reports `caStillTrusted` and does not remove the other copy.
- Trust removal is idempotent. If the certificate file is already gone but the bundle still contains the manifest fingerprint, uninstall reruns `update-ca-certificates` and rechecks. If both are gone, the trust step is complete. If the file is present, uninstall verifies the fingerprint before removing it. A partial trust step therefore does not require restoring a deleted certificate or editing the bundle by hand.

`--purge` removes settings, mappings, logs, state-directory backups, and the Caddy CA after host integration is removed. The root-owned setup backups under `/etc/own-domains/backups` are removed by every successful uninstall, not only `--purge`. It refuses if trust removal is unresolved or the CA is still trusted. Without `--purge`, settings and the CA remain, but the CA is no longer trusted. A later setup reuses the same CA identity and trusts it again.

## Runtime and inspection changes

`LinuxRuntimeBackend` implements the existing `RuntimeBackend` contract using the shared Unix runtime. It does not add a new protocol command or GUI surface.

`HostIntegrationInspector` validates:

- the Linux manifest structure, platform, host integration version, ownership, and mode;
- the four artifact paths, kinds, modes, owners, groups, and SHA-256 hashes;
- the covered interface list and the selected settings address; covered names are validated against the allowlist and denylist without requiring the interface to be present;
- the nftables file content and, where permitted, the active table;
- both systemd units: present, enabled, and active;
- the resolved dummy link, DNS server, route-only domain, default-route setting, DNSSEC, and DNS-over-TLS settings;
- conflicts where another resolved link claims `~own`;
- the trust certificate file, fingerprint, and system bundle verification;
- sidecar presence, version, and digest;
- required port conflicts on loopback and every covered interface's primary IPv4 address, excluding the expected resolved stub loopback listener on port 53;
- Docker publications and active firewall policy that would block the high ports, where readable;
- the selected address is the primary IPv4 address of a covered interface.

The inspector runs as the target user. `nft list table ip own_domains` normally requires `CAP_NET_ADMIN`; when it is unavailable, the inspector reports a `nftablesUnverifiable` warning and relies on the manifest hash, unit state, and setup's recorded verification. This mirrors the macOS `pfAnchorUnverifiable` warning. The root-owned ruleset file and the unit state remain checkable without root.

Unprivileged inspection of host policy is best-effort. `nft list`, `ufw status`, and `docker ps` may require root, `CAP_NET_ADMIN`, or docker-group membership. When a source is unreadable, the inspector emits `nftablesUnverifiable`, `firewallUnverifiable`, or `dockerUnverifiable` and does not claim the policy is safe. Setup runs as root and performs the authoritative preflight. The physical-phone acceptance test is the final evidence for LAN reachability.

New diagnostics:

| Code | Meaning | Remedy |
| --- | --- | --- |
| `nftablesTableMissing` | The nftables configuration is missing or the active table is absent | Run privileged setup again |
| `nftablesRulesInvalid` | The active table differs from the owned ruleset when it can be inspected | Run privileged setup again |
| `nftablesUnverifiable` | The active table cannot be inspected without root (warning) | Check `sudo nft list table ip own_domains` if routing fails |
| `nftablesServiceConflict` | `nftables.service` is enabled with a `flush ruleset` file and can delete the table on reload (warning) | Remove the flush, disable `nftables.service`, or rerun setup after the reload |
| `firewallUnverifiable` | UFW or firewalld state cannot be read without root (warning) | Check `sudo ufw status` or `firewall-cmd --list-all`, or test from the LAN |
| `dockerUnverifiable` | Docker state cannot be read as the target user (warning) | Check `docker ps` as a Docker-group user or run setup again |
| `resolvedUnavailable` | systemd-resolved is not active or not the effective resolver | Enable systemd-resolved and retry |
| `resolvedLinkMissing` | `own-domains0` is missing or lacks the split-DNS settings | Restart the resolver unit or rerun setup |
| `resolvedDnsMismatch` | The link does not use `127.0.0.1:5354` | Restart the resolver unit or rerun setup |
| `resolvedDomainMismatch` | The link does not own the route-only `~own` domain | Restart the resolver unit or rerun setup |
| `resolvedDomainConflict` | Another link claims `~own` | Remove the conflicting split-DNS configuration |
| `systemdUnitMissing` | A required unit file is missing | Run privileged setup again |
| `systemdUnitInactive` | A required unit is not active | Start the unit or rerun setup |
| `systemdUnitDisabled` | A required unit is not enabled at boot | Enable the unit or rerun setup |
| `caFingerprintMismatch` | The installed certificate does not match the manifest | Restore the Own Domains certificate or rerun setup |
| `caStoreMismatch` | The installed certificate matches the manifest but is not in the system bundle | Run `update-ca-certificates` or rerun setup |
| `caStillTrusted` | The certificate remains trusted after removal | Remove the other trusted copy manually |
| `dockerPortConflict` | Docker publishes a port Own Domains needs | Stop or republish the container |
| `firewallConflict` | UFW or firewalld blocks the translated high ports | Allow 5354/tcp, 5354/udp, 8080/tcp, and 8443/tcp, or rerun setup after changing policy |

Existing codes keep their meaning: `setupRequired`, `setupManifestInvalid`, `sidecarMissing`, `sidecarDigestMismatch`, `sidecarVersionMismatch`, `caNotTrusted`, `portConflict`, `lanAddressInactive`, `interfaceCoverageMissing`, `inspectionFailed`, and `runtimeUnavailable`.

Diagnostic precedence:

- A missing, corrupt, or incompatible manifest produces `setupRequired` or `setupManifestInvalid` and stops further artifact checks, as on macOS.
- An artifact hash mismatch produces `setupRequired` with the failing path. `nftablesRulesInvalid` is reserved for an active table that differs from the owned ruleset when it can be inspected.
- Unit and resolved failures are separate diagnostics, one per failed condition, never duplicated.
- Trust inspection emits at most one of `caFingerprintMismatch`, `caStoreMismatch`, or `caNotTrusted`. `caStoreMismatch` means the installed certificate matches the manifest but is absent from the system bundle; `caNotTrusted` is the shared fallback when trust cannot be verified for another reason.
- `interfaceCoverageMissing` wins when the selected address belongs to a present interface that is not covered, including an ineligible interface. Otherwise an inactive address produces `lanAddressInactive`.
- Docker and firewall conflicts are errors when they would make `.own` unreachable and warnings when they are latent. Unverifiable nftables, firewall, or Docker state is a warning, never a claim of safety; it does not block start by itself.
- A covered interface that is absent is not a manifest error and does not by itself block start. The selected address must still be active on a covered interface.
- A stopped runtime may save an address that is not active. Start fails with `lanAddressInactive` or `interfaceCoverageMissing` until the address is active on a covered interface.

The GUI displays the diagnostic message and remedy. It must not instruct the user to rerun setup for an ordinary address change or an interface switch among covered interfaces.

## Shared Unix extraction

The Linux adapter must not copy the macOS runtime wholesale. The following behavior-preserving extraction happens first, with the macOS tests passing unchanged:

- `platform/unix/command.rs`: `HostCommand`, `HostCommandOutput`, `CommandRunner`, `SystemCommandRunner` with bounded output and timeouts.
- `platform/unix/files.rs`: `HostFileKind`, `HostFileMetadata`, `HostFileSystem`, `SystemHostFileSystem`, `SetupFile`, atomic write, chown, remove-file, remove-tree, and symlink-safe open helpers for user-owned paths.
- `platform/unix/process.rs`: `ProcessComponent`, `OwnedProcessRecord`, `ProcessIdentity`, `ListenerBinding`, `SpawnSpec`, `ChildProcess`, `ProcessHost`, `ProcessRecords`, `FileProcessRecords`, `ProcessSupervisor`, `RetryPolicy`, `Waiter`, `SystemWaiter`, `SystemProcessHost`, `CaddyAdminClient`, `DnsReadinessProbe`, `HttpReadinessProbe`, `canonical_program`, `system_spawn`, `system_signal_group`, and the platform-specific `system_identity` and `system_listeners` implementations.
- `platform/unix/config.rs`: `ConfigurationStager`, `ConfigurationValidator`, `ConfigurationFiles`, `SystemConfigurationFiles`, `SystemConfigurationValidator`, `CandidateConfiguration`, `ConfigurationError`, and `DEFAULT_DNS_VALIDATION_PORT`.
- `platform/unix/runtime.rs`: the start/reconcile/stop/inspect orchestration, parameterized by a platform inspector, runtime paths, setup server, and readiness implementation. `MacOsRuntimeBackend` and `LinuxRuntimeBackend` become thin platform wrappers.
- `platform/unix/setup.rs`: the transaction journal, backup naming, atomic file planning, preview/apply result construction, generic rollback, and the `ensure_caddy_ca` helper moved from the macOS CLI adapter. Platform hooks supply trust verification, unit/table actions, and the final probe.
- `platform/unix/paths.rs`: the common state layout (`runtime/`, `runtime/candidate`, `runtime/applied`, `runtime/daemon.sock`, `logs/`, `caddy-data/`, `caddy-config/`, `install-manifest.json`), a parameterized backup directory (macOS uses `backups/setup/` under the state directory; Linux uses `/etc/own-domains/backups`), target triple, and platform-specific binary names.
- `platform/sidecar.rs`: the shared `SidecarManifest` and `SidecarSpec` types.

The following stay in `platform/macos`:

- PF anchor rendering and `/etc/pf.conf` managed-block merging.
- `/etc/resolver/own`.
- `security add-trusted-cert`, `delete-certificate`, and `verify-cert`.
- `ifconfig` discovery and macOS interface eligibility.
- The schema 2 macOS manifest and its migration.
- macOS target triples and resource paths.

The following stay in `platform/linux`:

- nftables rendering and the `own_domains` table shape.
- systemd unit contents and unit names.
- `ip` discovery and Linux interface eligibility.
- `resolvectl` dummy-link management.
- `update-ca-certificates` and `openssl verify`.
- The Linux schema 1 manifest.
- Linux target triples and resource paths.

The shared extraction is a separate commit series. It changes no host artifacts, paths, manifest fields, diagnostics, or runtime behavior on macOS. Linux host work starts only after the extraction lands and the macOS suite passes.

Coordination with the Windows design (`2026-10-26-windows-host-integration-design.md`): the endpoint/transport abstraction, the JSON-line codec move, and the shared `SidecarManifest`/`SidecarSpec` and read-only setup-server moves are done once, in the transport-first commit series. The Unix process/config/files/runtime/paths/setup extraction above builds on top of that series and must not duplicate it. Order: transport extraction first, then this Unix extraction, then the Linux and Windows adapters.

The platform facade gains:

```text
platform::runtime_backend
  macOS -> macos::runtime_backend
  Linux -> linux::runtime_backend
  other -> UnavailableRuntimeBackend

cli::platform::host_installer
  macOS -> MacOsHostInstaller
  Linux -> LinuxHostInstaller
  other -> UnsupportedHostInstaller
```

The two `cfg!(target_os = "macos")` guards in `execute_setup` and `execute_uninstall` are removed. The unsupported installer already returns an actionable failure, so unsupported platforms still fail cleanly without a shared platform branch.

## Sidecars and packaging

Linux uses the same pinned CoreDNS 1.14.7 and Caddy 2.11.7 versions as macOS, with Linux release assets for both architectures:

| Component | amd64 target | arm64 target | Asset |
| --- | --- | --- | --- |
| CoreDNS | `x86_64-unknown-linux-gnu` | `aarch64-unknown-linux-gnu` | `coredns_1.14.7_linux_{amd64,arm64}.tgz` |
| Caddy | `x86_64-unknown-linux-gnu` | `aarch64-unknown-linux-gnu` | `caddy_2.11.7_linux_{amd64,arm64}.tar.gz` |

The build manifest gains an additive `linuxReleases` map under each sidecar:

```json
"linuxReleases": {
  "amd64": {
    "target": "x86_64-unknown-linux-gnu",
    "url": "...",
    "archiveMember": "coredns",
    "checksum": { "algorithm": "sha256", "url": "...", "value": "..." },
    "executableSha256": "..."
  },
  "arm64": { "...": "..." }
}
```

The macOS `releases` map is unchanged. The fetch and verification scripts gain a platform dimension:

- `--platform darwin|linux`, defaulting to the host platform.
- Platform-aware target triples.
- The same archive URL allowlist, redirect limit, size limits, checksum verification, and Caddy checksum-signature verification as macOS.
- Runtime manifest generation per target: `manifest-<target>.json` with the same `schemaVersion: 1` runtime shape.

The Linux executable hashes are pinned from the published release checksums and the extracted ELF binaries. macOS hashes are never reused for Linux. The implementation must download and verify the actual assets; this design does not invent hashes.

Runtime sidecar paths are fixed and never searched on `PATH`:

```text
<resource-directory>/sidecars/coredns-<target>
<resource-directory>/sidecars/caddy-<target>
<resource-directory>/sidecars/manifest-<target>.json
```

Packaging layout:

- `.deb`: `/usr/bin/own-domains`, `/usr/bin/own-domains-daemon`, and `/usr/lib/own-domains/sidecars/` containing the sidecar binaries and manifests for the package's target architecture only. Files are root-owned and not writable by the user. Both binaries resolve the resource directory as `/usr/lib/own-domains` (equivalently `../lib/own-domains` from `/usr/bin`) after canonicalizing their executable path.
- AppImage: the same relative layout under `<AppDir>/usr/lib/own-domains/` for the AppImage's target architecture. The binaries resolve the resource directory from the mounted AppImage root, not the user's current directory. AppImage integrity relies on the downloaded artifact's checksum or signature; the app still verifies the sidecar SHA-256 before execution.
- `OWN_DOMAINS_RESOURCES` may override the resource directory for unprivileged development and tests; it must be an absolute path containing `sidecars/`. Privileged `setup --apply` ignores the override unless the resolved directory is root-owned and not group- or world-writable.
- The `.deb` is the only in-scope Linux package for privileged setup. Product scope decision (2026-10-26): AppImage is deferred and out of scope for the first Linux support claim. An AppImage is user-writable; running it with `sudo` is outside the trust boundary unless the user first copies it to a root-owned path.
- The desktop app passes the absolute state directory (`$XDG_DATA_HOME/own-domains`, normally `~/.local/share/own-domains`) to the daemon and CLI. The CLI's development default `.own-domains` remains for local runs; the CLI canonicalizes it to an absolute path under the target user's home before invoking setup. Setup rejects relative paths and never resolves a path relative to root's home.

The setup and runtime verify the sidecar digest before execution. The process supervisor verifies the canonical executable path, arguments, PID, process group, and start identity before sending signals to a recovered child. A sidecar update that changes the pinned version or digest requires one privileged setup to update the manifest; the runtime never downloads or updates sidecars.

arm64 is build-only until accepted. The release check must fetch, verify, and package both Linux targets, but must not execute arm64 binaries on an x86_64 host. The platform support matrix must not claim arm64 runtime support until a real arm64 acceptance record exists.

## Persistence and upgrades

### Reboot

At boot, systemd loads `own-domains-nftables.service` before `network.target` and starts `own-domains-resolver.service` after `systemd-resolved.service`. The nftables table and dummy link are recreated. The runtime itself does not auto-start; the user launches the GUI or runs `own-domains start`.

If `nftables.service` is enabled and its `/etc/nftables.conf` contains `flush ruleset`, the Own Domains unit loads after it at boot. A later `systemctl reload nftables` can delete the Own Domains table; doctor reports `nftablesServiceConflict` or `nftablesTableMissing`. Setup does not edit the distribution file.

After reboot and before the runtime starts, host `.own` queries route to a closed `127.0.0.1:5354` and fail while all other host names resolve normally. LAN clients' ports 53, 80, and 443 on covered interfaces are redirected to closed high ports, so `.own` fails and no wildcard service is exposed. Other services on those ports remain shadowed while host integration is installed; uninstall is required to release them.

### systemd-resolved restart

`PartOf=systemd-resolved.service` restarts the resolver unit when resolved restarts. The unit reverts and deletes the old dummy link, then recreates it and reapplies the DNS/domain settings. The final state is revalidated by doctor.

If a NetworkManager restart clears resolved's per-link state without restarting resolved, the resolver unit is not automatically restarted. Doctor reports `resolvedLinkMissing` or `resolvedDnsMismatch`, and the remedy is `systemctl restart own-domains-resolver.service` or a setup rerun. This is a real Ubuntu verification item. If it proves common, the design may add a narrowly scoped NetworkManager dispatcher, but that is a new privileged hook and requires a separate decision.

### App upgrade

Host artifacts are root-owned files outside the app package. A normal app upgrade does not remove or rewrite them.

- If the sidecar versions, host integration version, and artifact shapes are unchanged, no setup is required. The new app validates the existing manifest and starts normally.
- If the sidecars or host integration version change, the inspector reports `setupRequired`. The user runs `setup --apply` once to update the manifest and any changed host artifacts.
- If the package layout changes the resource directory, the app must still resolve the sidecars from the packaged manifest; a missing sidecar is `sidecarMissing`, not a reason to search `PATH`.
- Uninstall uses the manifest written by the last successful setup, not the current app's defaults, except for the fixed allowlist validation.

### OS upgrade

This design targets Ubuntu 24.04 LTS. A later Ubuntu release may change systemd-resolved, nftables, NetworkManager, or `update-ca-certificates` behavior. The inspector detects drift, but support is not claimed until the acceptance suite is rerun. A setup rerun may be required after a major OS upgrade.

## Docker, UFW, and firewall coexistence

Own Domains uses its own `ip own_domains` table and does not edit Docker, UFW, firewalld, netplan, or NetworkManager configuration.

### Docker

Docker's NAT chains normally run at priority `dstnat` (`-100`). The Own Domains prerouting chain runs at `-110`, so it sees covered-interface traffic first and cannot be silently shadowed by a published container port.

Setup and doctor, where Docker state is readable, detect Docker publications on 53, 80, 443, 5354, 8080, and 8443. If a container publishes one of those ports on a covered interface, including through a wildcard publication, the conflict is an error: the user must stop or republish the container. Own Domains does not modify Docker tables or `DOCKER-USER`. If Docker is installed but does not publish those ports, the two stacks coexist; the dedicated table does not affect container traffic on other ports.

Docker's forwarded traffic is not input traffic and does not traverse the filter input hook. It does traverse the prerouting hook, which is why a conflicting publication on a covered interface is a preflight error. Containers that resolve `.own` through the host's LAN address may work through the host's DNS; containers with their own DNS configuration may not. This is documented, not auto-configured.

### UFW

A NAT redirect does not bypass UFW. After translation, UFW's input filter sees the high ports 5354, 8080, and 8443, not the original 53, 80, and 443. If UFW is active with a default-deny incoming policy, the high ports must be allowed.

Setup detects an active UFW and checks whether the required traffic is allowed. It does not run `ufw allow`. The remedy is a one-time, port-based policy change that does not need to change when the LAN address or covered interface changes:

```text
sudo ufw allow 5354/tcp
sudo ufw allow 5354/udp
sudo ufw allow 8080/tcp
sudo ufw allow 8443/tcp
```

Port-based rules are broader than the service bindings, but CoreDNS and Caddy bind only to loopback and the selected LAN address, so no listener appears on other interfaces. Interface-specific UFW rules would need to change when the user switches between Wi-Fi and Ethernet, which would reintroduce root prompts.

If the user's UFW policy uses a nonstandard chain priority or a deny rule before the allow, the acceptance test must catch it. The design does not add a filter accept chain that claims to override UFW; a separate nftables base chain cannot reliably override another table's drop.

### firewalld and iptables

firewalld is not the Ubuntu default but may be installed. Setup detects it and reports the same high-port requirement. It does not modify zones or policies.

Ubuntu 24.04 uses the nftables backend for iptables. Docker and UFW rules are visible in the nftables ruleset. A system using legacy iptables is outside this design's acceptance scope; the nftables table still exists at the kernel level, but coexistence with legacy rules must be verified before support is claimed.

### nftables.service

Ubuntu's optional `nftables.service` loads `/etc/nftables.conf` at boot and on reload. The Own Domains unit orders after it, so a boot load survives an enabled unit. A file containing `flush ruleset` still deletes the Own Domains table when the distribution unit is reloaded later. Setup detects this combination and reports `nftablesServiceConflict`; it does not edit `/etc/nftables.conf`, disable the unit, or add a competing flush.

### Unprivileged detection

Setup runs as root and performs the authoritative Docker, UFW, and firewalld checks. Doctor runs as the target user and may be unable to read `nft list`, `ufw status`, or `docker ps`. In that case it reports `nftablesUnverifiable`, `firewallUnverifiable`, or `dockerUnverifiable` and does not claim the policy is safe. A port that is occupied is still visible through `ss` and is reported as `portConflict`; Docker attribution requires readable Docker state.

## Security boundaries

- Only `sudo own-domains setup --apply` and `sudo own-domains uninstall --apply` run as root. Start, stop, status, mapping edits, address changes, interface switches, doctor, and GUI launch run as the target user.
- There is no privileged helper daemon, root Unix socket, root TCP listener, polkit action, or privileged command channel.
- The two systemd units are static oneshot units. They run fixed commands with fixed arguments at boot or on resolved restart, then exit. They do not read app input and cannot be commanded by the GUI or CLI.
- Setup accepts only a state directory, a parsed IPv4 address, and a validated interface name. It resolves the target user from `SUDO_UID`, uses fixed paths and command arrays, and never invokes a shell. Interface names and the UID are validated before rendering.
- Setup resolves the state directory and requires the final directory and existing user-owned files to be owned by the target user and not group/world-writable. It opens the manifest and settings files with no-symlink semantics after dropping to the target UID. Root never writes a user-owned file as root.
- Root-owned directories are `root:root` `0755`, except `/etc/own-domains/backups`, which is `root:root` `0700`; root-owned artifacts are `root:root` `0644`. The install manifest and settings are written as the target user with mode `0600`; the state directory is `0700`. Privileged backups preserve the original owner and mode and are readable only by root. Root drops to the target UID before writing user-owned files.
- CA generation runs the packaged Caddy as the target user with `install_trust: false` and a temporary owner-only admin socket. No root-owned CA material is created.
- The nftables table is dedicated. It does not edit `/etc/nftables.conf`, Docker tables, UFW chains, firewalld zones, or network-manager profiles.
- Input redirects are scoped to covered interfaces and to destinations that are local on that interface. They do not enable IP forwarding and do not add a forward-hook rule. A Docker publication on a covered interface is detected as a preflight conflict rather than left to shadow the container silently.
- Output redirects are scoped to the target UID, local destinations, non-loopback and non-link-local addresses, and ports 80 and 443. They do not intercept external traffic or DNS.
- CoreDNS and Caddy bind only to loopback and the selected LAN address. No wildcard listener is introduced.
- The resolved dummy link has no routable address, no default route, and no forwarding. `DNSDefaultRoute=no` prevents it from becoming the general resolver.
- Only the public CA certificate is installed system-wide. The private key stays in the owner-only state directory. Uninstall verifies the DER SHA-256 fingerprint before removing trust.
- The setup page remains read-only and exposes only the approved certificate and guidance. No management route, settings, logs, private keys, or IPC is exposed.
- Setup verifies sidecar digests before execution. The process supervisor verifies executable identity, arguments, PID, process group, and start identity before signaling a recovered process.
- Uninstall validates the manifest against a fixed artifact allowlist and removes only exact owned paths. A tampered manifest cannot widen the removal set.
- Setup rejects a foreign nftables table, dummy link, unit file, certificate, or manifest rather than overwriting it. It does not silently take ownership of another administrator's configuration.
- Doctor's unprivileged nftables, firewall, and Docker checks are best-effort. Unreadable policy is reported as unverifiable, not assumed safe.
- The CLI should be run from the packaged, root-owned binary. Setup should reject an `--apply` invocation whose executable is group- or world-writable when it can determine that safely, and ignores a resource-directory override unless it is root-owned and not group- or world-writable. Development runs may use an explicit override. This is a defense-in-depth check, not a replacement for package integrity. A user-writable AppImage run with `sudo` is outside the trust boundary.

## Diagnostics, failure, and rollback

The runtime gates start on any error diagnostic, except the existing owned-runtime `portConflict` exception. The Linux inspector therefore distinguishes:

- setup problems that require root: missing manifest, artifact hash mismatch, missing unit, inactive unit, missing table, untrusted certificate;
- unprivileged runtime problems: inactive selected address, uncovered interface, port conflict, sidecar drift, invalid candidate configuration, readiness failure;
- host-policy problems: Docker publication, UFW/firewalld policy, `nftables.service` reload conflict, conflicting resolved domain;
- unverifiable state: the active nftables table, UFW/firewalld policy, or Docker state cannot be read without root. These are warnings; setup and the LAN acceptance test remain authoritative.

A stopped runtime may save an address that is not active. Start fails before launching sidecars. A running runtime that receives an address change attempts reconciliation; if the candidate fails, the previous applied configuration remains authoritative and the runtime reports the failure. A failed candidate never deletes the last applied files.

Setup and uninstall failures return the shared `SetupFailure` shape:

- exit code 2 for invalid input or privilege entry;
- exit code 3 for an unsupported platform;
- exit code 4 for an apply/rollback failure.

`unresolved` lists every item that could not be restored or removed. The CLI prints the message and unresolved items. The GUI is not involved in privileged setup, so it shows the diagnostic and the exact command instead.

The daemon never attempts to repair host integration automatically. It cannot reload nftables, restart a system unit, change trust, or rewrite the manifest without root. The remedies are explicit: rerun setup, start a unit, select an active address, stop a conflicting container, or change the firewall policy.

## Testing and acceptance

Automated coverage must include:

### Linux rendering and validation

- exact nftables rendering for multiple interfaces, including per-interface rules, no LAN address interpolation, the target UID, the dedicated table, the input link-local exclusion, the output exclusions, and the interface cap;
- rejection of invalid, duplicate, oversized, loopback, Docker (`docker*`, `br-` plus 12 hex digits), VPN, bridge-slave, bond-slave, and otherwise ineligible interface names;
- acceptance of a bridge or bond master and rejection of its slaves, using `ip -j link show` `master` fixtures;
- primary-address selection from `ip -j` fixtures, including aliases and addressless interfaces;
- manifest validation accepting a covered name whose interface is currently absent;
- systemd unit rendering with fixed paths, no user input, and the expected ordering, `nftables.service` ordering, and `PartOf` directives;
- Linux manifest serialization, validation, allowlist enforcement, rejection of extra or unexpected paths, and rejection of a backup directory that is not exactly `/etc/own-domains/backups`, is symlinked, or is not `root:root` `0700`;
- CA PEM/DER fingerprint parsing and mismatch rejection;
- sidecar manifest parsing for both Linux targets.

### Transaction and uninstall

- preview changes nothing;
- idempotent setup for the same interface set and sidecars;
- setup after a new interface adds it and backs up the previous ruleset under `/etc/own-domains/backups`;
- setup rerun with a changed ruleset restarts the nftables unit and loads the new table;
- setup rerun with a changed resolver unit restarts the resolver unit;
- rejection of a user-owned, group/world-writable, or symlinked privileged backup directory;
- setup retaining an absent covered interface name without a manifest error;
- address-aware port checks that ignore the resolved stub loopback listener on port 53 and reject wildcard, covered-interface, and loopback setup-server listeners;
- state-directory rejection for relative paths, `..`, paths resolving to a directory not owned by the target user, and group/world-writable paths;
- `nftables.service` plus `flush ruleset` conflict detection as a warning;
- unowned file, unit, table, link, or certificate conflicts;
- rollback after nftables validation failure, unit start failure, `update-ca-certificates` failure, final-probe failure, and settings-persistence failure;
- unresolved-item reporting when rollback itself fails;
- uninstall preview/apply, fingerprint mismatch refusal, resumable uninstall after a partial trust removal, manifest removal last, and `--purge` behavior;
- uninstall refusal while the runtime is live.

### Inspector and runtime

- missing, corrupt, wrong-owner, wrong-mode, wrong-platform, and wrong-host-integration-version manifests;
- artifact hash, unit state, resolved DNS/domain/default-route, trust, sidecar, port, Docker, and firewall diagnostics;
- `nftablesUnverifiable`, `firewallUnverifiable`, and `dockerUnverifiable` warnings when policy sources are unreadable;
- no duplicate diagnostics;
- address change without setup, interface switch, inactive address, uncovered interface, interface rename, and an absent covered interface;
- runtime start/reconcile/stop with fake process, file, command, and readiness adapters;
- failed candidate retaining the previous applied state;
- listener inspection fixtures for `ss` output and owned-PID filtering;
- crash recovery and ambiguous-PID refusal;
- frontend save behavior proving an address change does not produce `setupRequired`;
- platform-facade contract tests with fake Linux/macOS installers and inspectors, proving shared callers do not depend on platform types;
- the existing macOS runtime, host, setup, PF, manifest, frontend, and CLI suites passing unchanged after the shared Unix extraction.

### Packaging and release

- Linux sidecar fetch and verify for both targets;
- archive checksum, executable SHA-256, and version checks on matching hosts;
- Caddy checksum-signature verification;
- package layout checks for `.deb` and AppImage;
- `.deb` `/usr/lib/own-domains` and AppImage `<AppDir>/usr/lib/own-domains` resource resolution, including rejection of a non-root-owned `OWN_DOMAINS_RESOURCES` during privileged setup;
- arm64 build-only checks that do not execute arm64 binaries on x86_64;
- macOS release checks still passing.

Run the repository's normal gates: `pnpm test`, `pnpm run lint`, `pnpm run typecheck`, `pnpm run build`, `cargo fmt --check`, `cargo clippy`, and the release checks. Linux-specific tests must run on Ubuntu 24.04, not only macOS.

### Manual acceptance

On a physical Ubuntu 24.04 x86_64 host:

1. Preview setup and confirm no files, units, links, tables, or trust changes.
2. Apply setup and confirm the four owned artifacts (nftables configuration, both unit files, and certificate), the dummy link, the table, and system trust.
3. Start routing from the GUI and CLI. Confirm `.own` DNS over UDP and TCP, HTTP, HTTPS, `setup.own`, and a phone on the same LAN.
4. Change the LAN address on the same interface; save it; confirm routing without sudo.
5. Switch between covered Wi-Fi and Ethernet; confirm routing without sudo.
6. Stop and restart routing; confirm no setup prompt and that `.own` fails while stopped. Record whether the phone's other names also fail when the phone uses the host as its only DNS server.
7. Confirm the manifest contains no LAN address and the nftables/resolved artifacts are unchanged across address changes.
8. Reboot; confirm the units load and routing works after an unprivileged start. If `nftables.service` is enabled, reload it and confirm doctor reports the table loss instead of silently claiming health.
9. Enable UFW with a default-deny policy; confirm the documented high-port rules make `.own` work and no other ports are opened.
10. Install Docker; confirm a container publishing 80 is reported as a conflict and a container on an unrelated port coexists.
11. Run uninstall; confirm the units, table, dummy link, certificate, and manifest are gone, trust is removed, and prior settings are preserved without `--purge`.
12. Repeat the physical-phone HTTPS and CA-install checks after a fresh setup.

Also unplug and reconnect a covered USB Ethernet adapter, and disconnect and reconnect a covered Wi-Fi interface, to prove absent covered names remain valid and become usable without setup.

arm64 is build-only until an arm64 host or runner records the same evidence. No runtime support is claimed before that record exists.

## Real Ubuntu verification gates

No gate below has been run. Implementation may begin, but the design is not accepted and Linux support is not claimed until the gates pass and the results are recorded in `docs/linux-acceptance.md`. A failed gate blocks the affected part of the design; it must not be worked around with a privileged helper.

| Gate | What must be proved | Status |
| --- | --- | --- |
| A. nftables input | `fib daddr . iif type local` matches only local destinations on the input interface; `redirect to :port` targets the input interface's primary address; per-interface scoping works; an address change follows without reload; a rule on a present addressless interface begins matching when an address appears | Not run |
| B. nftables output | `meta skuid` plus `fib daddr type local` redirects only the target user's host-originated HTTP/HTTPS to local non-loopback addresses; external traffic, loopback, and CoreDNS upstream queries are unaffected | Not run |
| C. nftables coexistence | The `-110` prerouting priority behaves as intended with Docker; a published conflicting port is detected; the dedicated table coexists with Docker, UFW, and an enabled `nftables.service` without editing their tables; a `flush ruleset` reload is detected | Not run |
| D. resolved split DNS | The dummy link plus `resolvectl dns ... 127.0.0.1:5354`, `domain ... ~own`, `default-route no`, `dnssec no`, and `dnsovertls no` routes `.own` to CoreDNS and all other names to the physical link; works with NetworkManager (Desktop) and systemd-networkd (Server); the stub accepts UDP and TCP; survives a resolved restart; `/etc/resolv.conf` points at the stub | Not run |
| E. resolved failure modes | Stopping CoreDNS makes host `.own` fail while host other names resolve; a conflicting `~own` on another link is detected; negative caching does not cause an unacceptable delay after start/stop; LAN-client behavior when the host is the only DNS server is recorded | Not run |
| F. UFW | With UFW active and default-deny incoming, the documented high-port rules make `.own` work; without them, the diagnostic is accurate; unprivileged doctor reports `firewallUnverifiable` rather than claiming safety; no unrelated ports are opened | Not run |
| G. trust | `update-ca-certificates` adds the certificate; `openssl verify` succeeds; uninstall removes the file and bundle entry; a fingerprint mismatch is refused; a partial trust removal resumes; Firefox/Chrome behavior is recorded | Not run |
| H. systemd units | Both units load at boot, are enabled/active, are idempotent on restart, and clean up the table/link on uninstall; the hardening directives do not block `nft`, `ip`, or `resolvectl`; `PartOf` reapplies resolved settings; `nft` `destroy table` or the documented fallback works on Ubuntu 24.04's nftables; an enabled `nftables.service` reload is detected after it deletes the table | Not run |
| I. reboot and upgrade | Host integration survives reboot; an unchanged app upgrade needs no setup; a sidecar or host-integration change produces `setupRequired` and a setup rerun succeeds | Not run |
| J. physical phone | DNS, HTTP, HTTPS, CA installation through `setup.own`, and loss of `.own` after stop work on a real phone; no management endpoint is exposed | Not run |
| K. packaging | `.deb` and AppImage resource resolution, sidecar digests, and the desktop launch path work on a clean Ubuntu 24.04 install | Not run |
| L. arm64 | Build and package for `aarch64-unknown-linux-gnu`; run the same gates on arm64 hardware before claiming runtime support | Not run |

Specific syntax and behavior that must be verified, not assumed:

- `redirect to :port` in `prerouting` and `output` on Ubuntu 24.04's nftables.
- `fib daddr . iif type local` and `fib daddr type local` in the intended hooks.
- Whether bridged or bonded traffic presents `iifname` as the master and matches exactly once, including with `br_netfilter`.
- `meta skuid` matching the socket owner for browsers and sandboxed apps.
- `destroy table ip own_domains` support; otherwise the `ExecStart=-/usr/sbin/nft delete table ...` fallback.
- `resolvectl dns <link> 127.0.0.1:5354` with a port and `resolvectl domain <link> "~own"` on both NetworkManager and systemd-networkd systems.
- Whether `resolvectl` can configure a just-created dummy link immediately after `ip link add` at boot, or the unit needs a bounded retry; a boot race must not leave the link without `~own`.
- Whether resolved uses DNS servers on a dummy link with no addresses; if not, add a link-local address and re-run Gate D.
- Whether a NetworkManager restart clears the dummy link's resolved settings; if so, decide whether a dispatcher is justified.
- `ss -H -ltnup` output and PID visibility for owned children.
- `update-ca-certificates` add/remove behavior and the exact symlink/bundle state.
- Whether UFW sees the translated high ports and whether the documented rules are sufficient.
- Whether `ufw status` and `docker ps` are readable as the target user, and whether `firewallUnverifiable`/`dockerUnverifiable` accurately describe the fallback.
- Whether Docker's `dstnat` ordering and the `-110` priority behave as intended.
- `.deb` and AppImage resource-directory resolution and the Tauri Linux bundle layout.
- Caddy CA generation as the target user on Linux, including XDG paths and file ownership.

## Risks and fallbacks

- The dummy-link split-DNS mechanism may not work with every NetworkManager/resolved combination. If Gate D fails because resolved ignores a DNS server on an addressless link, the first fallback is to add a link-local address (for example `169.254.53.54/32`) to `own-domains0`; the output chain already excludes link-local destinations. If that still fails, the fallback is a systemd-networkd `.netdev`/`.network` pair that creates the dummy link declaratively. Enabling networkd on a NetworkManager desktop is a larger host change and must be verified separately. A privileged helper is not an acceptable fallback.
- `redirect to :port` may not target the input interface's primary address. If it targets a different address, the input rule must use an explicit destination expression or the selected address must be pinned, which would violate the address-change requirement. The gate must resolve this before implementation.
- The output redirect is broader than the input rule because nftables cannot express "the destination belongs to a covered interface" in the output hook. The UID and loopback/link-local exclusions bound the risk. If a user runs another local web service on a non-loopback address, host access to that service may be redirected. A future design can add a narrow root network-change hook to maintain an address set, but that is a new privileged mechanism and is out of scope.
- UFW/firewalld may block the translated high ports. The design documents the required rules and detects the conflict; it does not modify firewall policy. If the user cannot change the policy, LAN `.own` cannot work without a different design.
- Docker may publish a conflicting port after setup. Doctor detects it when Docker state is readable, but the user must resolve it. The design does not edit Docker tables.
- Doctor may not be able to read UFW/firewalld or Docker state as the target user. It reports the state as unverifiable instead of claiming safety; setup and the physical-phone test remain authoritative.
- Firefox and Chrome may not trust the system store. Users may need to import the CA manually. This is a browser limitation, not a setup failure.
- A resolved restart or NetworkManager restart may clear the dummy-link settings. The resolver unit handles resolved restarts; a NetworkManager-only reset may require a manual unit restart or a future dispatcher.
- The Linux manifest is new and has no migration from macOS. A macOS state directory copied to Linux is rejected with `setupRequired`/`setupManifestInvalid`. A Linux schema change requires a manifest schema bump and a setup rerun.
- A sidecar update requires one privileged setup. This is intentional: the manifest records the privileged verification of the sidecars.
- arm64 remains build-only until hardware acceptance. The x86_64 acceptance does not prove arm64 runtime behavior.

## Known limitations

- IPv4 only. IPv6-only LANs and IPv6 `.own` records are unsupported.
- systemd-resolved is required. Systems using another resolver, a static `/etc/resolv.conf`, or a resolver that does not honor route-only domains are unsupported.
- Browser DNS-over-HTTPS can bypass resolved and break `.own`. Users must disable DoH or add an exception.
- Firefox and Chrome may not use the system trust store. Manual CA import may be required.
- Sandboxed browsers whose network process runs under a different UID may not match the output redirect. `.own` may fail in those browsers even when system DNS is correct.
- The host-originated output redirect can affect the target user's access to other local non-loopback services on ports 80 and 443. This is bounded by UID and loopback/link-local exclusions but not eliminated. A mapping whose upstream is another local non-loopback service on 80 or 443 can loop back into Caddy; use a loopback upstream for local services.
- Only the selected interface's LAN address is served. A LAN client on another covered interface is redirected to a closed high port and cannot use the host for `.own` until that interface is selected.
- Ports 53, 80, and 443 on every covered interface, not only the selected interface, remain redirected even when the runtime is stopped. No process listens on the targets, so `.own` fails, but another service on those ports is shadowed until uninstall.
- A LAN client that uses the host as its only DNS server loses all DNS while the runtime is stopped, not just `.own`. Clients that use the host only for `.own` need their own split-DNS configuration.
- New hardware interfaces and renamed interfaces require one additional privileged setup.
- Absent covered interface names are retained so the interface works again when reconnected. They count toward the 32-interface cap; a host that accumulates more than 32 distinct names must uninstall and reinstall.
- VLAN subinterfaces named `<parent>.<id>` are not covered unless renamed to `vlan*`.
- UFW and firewalld require a one-time high-port policy change; the design does not modify firewall policy.
- Docker publications on the required ports are conflicts; the design does not republish or stop containers.
- Doctor may not be able to verify UFW/firewalld or Docker policy without root. It reports the state as unverifiable; setup and the phone test are authoritative.
- A later `systemctl reload nftables` can delete the Own Domains table when the distribution unit uses `flush ruleset`. Doctor detects the loss; setup does not edit `/etc/nftables.conf`.
- Containers, VMs, and sandboxed apps with their own network namespaces or DNS configuration may not use the host's `.own` path.
- VPN, WireGuard, Tailscale, ZeroTier, and tunnel interfaces are not covered. `.own` over those interfaces is unsupported.
- The resolver unit is a static oneshot, not a helper. If an environment forbids any root systemd unit, the no-root-after-setup requirement cannot be met by this design.
- Unix domain socket paths are limited to 108 bytes. A very long `$XDG_DATA_HOME` can prevent the daemon socket or Caddy admin socket from binding.
- The CLI must be run from a trusted packaged binary. A user-writable binary or AppImage run with `sudo` is outside the security boundary; use the `.deb` or a root-owned copy for setup.
- Privileged setup backups live under `/etc/own-domains/backups`, are readable only by root, and are removed by uninstall.
- Ubuntu versions other than 24.04 LTS are not covered by this design's acceptance.
- arm64 is build-only until an acceptance record exists.

## Documentation

Update:

- `README.md`: Linux support status and the one-time setup boundary.
- `docs/platform-support.md`: Linux implementation and acceptance status, x86_64 accepted vs arm64 build-only.
- `docs/desktop.md`: Linux `.deb`/AppImage layout, state directory, setup command, and GUI behavior.
- `docs/cli.md`: Linux `setup --apply`, `uninstall --apply`, `doctor`, and the no-root-after-setup boundary.
- `docs/linux-acceptance.md`: the gates, commands, outputs, environment versions, and pass/fail evidence.

Documentation must state:

- setup and uninstall are the only privileged commands;
- address changes and covered-interface switches do not require root;
- a new or renamed interface may require one setup;
- UFW/firewalld high-port rules may be required once;
- Firefox/Chrome may need manual CA import;
- ports 53, 80, and 443 remain reserved on every covered interface, not only the selected one, while host integration is installed;
- a LAN client that uses the host as its only DNS server loses all DNS while the runtime is stopped;
- covered interface names are retained when an interface is absent and count toward the 32-interface cap;
- VLAN subinterfaces named `<parent>.<id>` are not covered unless renamed to `vlan*`;
- doctor may report UFW/firewalld or Docker policy as unverifiable without root;
- a `nftables.service` reload with `flush ruleset` can remove the table;
- the `.deb` is the recommended package for privileged setup, and a user-writable AppImage run with `sudo` is outside the trust boundary;
- the exact diagnostics and remedies;
- the arm64 support status.

## Implementation order

1. Land the behavior-preserving transport extraction shared with Windows (endpoint abstraction, JSON-line codec, shared `SidecarManifest`/`SidecarSpec`, setup server, client/desktop/CLI plumbing). macOS tests, lint, typecheck, and build must pass unchanged.
2. Land the behavior-preserving shared Unix extraction. macOS tests, lint, typecheck, and build must pass unchanged.
3. Move platform support decisions behind the facade; remove the macOS-only guards from shared CLI command code.
3. Add Linux sidecar targets, build manifest entries, fetch/verify/release support, and packaging layout.
4. Implement the Linux host artifact rendering: nftables table, systemd units, trust certificate, and Linux manifest.
5. Implement the Linux setup/uninstall transaction, inspector, and diagnostics.
6. Implement `LinuxRuntimeBackend` using the shared Unix runtime and Linux listener inspection.
7. Run Gates A–H on a disposable Ubuntu 24.04 x86_64 host. Stop and revise the design if the nftables or resolved behavior fails.
8. Run Gates I–K, then physical-phone Gate J.
9. Record acceptance evidence, update the support matrix, and mark x86_64 supported. Leave arm64 build-only until Gate L passes.

No Linux host behavior may be mixed into the transport extraction or the shared Unix extraction commit series.

## Out of scope

- A privileged helper daemon, root socket, polkit action, or sudoers entry for runtime operations.
- Zero-root installation or operation before the first setup.
- IPv6 LAN support.
- Supporting resolvers other than systemd-resolved.
- Automatic NetworkManager, netplan, UFW, firewalld, Docker, or browser-store changes.
- Privileged-port listeners, socket activation, or file capabilities for CoreDNS/Caddy.
- A GUI button that runs setup.
- Changing the IPC protocol, settings schema, mapping semantics, or reserved `setup.own` behavior.
- Windows support.
- arm64 runtime support before acceptance.
- Linux AppImage packaging (deferred; `.deb` only for the first Linux support claim).
- AppImage/deb auto-update behavior and distribution signing beyond the package integrity assumptions in this design.

## Source constraints

- systemd-resolved and `resolvectl` provide the stub resolver, per-link DNS servers, route-only domains, DNSSEC, and DNS-over-TLS controls used here: https://www.freedesktop.org/software/systemd/man/latest/systemd-resolved.service.html, https://www.freedesktop.org/software/systemd/man/latest/resolved.conf.html, and https://www.freedesktop.org/software/systemd/man/latest/resolvectl.html
- systemd `.network` files document `DNS=`, `Domains=`, and `DNSDefaultRoute=` for the declarative fallback: https://www.freedesktop.org/software/systemd/man/latest/systemd.network.html
- systemd unit ordering, `PartOf=`, and `RemainAfterExit=` are used for the static persistence units: https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html and https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html
- nftables documents NAT base chains, priorities, `redirect`, `fib`, and `meta skuid`: https://netfilter.org/projects/nftables/manpage.html and https://wiki.nftables.org/wiki-nftables/index.php/Performing_Network_Address_Translation_(NAT)
- Ubuntu's `update-ca-certificates` installs certificates from `/usr/local/share/ca-certificates` into the system store: https://manpages.ubuntu.com/manpages/noble/man8/update-ca-certificates.8.html
- Docker's packet-filtering documentation describes its nftables/iptables chains and the `DOCKER-USER` chain: https://docs.docker.com/network/packet-filtering-firewalls/
- XDG base directories define the Linux state layout: https://specifications.freedesktop.org/basedir-spec/latest/
- CoreDNS and Caddy continue to use the existing pinned versions and bindings: https://coredns.io/manual/toc/, https://coredns.io/plugins/bind/, https://caddyserver.com/docs/command-line, and https://caddyserver.com/docs/api

