# Linux Runtime Plan (Phase 2, Milestone 1)

## Goal

Render byte-exact Linux host artifacts (the `ip own_domains` nftables table,
both systemd units, the Linux schema 1 manifest, and the trust certificate
fingerprint) and add the `platform::linux` runtime-backend skeleton, with zero
change to macOS output. Linux CoreDNS and Caddy configuration reuses the shared
TTL-0 and h1/h2 options; this plan does not re-implement them. Source design:
`docs/superpowers/specs/2026-10-26-linux-host-integration-design.md` on the
`docs/cross-platform-adapters` branch.

## Constraints

- macOS rendered output must stay byte-identical: default TTL 60 and no
  `protocols` field unless explicitly set. Do not modify
  `crates/own-domains-core/src/dns.rs` or `crates/own-domains-core/src/proxy.rs`;
  their shared option changes are prerequisites from the Windows rendering plan.
- Shared prerequisites from `docs/plans/2026-10-27-windows-rendering.md`:
  Task 1 (`ttl_seconds` in `DnsConfigOptions`) and Task 2 (`protocols` in
  `ProxyConfigOptions`) must be merged before Task 2 here. The endpoint/transport
  extraction is already merged and is not touched; Task 1 here completes the
  shared `SidecarManifest`/`SidecarSpec` move that remains in the macOS
  namespace.
- Linux selects TTL 0 so address changes are not hidden by client caches, and
  h1/h2 so Caddy never opens a UDP 443 listener; the nftables redirect is
  TCP-only for HTTP and HTTPS.
- No `cfg(target_os)` branches in shared command or core logic. Linux
  constructors live in `platform::linux`; only the `platform/mod.rs` facade
  selection is cfg-gated. `pub mod linux;` is unconditional because this
  milestone contains only pure Rust with no Linux syscalls, so exact-render
  tests run with `cargo test` on every host.
- No new production dependencies. `base64`, `sha2`, `serde_json`, and
  `thiserror` are already present in `crates/own-domains-daemon/Cargo.toml`.
- Sequencing: the design's shared Unix extraction (`platform/unix/*`) is a
  prerequisite for the full runtime backend in Milestone 2, not for pure
  artifact rendering. This plan does not create `platform/unix` or duplicate
  its process/config/files/runtime/setup code. The Milestone 2 backend must land
  on top of that extraction.
- Linux artifacts contain no LAN address, no selected-interface address, and no
  user-controlled input. Covered interface names are validated, sorted, unique,
  and capped at 32.
- All tests are pure: no root, no `ip`, `nft`, `resolvectl`, `systemctl`,
  `update-ca-certificates`, or file writes outside temporary directories.
- Exact-render tests compare against checked-in fixtures; do not replace them
  with substring assertions.

## Out of Scope

- Live `ip`, `nft`, `resolvectl`, `systemctl`, `update-ca-certificates`, and
  `openssl` execution; privileged setup/uninstall transactions; live artifact
  writes; backup, rollback, and trust installation.
- Inspector artifact/unit/resolved/port/Docker/firewall diagnostics and
  remedies beyond the backend skeleton's manifest gate (Milestone 2).
- Runtime start/stop, process supervision, listener inspection, sidecar loading,
  shared Unix extraction, Linux `RuntimePaths`/target triples, and the CLI
  installer/facade-guard removal (Milestone 2).
- Packaging, sidecar fetch/verify, `.deb`/AppImage layout, and acceptance runs.

## Verification Commands

- test: `cargo test --workspace --locked` (repo root)
- sidecar: `cargo test -p own-domains-daemon --test sidecar_manifest` (repo root)
- linux rendering/backend: `cargo test -p own-domains-daemon --test linux_config --test linux_interfaces --test linux_ip --test linux_nftables --test linux_systemd --test linux_trust --test linux_manifest --test linux_platform` (repo root)
- shared defaults: `cargo test -p own-domains-core --test dns --test proxy` (repo root)
- macOS regression: `cargo test -p own-domains-daemon --test macos_config --test macos_host --test macos_setup --test macos_runtime` (repo root)
- linux check: `cargo check --workspace --all-targets --locked` (repo root; run on Ubuntu 24.04 to compile the `cfg(target_os = "linux")` facade branch)
- lint: `cargo clippy --workspace --all-targets --locked -- -D warnings` (repo root)
- fmt: `cargo fmt --all -- --check` (repo root)

