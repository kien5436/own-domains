# Network-Independent macOS Host Integration Design

## Summary

The current macOS runtime pins the selected LAN IPv4 address in the privileged PF anchor and install manifest. Changing the address, or switching between Wi-Fi and Ethernet, therefore requires rerunning `setup --apply` with administrator credentials. The GUI can save the new value, but the daemon correctly refuses to widen host integration from an unprivileged process and reports host-integration drift with `lanIntegrationDrift`.

This design removes the LAN address from all persistent privileged artifacts. Setup writes PF rules that reference each covered interface dynamically. The selected address remains unprivileged settings, and the daemon reconfigures CoreDNS and Caddy when it changes. After the one-time setup, changing the address or switching between covered interfaces requires no administrator credentials and no privileged helper.

## Context

The existing macOS runtime activation design deliberately made LAN address/interface changes an explicit privileged update:

- `/etc/resolver/own` is static and already independent of the LAN address.
- `/etc/pf.anchors/dev.owndomains` contains the selected interface and address.
- `install-manifest.json` records the selected interface and address and hashes the anchor.
- `HostIntegrationInspector` compares settings to the manifest and reports drift.
- CoreDNS and Caddy bind to loopback and the selected LAN address on unprivileged high ports.

That design is secure and simple, but it forces a root prompt on ordinary DHCP changes. The selected address is not a persistent security principal; it is mutable runtime state. Storing it in the root-owned PF anchor creates recurring friction without adding a security property that cannot be preserved dynamically.

The one-time setup remains privileged because macOS requires root to write `/etc/resolver`, install PF rules, and modify the trust store. Normal launches, address changes, interface changes, mapping mutations, and shutdown remain unprivileged.

## Platform boundaries

This feature is macOS-specific because PF, `/etc/resolver`, and the trust store are macOS-specific. The shared application must not learn PF, interface-list, or manifest-schema concepts.

Portable contracts stay platform-neutral:

- `RuntimeBackend` remains the daemon's runtime contract: apply, stop, inspect, and diagnose.
- `HostInspector` becomes a shared daemon contract returning `Diagnostic` values. The macOS implementation continues to own resolver, PF, trust, sidecar, and listener checks.
- `HostInstaller` becomes a shared daemon contract for setup preview/apply and uninstall preview/apply. Its request contains only a state directory, selected LAN address, interface name, and purge flag. Each platform adapter validates those values and owns its own privileged artifacts.
- `SetupPreview`, `SetupApplyResult`, `SetupFailure`, and shared command/file abstractions move out of the macOS namespace when they have no macOS-specific fields.
- Platform selection is compile-time through a facade in `platform`. The daemon binary and CLI ask the facade for a runtime backend or host installer instead of importing `platform::macos` directly.
- Non-macOS platforms continue to report unsupported setup and `runtimeUnavailable` until an adapter exists.

The following remain inside `platform/macos` and are not abstracted into shared code:

- PF anchor rendering and rule syntax.
- `/etc/resolver/own` and `/etc/pf.conf` composition.
- The macOS install manifest schema, artifact paths, trust-store commands, sidecar target triples, and interface discovery rules.
- Mutable macOS network-selection details beyond the portable request fields.

A future Linux or Windows adapter can implement the installer, inspector, and runtime contracts without changing the GUI, protocol, core settings model, or daemon request handling. It must supply its own privileged host artifacts and acceptance evidence.

## Preparatory refactor

The platform boundary is a separate, behavior-preserving refactor before the network-independent behavior:

1. Move the shared `HostInspector` contract and generic installer result/failure/action types out of `platform::macos`.
2. Define `HostInstaller` and platform-neutral setup/uninstall request types.
3. Move macOS setup orchestration behind a macOS `HostInstaller` implementation; the CLI retains argument parsing, privilege entry checks, and output rendering.
4. Add the compile-time platform facade for runtime and installer selection. Keep the existing macOS re-exports while callers migrate.
5. Add contract tests using fake installer and inspector implementations. Existing macOS artifact, PF, manifest, setup, runtime, and frontend tests must pass unchanged.

This refactor changes no host artifacts, paths, manifest fields, diagnostics, or runtime behavior. The network-independent feature work starts only after the refactor and the PF feasibility gate pass.

## Goals