### Task 1: Shared sidecar manifest types

Files:
- create `crates/own-domains-daemon/src/platform/sidecar.rs`
- modify `crates/own-domains-daemon/src/platform/mod.rs`
- modify `crates/own-domains-daemon/src/platform/macos/host.rs`
- modify `crates/own-domains-daemon/src/platform/macos/mod.rs`
- create `crates/own-domains-daemon/tests/sidecar_manifest.rs`

Steps:
1. Write test: `own_domains_daemon::platform::SidecarManifest` deserializes a
   JSON object with `schemaVersion: 1`, CoreDNS `1.14.7`, Caddy `2.11.7`, and
   two distinct 64-character lowercase hex digests built with
   `"a".repeat(64)` and `"b".repeat(64)`; assert the schema, versions, and
   digests. On macOS, also assign the parsed value to
   `own_domains_daemon::platform::macos::SidecarManifest` to prove the macOS
   re-export is the same type.
2. Run `cargo test -p own-domains-daemon --test sidecar_manifest`; confirm it
   fails with `unresolved import own_domains_daemon::platform::SidecarManifest`.
3. Implement: move `SidecarSpec` and `SidecarManifest` verbatim from
   `platform/macos/host.rs` to `platform/sidecar.rs`; re-export them from
   `platform/mod.rs`; import them in `platform/macos/host.rs`; re-export them
   from `platform/macos/mod.rs` so every existing macOS import and test keeps
   compiling unchanged.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-daemon --test macos_host --test macos_setup`;
   confirm nothing regressed.

Done when: the shared type parses the inline manifest and the macOS re-export is
unchanged.

### Task 2: Linux runtime config constructors

Files:
- create `crates/own-domains-daemon/src/platform/linux/mod.rs`
- create `crates/own-domains-daemon/src/platform/linux/config.rs`
- modify `crates/own-domains-daemon/src/platform/mod.rs`
- create `crates/own-domains-daemon/tests/linux_config.rs`

Steps:
1. Write test with `Settings { lan_address: "192.168.1.20".into(),
   upstream_dns: vec!["1.1.1.1".into()], ..Settings::default() }`:
   `linux_dns_options` yields `ttl_seconds == 0`, `port == 5354`,
   `upstream_port == 53`, and `listen_addresses == [127.0.0.1, 192.168.1.20]`;
   `render_linux_corefile` returns `CoreResult<String>` and contains
   `0 IN A 192.168.1.20`, `bind 127.0.0.1 192.168.1.20`, and
   `forward . 1.1.1.1:53`, and does not contain `60 IN A`.
   `linux_proxy_options` with admin socket
   `/home/alice/.local/share/own-domains/runtime/caddy-admin.sock` yields
   `protocols == Some(["h1", "h2"])`, ports 8080/8443/4781, and
   `admin_listen == "unix//home/alice/.local/share/own-domains/runtime/caddy-admin.sock"`;
   `render_linux_proxy` returns `CoreResult<serde_json::Value>`; serialize it
   with `serde_json::to_string` and assert the result emits
   `"protocols":["h1","h2"]` on both the `http` and `https` servers, listens on
   loopback and the LAN on 8080/8443, and contains no `0.0.0.0`, no `"h3"`, and
   no `"udp"`. Assert shared `DnsConfigOptions::for_settings` still returns
   `ttl_seconds == 60` and `ProxyConfigOptions::for_runtime` still returns
   `protocols == None`.
2. Run `cargo test -p own-domains-daemon --test linux_config`; confirm it fails
   because `platform::linux` and the constructors do not exist.
3. Implement: register `pub mod linux;` in `platform/mod.rs`; add
   `linux_dns_options` by calling `DnsConfigOptions::for_settings` and setting
   `ttl_seconds = 0`; add `linux_proxy_options` by calling
   `ProxyConfigOptions::for_runtime` and setting
   `protocols = Some(vec!["h1".into(), "h2".into()])`; add
   `render_linux_corefile` and `render_linux_proxy` wrappers over the shared
   builders. Do not add or change the shared option fields here.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-core --test dns --test proxy` and
   `cargo test -p own-domains-daemon --test macos_config`; confirm macOS output
   and shared defaults are unchanged.

Done when: Linux CoreDNS renders TTL 0, Linux Caddy renders h1/h2, and macOS
defaults remain TTL 60 / no protocols field.

### Task 3: Covered interface validation

Files:
- create `crates/own-domains-daemon/src/platform/linux/error.rs`
- create `crates/own-domains-daemon/src/platform/linux/interfaces.rs`
- modify `crates/own-domains-daemon/src/platform/linux/mod.rs`
- create `crates/own-domains-daemon/tests/linux_interfaces.rs`

Steps:
1. Write test: accepted names are `enp3s0`, `eth0`, `wlp2s0`, `wlan0`, `br0`,
   `bond0`, `vlan100`, `eno1`, `ens3`, and `wlx001122334455`. Rejected names
   are `lo`, `docker0`, `br-0123456789ab`, `veth123`, `virbr0`, `tun0`, `tap0`,
   `wg0`, `tailscale0`, `ztabc`, `podman0`, `cni0`, `flannel.1`, `cali123`,
   `lxcbr0`, `lxdbr0`, `incusbr0`, `vmnet1`, `vboxnet0`, `dummy0`, `sit0`,
   `ip6tnl0`, `gre0`, `gretap0`, `erspan0`, `ipip0`, `ifb0`, the empty string,
   a 16-byte name, and names containing `/`, a space, `;`, or `"`.
   `validate_covered_interfaces(["wlp2s0", "enp3s0"])` returns
   `["enp3s0", "wlp2s0"]`; the same function rejects an empty list, duplicate
   names, 33 names, and an ineligible name.
2. Run `cargo test -p own-domains-daemon --test linux_interfaces`; confirm it
   fails with an unresolved import for `platform::linux::interfaces`.
3. Implement `LinuxArtifactError` with `EmptyInterfaces`,
   `TooManyInterfaces(usize)`, `InvalidInterface(String)`,
   `DuplicateInterface(String)`, `InvalidUserId`, `InvalidIpOutput(String)`,
   `InvalidManifest(String)`, `InvalidCertificate(String)`, and
   `CertificateFingerprintMismatch { expected: String, actual: String }`.
   Implement
   `MAX_INTERFACES = 32`, `MAX_INTERFACE_NAME_BYTES = 15`,
   `is_eligible_interface_name`, and `validate_covered_interfaces`. Names must
   be nonempty, at most 15 bytes, and contain only ASCII alphanumeric
   characters plus `.`, `_`, and `-`. The allowlist prefixes are `en`, `eth`,
   `wl`, `wlan`, `br`, `bond`, and `vlan`; the denylist is the exact name `lo`
   plus the prefixes `docker`, `veth`, `virbr`, `tun`, `tap`, `wg`, `tailscale`,
   `zt`, `podman`, `cni`, `flannel.`, `cali`, `lxc`, `lxd`, `incus`, `vmnet`,
   `vboxnet`, `dummy`, `sit`, `ip6tnl`, `gre`, `gretap`, `erspan`, `ipip`, and
   `ifb`. Apply the denylist before the allowlist; check Docker's `br-` plus 12
   hex digits before the `br` allowlist.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-daemon --test linux_config`; confirm no
   regression.

Done when: the allowlist, denylist, byte cap, uniqueness, and sort order are
exact.

### Task 4: `ip -j` discovery and primary address

Files:
- create `crates/own-domains-daemon/src/platform/linux/ip.rs`
- create `crates/own-domains-daemon/tests/fixtures/linux/ip-link.json`
- create `crates/own-domains-daemon/tests/fixtures/linux/ip-addr.json`
- modify `crates/own-domains-daemon/src/platform/linux/mod.rs`
- create `crates/own-domains-daemon/tests/linux_ip.rs`

Steps:
1. Write fixtures with these interfaces and assert the exact result:

   | ifname | link `master` | link type | address data | expected |
   | --- | --- | --- | --- | --- |
   | `enp3s0` | none | ether | `192.168.1.21` secondary first, then `192.168.1.20` global primary, then `169.254.10.10` link-local | covered, primary `192.168.1.20` |
   | `wlp2s0` | none | ether | none | covered, primary `None` |
   | `br0` | none | bridge | `10.0.0.5` global | covered, primary `10.0.0.5` |
   | `bond0` | none | bond | none | covered, primary `None` |
   | `vlan100` | none | vlan | `10.0.0.100` global | covered, primary `10.0.0.100` |
   | `enp3s0.100` | `br0` | vlan | `10.0.0.101` global | excluded slave |
   | `enp4s0` | `bond0` | ether | `10.0.0.102` global | excluded slave |
   | `docker0` | none | bridge | `172.17.0.1` global | excluded name |
   | `br-0123456789ab` | none | bridge | `172.18.0.1` global | excluded Docker bridge |
   | `veth123` | none | veth | `169.254.1.1` link-local | excluded name |
   | `virbr0` | none | bridge | `192.168.122.1` global | excluded name |
   | `tun0` | none | tun | `10.8.0.1` global | excluded name |
   | `lo` | none | loopback | `127.0.0.1` loopback | excluded name |

   `discover_covered_interfaces` returns the sorted names
   `["bond0", "br0", "enp3s0", "vlan100", "wlp2s0"]` with primary addresses
   `None`, `10.0.0.5`, `192.168.1.20`, `10.0.0.100`, and `None`. Assert the
   alias `192.168.1.21` is never selected. Assert malformed JSON returns an
   error.
2. Run `cargo test -p own-domains-daemon --test linux_ip`; confirm it fails
   because `platform::linux::ip` does not exist.
3. Implement `LinuxInterface { name, primary_ipv4 }` and
   `discover_covered_interfaces(link_json, addr_json)`. Parse
   `/usr/sbin/ip -j link show` and `/usr/sbin/ip -j -4 addr show` shapes with
   serde. Skip any link with a `master` or a `link_type` of `loopback`,
   `dummy`, `veth`, `tun`, or `wireguard`; skip ineligible names; select the
   first `family == "inet"`, `scope == "global"`, non-secondary IPv4 address;
   ignore loopback, link-local, multicast, and unspecified addresses; sort by
   name.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-daemon --test linux_interfaces`; confirm no
   regression.