- Remove recurring administrator prompts for LAN IPv4 address changes.
- Remove recurring administrator prompts when switching between covered LAN interfaces.
- Keep Caddy and CoreDNS bound only to loopback plus the selected LAN address; do not introduce wildcard listeners.
- Keep PF redirects scoped to traffic addressed to a covered interface's own address.
- Keep the install manifest free of mutable network state.
- Preserve existing mappings, settings, CA identity, resolver behavior, and uninstall reversibility.
- Preserve the option to uninstall and restore the prior host configuration.
- Make diagnostics actionable when the selected address is not active.

## Non-goals

- Zero-root installation. The initial setup still requires administrator credentials once.
- A persistent privileged helper daemon or root command socket.
- Apple Network Extension entitlements or a no-root-at-install architecture.
- Automatic support for a hardware interface added after setup if PF cannot safely pre-cover absent interfaces; that case may require one additional setup.
- Changing the CoreDNS, Caddy, or PF ports.
- Changing settings schema version 1 or mapping semantics.
- Supporting Ubuntu or Windows in this design.
- Abstracting PF syntax, resolver files, or install-manifest fields into shared code.
- Automatically modifying third-party VPN, DNS, proxy, firewall, or trust configuration.

## Selected architecture

The privileged host integration becomes static except for the interface list:

| Artifact | Content | Changes after setup |
| --- | --- | --- |
| `/etc/resolver/own` | `nameserver 127.0.0.1`, `port 5354` | Never |
| `/etc/pf.conf` block | Own Domains anchor load/redirect block | Never |
| `/etc/pf.anchors/dev.owndomains` | Dynamic per-interface redirect rules | Only when covered interfaces change |
| `install-manifest.json` | Schema 2: owned artifact hashes, sidecars, covered interfaces | Only when setup is rerun |
| `settings.json` | Selected LAN address, upstream DNS, mappings | Freely changed by the GUI/CLI |

Caddy and CoreDNS continue to listen on `127.0.0.1` and the selected LAN address. PF redirects traffic arriving on a covered interface to that same interface's current address. The redirect target is never loopback, preserving the existing protection against exposing loopback-only services to external interfaces.

### Dynamic PF rules

For each covered interface `<if>` and each service pair, setup writes rules of this form:

```text
rdr pass on lo0 inet proto udp from any to (<if>:0) port 53 -> (<if>:0) port 5354
rdr pass on lo0 inet proto tcp from any to (<if>:0) port 53 -> (<if>:0) port 5354
rdr pass on lo0 inet proto tcp from any to (<if>:0) port 80 -> (<if>:0) port 8080
rdr pass on lo0 inet proto tcp from any to (<if>:0) port 443 -> (<if>:0) port 8443
rdr pass on <if> inet proto udp from any to (<if>:0) port 53 -> (<if>:0) port 5354
rdr pass on <if> inet proto tcp from any to (<if>:0) port 53 -> (<if>:0) port 5354
rdr pass on <if> inet proto tcp from any to (<if>:0) port 80 -> (<if>:0) port 8080
rdr pass on <if> inet proto tcp from any to (<if>:0) port 443 -> (<if>:0) port 8443
```

The existing static loopback rules for `127.0.0.1` remain for DNS, HTTP, and HTTPS. A macOS PF quirk corrupts state and drops later connections when a loopback redirect preserves the address (`127.0.0.1` to `127.0.0.1`). A direct UDP query to `127.0.0.1:5354` receives a reply addressed to port 53, which breaks the static `/etc/resolver/own` entry that points at port 5354. A direct TCP connection to `127.0.0.1:5354`, `:8080`, or `:8443` succeeds once, then later connections time out until the bogus state expires. Exempt replies from the high-port targets before the loopback rules:

```text
no rdr on lo0 inet proto udp from any port 5354 to any
no rdr on lo0 inet proto tcp from any port 5354 to any
no rdr on lo0 inet proto tcp from any port 8080 to any
no rdr on lo0 inet proto tcp from any port 8443 to any
rdr pass on lo0 inet proto udp from any to 127.0.0.1 port 53 -> 127.0.0.1 port 5354
rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 53 -> 127.0.0.1 port 5354
rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port 8080
rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 443 -> 127.0.0.1 port 8443
```

The exemptions must precede the `rdr` rules. They only affect local packets sourced from the high-port targets; redirected replies still receive their stateful reverse translation, and the 53-to-5354, 80-to-8080, and 443-to-8443 redirects continue to work.

The `(<if>:0)` form selects the interface's primary IPv4 address and is updated automatically when that address changes. It preserves the exact-address scope: a rule for `en0` does not redirect traffic addressed to `en1` or to an unrelated destination.

### Covered interfaces

Setup discovers eligible interfaces with fixed `/sbin/ifconfig -l` and `/sbin/ifconfig <name>` commands, without a shell. Eligible names are physical LAN interfaces:

- `en*` for built-in and USB Ethernet/Wi-Fi.
- `bridge*` for bridge interfaces.

Loopback, `utun*`, `awdl*`, `llw*`, `anpi*`, `gif*`, `stf*`, and other virtual interfaces are excluded. Interface names must be nonempty ASCII alphanumeric identifiers no longer than 15 bytes.

The selected initial address must be the primary IPv4 address of the selected interface. Alias addresses are not accepted as the selected LAN address because the dynamic `:0` rules intentionally target the primary address only.

Setup covers every eligible interface present at setup, including interfaces that do not currently have a primary IPv4 address. A rule for a present addressless interface is inert until the interface gains an address; the feasibility gate confirmed that such a rule begins redirecting once the address appears, without reloading the anchor.

Absent interface names are not pre-covered. `pfctl` accepts a rule for an absent name, but the gate could not prove that the rule becomes usable when the interface appears. Adding a previously absent hardware interface therefore requires one additional setup. This is the only accepted recurring-root exception.

## Manifest schema 2

`HostInstallManifest` gains schema version 2:

```text
schemaVersion: 2
userName
userId
backupDirectory
interfaces: string[]
artifacts: ManifestArtifact[]
sidecars: SidecarManifest
```

The schema removes `interface` and `lanAddress`. The manifest is owned by the target user with mode `0600` and records only immutable installation facts. Setup and uninstall validate the manifest structure, artifact paths and metadata, backup location, and manifest ownership/mode before acting.

Migration behavior:

- A schema 1 manifest is readable for setup and uninstall after structural validation of its artifact paths, backup location, manifest ownership/mode, and artifact metadata.
- Runtime activation with schema 1 reports `setupRequired` with the remedy “Run setup once to enable network-independent routing.”
- Running `setup --apply` against schema 1 validates the existing owned artifacts, backs them up, writes schema 2 and the new anchor, and succeeds without requiring the old address to still be active.
- Uninstall supports both schema versions through the existing artifact list and backups.
- Settings remain schema version 1.

## Setup changes

The `setup` command keeps its `--lan-address` and `--interface` arguments for initial validation and settings persistence, but the generated host plan covers all eligible interfaces:

1. Discover eligible present interfaces and validate that the requested interface is present and its primary IPv4 address equals the requested address; reject aliases and absent interfaces.
2. Reject an empty eligible interface list.
3. Generate the dynamic anchor; do not interpolate the selected address.
4. Persist schema 2 with the covered interface list and artifact hashes.
5. Persist the selected address in settings as today.
6. Validate and load the anchor, then run the existing final probes.

Setup remains idempotent. Re-running with the same covered interface set produces identical artifacts and reports them unchanged. Re-running after a new eligible interface appears adds that interface, creates a backup, and updates the manifest. Existing files not owned by the installation remain conflicts.

Uninstall continues to remove only exact owned artifacts, restore backups, and preserve settings unless `--purge` is supplied. It must not depend on the currently selected address.

## Runtime and inspection changes

`HostIntegrationInspector` validates:

- schema 2 manifest structure and artifact hashes;
- resolver content and PF composition block;
- anchor content and active PF state;
- sidecar presence, versions, and digests;
- CA trust;
- the selected settings address is the primary IPv4 address of a covered interface.

It no longer compares settings to a manifest address. The two-command network probe is replaced with one bounded `ifconfig` inspection per eligible interface present at inspection time. This discovers interfaces that appeared after setup as well as currently covered ones. A failure produces one diagnostic per failed condition, never duplicate diagnostics for the same condition.

New diagnostic behaviors:

- `setupRequired`: missing manifest, incompatible manifest, artifact mismatch, or schema 1 migration.
- `lanAddressInactive`: no present eligible interface owns the selected address as its primary IPv4 address. Remedy: select an active LAN address in Network Settings and save.
- `interfaceCoverageMissing`: the selected address is the primary IPv4 address of an eligible interface that is not in the manifest. Remedy: run setup once to add the interface.
- Coverage precedence: `interfaceCoverageMissing` wins when the selected address belongs to an uncovered eligible interface, even if every covered interface is temporarily addressless. Otherwise the inspector reports `lanAddressInactive`. An unused interface that appeared after setup does not produce a diagnostic.
- Existing resolver, PF, CA, sidecar, and port diagnostics remain unchanged.
- `lanIntegrationDrift` is removed because address drift is no longer a host-integration failure. Tests that assert its presence are updated.

Runtime start validates the selected address before starting sidecars. A stopped runtime may save an address that is not currently active; start fails with `lanAddressInactive` or `interfaceCoverageMissing` until the selected address is active on a covered interface. No host setup is required for an address change.

## GUI and daemon behavior

The Network Settings form continues to save `lanAddress` and `upstreamDns` through `updateNetwork`. No new privilege path is added.

When the runtime is running and the selected address changes:

1. The daemon saves the new settings at the expected revision.
2. Runtime reconciliation rebuilds the CoreDNS and Caddy configuration with the new listeners and `.own` A records.
3. If the new address is active, the runtime restarts or reconciles the sidecars and reports success.
4. If it is not active, the persisted settings remain, the runtime reports `lanAddressInactive`, and the previous applied configuration is retained where the existing reconciliation semantics allow it.

The GUI displays the diagnostic remedy as an unprivileged action. It must not instruct the user to rerun setup for an ordinary address change.

A future address picker may list the active primary addresses, but it is not required for this design. The existing text field remains supported.

## Security boundaries

- Only explicitly eligible interfaces receive PF rules.
- Rules match traffic addressed to that interface's own primary IPv4 address.
- Redirect targets are the same interface address, not loopback, so external connections cannot reach services that listen only on loopback.
- Caddy and CoreDNS do not bind wildcard addresses.
- No root helper, root socket, or unauthenticated privileged command is introduced.
- The privileged anchor remains root-owned with mode `0644`; the install manifest is owned by the target user with mode `0600`. Both are replaced only by `setup --apply`.
- Setup and uninstall reject a manifest with unexpected ownership, mode, artifact paths, backup location, or artifact metadata.
- The selected address remains user-controlled data and never becomes interpolated into a root-owned template.
- PF rule generation uses fixed paths, fixed ports, validated interface names, and fixed argument arrays.
- VPN, utun, and other non-LAN interfaces remain unredirected.

## Testing and acceptance

Automated coverage must include:

- exact anchor rendering for multiple interfaces, including dynamic `(if:0)` rules and no selected-address interpolation;
- coverage of present interfaces without a primary IPv4 address;
- rejection of invalid, loopback, oversized, or duplicate interface names;
- schema 2 manifest serialization, schema 1 migration, setup preview/apply, and uninstall;
- inspector behavior for changed addresses without setup, switching to another covered interface, inactive addresses, uncovered interfaces, missing anchors, and sidecar drift;
- runtime reconciliation after an address change, including a failed candidate that retains the previous applied state and an inactive address that fails before replacing it;
- no duplicate diagnostics;
- frontend save behavior proving no `setupRequired` message after an address change;
- platform-facade contract tests with a fake installer and fake inspector, proving callers do not depend on macOS types;
- existing Rust, frontend, lint, typecheck, build, fmt, Clippy, and release checks.

A macOS feasibility gate must run before implementation proceeds beyond host rendering:

1. Load a temporary `dev.owndomains.test` anchor containing dynamic `(en0:0)` rules. Do not load it into the production anchor.
2. Confirm the rules load. `pfctl -s nat` displays `(if:0)` symbolically rather than expanding the address; use a packet test to confirm the effective redirect target.
3. Change the interface address using DHCP or a reversible test alias and confirm the loaded rule updates without reloading the anchor by repeating the packet test at the new address.
4. Switch between two available LAN interfaces and confirm each interface's rule uses that interface's own address by packet test.
5. Load a rule for an absent interface name such as `en9`; record whether `pfctl` accepts it. If an additional interface can be created or attached safely, verify that the rule becomes usable when the interface appears. This determines whether absent names may be pre-covered.
6. Remove the temporary anchor and leave the production PF state unchanged.