Done when: the fixtures produce the exact covered list, primary addresses, and
slave exclusions.

### Task 5: nftables exact rendering

Files:
- create `crates/own-domains-daemon/src/platform/linux/nftables.rs`
- create `crates/own-domains-daemon/tests/fixtures/linux/nftables.conf.expected`
- modify `crates/own-domains-daemon/src/platform/linux/mod.rs`
- create `crates/own-domains-daemon/tests/linux_nftables.rs`

Steps:
1. Write the fixture and test. For interfaces `["wlp2s0", "enp3s0"]` and UID
   1000, the fixture is exactly:

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

   The fixture ends with a single trailing newline. Assert the fixture contains
   no `192.168`, no `0.0.0.0`, no output-chain port 53 rule, and no filter
   chain. Assert `render_nftables` returns errors for an empty list, duplicates,
   an ineligible name, 33 names, and UID 0.
2. Run `cargo test -p own-domains-daemon --test linux_nftables`; confirm it
   fails because `platform::linux::nftables` does not exist.
3. Implement `render_nftables(interfaces, user_id)`: reject UID 0, call
   `validate_covered_interfaces`, and render the fixture shape exactly with
   fixed high ports 5354/8080/8443, priorities -110/-100, link-local input
   exclusion, and loopback/link-local output exclusions. Define
   `pub const NFTABLES_TABLE: &str = "ip own_domains";` and use it in the
   header/table lines so Task 8 can reuse it. Do not accept or interpolate a LAN
   address.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-daemon --test linux_interfaces --test linux_ip`;
   confirm no regression.

Done when: the ruleset is byte-exact, sorted, capped, and contains no LAN
address.

### Task 6: systemd unit exact rendering

Files:
- create `crates/own-domains-daemon/src/platform/linux/systemd.rs`
- create `crates/own-domains-daemon/tests/fixtures/linux/own-domains-nftables.service.expected`
- create `crates/own-domains-daemon/tests/fixtures/linux/own-domains-resolver.service.expected`
- modify `crates/own-domains-daemon/src/platform/linux/mod.rs`
- create `crates/own-domains-daemon/tests/linux_systemd.rs`

Steps:
1. Write both fixtures from the design's unit blocks verbatim, each ending with
   a single trailing newline, and assert exact equality. Assert the nftables
   unit has
   `After=network-pre.target nftables.service`, `Before=network.target`,
   `ExecStart=/usr/sbin/nft -f /etc/own-domains/nftables.conf`, and
   `ExecStop=-/usr/sbin/nft delete table ip own_domains`. Assert the resolver
   unit has `After=systemd-resolved.service`, `PartOf=systemd-resolved.service`,
   `ExecStart=-/usr/sbin/ip link add own-domains0 type dummy`,
   `ExecStart=/usr/bin/resolvectl dns own-domains0 127.0.0.1:5354`,
   `ExecStart=/usr/bin/resolvectl domain own-domains0 "~own"`,
   `ExecStart=/usr/bin/resolvectl dnssec own-domains0 no`,
   `ExecStart=/usr/bin/resolvectl dnsovertls own-domains0 no`,
   `ExecStart=/usr/bin/resolvectl default-route own-domains0 no`,
   `ExecStop=-/usr/bin/resolvectl revert own-domains0`, and
   `ExecStop=-/usr/sbin/ip link delete own-domains0`. Assert both units have
   `Type=oneshot`, `RemainAfterExit=yes`, `NoNewPrivileges=yes`,
   `PrivateTmp=yes`, `ProtectHome=yes`, `ProtectSystem=full`,
   `ProtectKernelTunables=yes`, `ProtectControlGroups=yes`,
   `CapabilityBoundingSet=CAP_NET_ADMIN`, and no LAN address, `$`, or `{`
   substitution.
2. Run `cargo test -p own-domains-daemon --test linux_systemd`; confirm it
   fails because `platform::linux::systemd` does not exist.
3. Implement `NFTABLES_UNIT_NAME = "own-domains-nftables.service"`,
   `RESOLVER_UNIT_NAME = "own-domains-resolver.service"`,
   `render_nftables_unit`, and `render_resolver_unit` as static `concat!`
   strings copied from the fixtures.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-daemon --test linux_nftables`; confirm no
   regression.

Done when: both units are byte-exact, static, and free of user input.

### Task 7: Trust certificate fingerprint

Files:
- create `crates/own-domains-daemon/src/platform/linux/trust.rs`
- modify `crates/own-domains-daemon/src/platform/linux/mod.rs`
- create `crates/own-domains-daemon/tests/linux_trust.rs`

Steps:
1. Write test using
   `include_bytes!("../../../tests/fixtures/sidecars/certificate.pem")`:
   `certificate_sha256` returns
   `15a63910f999a5e2e8493ccf7e57b0c7c85fccc70ee8eb62d8f309a4e013be6f`;
   decoding the PEM to DER and passing the DER to `certificate_sha256` returns
   the same value; `verify_certificate_fingerprint` accepts that value and
   rejects a different one; empty, malformed-base64, non-certificate, and
   non-SEQUENCE inputs return errors; the fingerprint is 64 lowercase hex
   characters.
2. Run `cargo test -p own-domains-daemon --test linux_trust`; confirm it fails
   because `platform::linux::trust` does not exist.
3. Implement `certificate_der`, `certificate_sha256`, and
   `verify_certificate_fingerprint` with the existing `base64` and `sha2`
   dependencies. Strip the PEM header/footer, reject multiple or missing
   blocks, remove ASCII whitespace (including CRLF) from the body,
   base64-decode it, and require a DER SEQUENCE (`0x30`) for both raw DER and
   PEM-decoded content. Reject an expected fingerprint that is not 64 lowercase
   hex.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-daemon --test macos_setup`; confirm no
   regression.

Done when: PEM and DER inputs produce the same lowercase SHA-256 fingerprint and
mismatches are refused.

### Task 8: Linux manifest exact rendering and validation

Files:
- create `crates/own-domains-daemon/src/platform/linux/manifest.rs`
- create `crates/own-domains-daemon/tests/fixtures/linux/install-manifest.json.expected`
- modify `crates/own-domains-daemon/src/platform/linux/mod.rs`
- create `crates/own-domains-daemon/tests/linux_manifest.rs`

Steps:
1. Write test: build the manifest for user `alice`, user ID 1000, backup
   directory `/etc/own-domains/backups`, interfaces `["wlp2s0", "enp3s0"]`,
   sidecars schema 1 with CoreDNS 1.14.7 and Caddy 2.11.7 and digests
   `"a".repeat(64)` and `"b".repeat(64)`, the CA fingerprint from Task 7, and
   artifacts loaded with
   `include_str!("fixtures/linux/nftables.conf.expected")`,
   `include_str!("fixtures/linux/own-domains-nftables.service.expected")`,
   `include_str!("fixtures/linux/own-domains-resolver.service.expected")`, and
   `include_bytes!("../../../tests/fixtures/sidecars/certificate.pem")`.
   Compute the fixture's artifact SHA-256 values independently with
   `shasum -a 256` over those fixture files; do not copy them from renderer
   output. Assert `render_linux_manifest` equals
   `include_str!("fixtures/linux/install-manifest.json.expected")`
   byte-for-byte, and that `parse_linux_manifest` round-trips and validates it.
   Assert validation rejects an extra artifact path, an unexpected path such as
   `/etc/passwd`, a wrong kind, mode `0o600`, owner or group `1000`, an
   uppercase or short SHA-256, empty/duplicate/unsorted/33/invalid interfaces,
   a backup directory other than `/etc/own-domains/backups`, a table other than
   `ip own_domains`, a link other than `own-domains0`, unit names other than
   the exact pair, `platform` other than `linux`, schema 2, host integration
   version 2, user ID 0, an empty or invalid user name, a sidecar schema other
   than 1, an uppercase or short sidecar digest, and an uppercase or short CA
   fingerprint.
2. Run `cargo test -p own-domains-daemon --test linux_manifest`; confirm it
   fails because `platform::linux::manifest` does not exist.
3. Implement `BACKUP_DIRECTORY = "/etc/own-domains/backups"` and
   `RESOLVER_LINK = "own-domains0"` in `manifest.rs` (reuse
   `nftables::NFTABLES_TABLE`, `systemd::NFTABLES_UNIT_NAME`, and
   `systemd::RESOLVER_UNIT_NAME` for the fixed table and unit fields),
   `LinuxInstallManifest`, `LinuxManifestArtifact`, `LinuxArtifactKind`,
   `LinuxArtifactContent` (`path: &str`, `kind: LinuxArtifactKind`,
   `content: &[u8]`), `build_linux_manifest(user_name, user_id, interfaces,
   sidecars, ca_fingerprint_sha256, artifacts) -> LinuxInstallManifest`,
   `render_linux_manifest(manifest) -> Result<String, LinuxArtifactError>`,
   `parse_linux_manifest(bytes) -> Result<LinuxInstallManifest,
   LinuxArtifactError>`, and `validate_linux_manifest(manifest) ->
   Result<(), LinuxArtifactError>`. Derive `Clone, Debug,
   Deserialize, Eq, PartialEq, Serialize` and apply
   `#[serde(rename_all = "camelCase")]` to `LinuxInstallManifest`,
   `LinuxManifestArtifact`, and `LinuxArtifactKind`; use `String` for paths,
   names, and hashes, `u32` for versions, modes, and IDs, `Vec<String>` for
   interfaces and units, and `SidecarManifest` for sidecars.
   `LinuxArtifactKind` variants are `Nftables`, `SystemdUnit`, and
   `TrustCertificate`, serialized as `nftables`, `systemdUnit`, and
   `trustCertificate`. Declare the top-level fields in the design's order:
   schemaVersion, platform, hostIntegrationVersion, userName, userId,
   backupDirectory, interfaces, artifacts, sidecars, caFingerprintSha256,
   nftablesTable, resolverLink, systemdUnits. Declare artifact fields in the
   order path, sha256, mode, ownerId, groupId, kind. Use schema 1, platform
   `linux`, host integration version 1, the four fixed artifact paths
   `/etc/own-domains/nftables.conf`,
   `/etc/systemd/system/own-domains-nftables.service`,
   `/etc/systemd/system/own-domains-resolver.service`, and
   `/usr/local/share/ca-certificates/own-domains.crt` in the order nftables,
   nftables unit, resolver unit, trust certificate, mode `0o644`, owner/group
   0, sorted interfaces, and pretty JSON plus a trailing newline. Validate the
   user name as nonempty, at most 32 bytes, and ASCII alphanumeric plus `.`,
   `_`, and `-`; validate the sidecar schema/version/digest shape. Compute each
   artifact SHA-256 from its content; validate hashes as 64 lowercase hex.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-daemon --test linux_nftables --test linux_systemd --test linux_trust`;
   confirm no regression.

Done when: the manifest is byte-exact and every invalid shape is rejected.

### Task 9: Platform facade and Linux backend skeleton

Files:
- create `crates/own-domains-daemon/src/platform/linux/runtime.rs`
- modify `crates/own-domains-daemon/src/platform/linux/mod.rs`
- modify `crates/own-domains-daemon/src/platform/mod.rs`
- create `crates/own-domains-daemon/tests/linux_platform.rs`

Steps:
1. Write test with `Settings::default()`: `LinuxRuntimeBackend::new(&state_dir)`
   reports one `setupRequired` diagnostic with `status ==
   DiagnosticStatus::Error`, `!message.is_empty()`, and `remedy.is_some()` when
   the state directory has no `install-manifest.json`; one
   `setupManifestInvalid` diagnostic with the same status, message, and remedy
   for corrupt JSON; one `setupManifestInvalid`
   diagnostic for a valid manifest whose `platform` is `macos`; no diagnostics
   for a valid manifest. `inspect` returns `RuntimeStatus::Stopped`, `stop`
   returns `Ok(())`, and `apply` returns `RuntimeError::Unavailable`. On Linux,
   assert `platform::runtime_backend(state, None, None, false)` returns the
   Linux backend by observing `setupRequired` on an empty state directory;
   assert `platform::runtime_backend(state, None, None, true)` returns the
   unavailable backend by observing no diagnostics.
2. Run `cargo test -p own-domains-daemon --test linux_platform`; confirm it
   fails because `platform::linux::runtime` and the facade selection do not
   exist.
3. Implement `LinuxRuntimeBackend` with
   `pub fn new(state_dir: &Path) -> Result<Self, RuntimeError>` holding only the
   absolute state directory; resolve a relative state directory against
   `current_dir()` before storing it. `diagnose` reads `install-manifest.json`,
   calls `parse_linux_manifest`, and returns `Ok(vec![...])` with
   `setupRequired` for a missing file or `setupManifestInvalid` for a read,
   parse, or validation failure. `apply` returns `RuntimeError::Unavailable`;
   `stop` and `inspect` are idempotent. Add `pub fn runtime_backend(state_dir,
   resources, sidecars, settings_only)` returning `UnavailableRuntimeBackend`
   for `settings_only` and the Linux backend otherwise; the skeleton ignores
   `resources` and `sidecars`, which the Milestone 2 backend consumes. In
   `platform/mod.rs`, select `linux::runtime_backend` on Linux, keep the macOS
   branch unchanged, and keep the unavailable branch for every other target.
   Do not add cfg branches inside `platform::linux`.
4. Run the test; confirm it passes.
5. Run `cargo test -p own-domains-daemon --test platform_contract --test macos_runtime`;
   confirm no regression.

Done when: the facade selects the Linux skeleton and its manifest gate reports
exactly `setupRequired` or `setupManifestInvalid`.

### Task 10: Full verification

Steps:
1. Run `cargo fmt --all`; then `cargo fmt --all -- --check`.
2. Run `cargo test --workspace --locked`; all green.
3. On Ubuntu 24.04, run the Linux rendering/backend command and
   `cargo check --workspace --all-targets --locked`; all green.
4. Run `cargo clippy --workspace --all-targets --locked -- -D warnings`; clean.
5. Confirm `git diff --stat` shows no changes to
   `crates/own-domains-core/src/dns.rs` or
   `crates/own-domains-core/src/proxy.rs` beyond the prerequisite Windows
   tasks, and no macOS fixture changes.
6. Do not commit; report the final diff and verification output for review.

Done when: everything above passes with output shown.