If dynamic `(if:0)` rules do not update on address changes, this design is blocked. Do not add a privileged helper without a new design decision.

#### Feasibility gate results

Run on 2026-10-24 against macOS 15.2 (24C101) with PF enabled. The test used the temporary child anchor `com.apple/owndomains-gate`, reversible `192.0.2.x` aliases on `en8`, and bounded Python TCP listeners and clients. It did not modify the production `dev.owndomains` anchor, `/etc/pf.conf`, or `en0` addressing.

| Check | Result |
| --- | --- |
| Static control redirect | Passed. |
| `(en8:0)` packet redirect at `192.0.2.210` | Passed. |
| Change `en8` primary address to `192.0.2.211`, no anchor reload | Passed at the new address. |
| Per-interface scoping with `en0` at `192.168.1.4` and `en8` at `192.0.2.210` | Each redirected its own address. |
| Rule loaded while present `en8` had no IPv4, then address added | Passed after the address appeared. |
| Rule loaded for absent `en9` | `pfctl` accepted it and displayed `(en9:0:*)`; appearance behavior was not tested. |
| Rule loaded for absent `bridge3`, then bridge created | Inconclusive. The bridge was inactive with no member and the probe failed; the setup could not establish a usable bridge path. |
| `pfctl -a <anchor> -s nat` output | Displays `(if:0)` symbolically, not the current address. The `pf.conf` man page documents that parenthesized interfaces update automatically; packet tests are authoritative. |

Cleanup removed all test aliases, listeners, the created bridge, and temporary anchor rules. The production anchor's SHA-256 remained `9bc57bac8aeaf1dd4963ff1a57d10115acf09094f14bb0a9ec15314fa36fc1bb`.

Decision: cover eligible interfaces present at setup, including addressless ones. Do not pre-cover absent names. This satisfies the gate's blocking condition because address changes and per-interface scoping work at packet level.


Manual acceptance:

- Run setup once on the new beta.
- Change the LAN address on the same interface; save it in the GUI; confirm DNS, HTTP, HTTPS, `setup.own`, and phone access without admin.
- Switch between covered interfaces; confirm the same behavior without admin.
- Stop and restart routing; confirm no setup prompt.
- Verify the install manifest contains no selected address and the anchor hash remains stable across address changes.
- Verify the phone cannot use `.own` when routing is stopped.
- Verify uninstall restores prior resolver and PF files.

## Risks and fallbacks

- PF syntax or macOS behavior may differ from the documented dynamic interface expansion. The feasibility gate is mandatory.
- Pre-covering absent interface names is not used: the gate could not prove that a rule loaded before an interface appears becomes usable when it appears. Only eligible interfaces present at setup are covered; a new hardware interface requires one setup.
- Requiring the selected address to be the primary IPv4 address may reject a current alias configuration. The UI remedy is to select the interface's primary address.
- Interface names can change after hardware changes. The app must report an uncovered interface rather than silently widening rules.
- The PF anchor becomes larger with multiple interfaces. Anchor size is bounded by the fixed port set and the covered-interface limit.
- The design does not eliminate one-time privileged setup, because macOS still requires root for resolver, PF, and trust installation.

## Documentation

Update `README.md`, `docs/desktop.md`, `docs/cli.md`, `docs/macos-acceptance.md`, and `docs/platform-support.md` to state:

- setup is a one-time privileged installation;
- address and covered-interface changes no longer require setup;
- new hardware interfaces may require one setup;
- the no-helper and no-root-after-setup boundary;
- the shared runtime, inspector, and installer contracts available to future platform adapters;
- the new diagnostics and acceptance evidence.

## Implementation order

1. Land the behavior-preserving platform boundary refactor.
2. Run the PF feasibility gate. Stop if dynamic rules fail.
3. Update the macOS setup plan, dynamic anchor, schema 2 manifest, and migration.
4. Update the macOS inspector, runtime reconciliation, and diagnostics.
5. Update the frontend behavior and tests.
6. Update documentation and collect manual acceptance evidence.

No network-independent behavior changes may be mixed into the platform refactor commit series.

## Out of scope

- A root helper.
- Network Extension or user-approved system extension architecture.
- No-root installation or no-root operation before the first setup.
- Replacing PF with a privileged-port forwarding mechanism.
- Changing the default ports or DNS resolution model.
- Automatic setup for interfaces that PF cannot pre-cover.
