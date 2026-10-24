# Dynamic PF and Schema 2 macOS Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the selected LAN address and interface address from privileged macOS artifacts, generate dynamic `(if:0)` PF rules for every eligible interface present at setup, migrate the install manifest to schema 2, and make ordinary address/covered-interface changes unprivileged.

**Architecture:** `MacOsSetupOptions` carries a discovered list of eligible present interfaces and their primary IPv4 addresses. `build_host_setup_plan` renders one set of dynamic rules per covered interface and writes schema 2 with `interfaces: string[]`, never the selected address. The inspector validates schema 2, re-renders the expected anchor, discovers current interfaces, and distinguishes inactive addresses from uncovered interfaces. The CLI discovers interfaces through fixed `ifconfig` commands and validates that the selected address is the selected interface's primary address.

**Tech Stack:** Rust 2024, Cargo workspace, existing `own-domains-daemon`, `own-domains-cli`, and `own-domains-core` crates; serde/serde_json; React + Vitest for the frontend assertion. No production dependencies are added.

**Spec:** `docs/superpowers/specs/2026-10-23-network-independent-macos-integration-design.md`

## Global Constraints

- One-time privileged setup remains the only root operation; no helper, root socket, or Network Extension is introduced.
- The privileged anchor and schema 2 manifest must contain no selected LAN address.
- PF rules use `(<interface>:0)` and remain scoped to each interface's own primary IPv4 address.
- Only eligible interfaces present at setup are covered. `en*` and `bridge*` are eligible; loopback, `utun*`, `awdl*`, `llw*`, `anpi*`, `gif*`, `stf*`, and other virtual interfaces are not.
- Names must be nonempty ASCII alphanumeric, at most 15 bytes, and validated before being passed to `ifconfig` or rendered into PF rules.
- The selected LAN address must be the selected interface's primary IPv4 address. Alias addresses are rejected.
- A schema 1 manifest remains readable for setup and uninstall. Setup and uninstall validate its structure, artifact paths, backup location, ownership, and mode before acting. Runtime inspection reports `setupRequired` with the remedy “Run setup once to enable network-independent routing.”
- `lanIntegrationDrift` is removed. Address changes must not report a host-integration failure.
- Shared platform contracts and the protocol stay unchanged; all PF, resolver, manifest, and interface-discovery details stay under the macOS implementation.
- Root `pnpm test` currently collects `.worktrees/tauri-gui-migration` and is pre-existing red. Use targeted Vitest files and the documented full Rust/frontend gates below; do not treat the root collection failure as a feature failure.
- Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` before completing the Rust work.

## Review Focus

- A selected address is an alias, is inactive, or belongs to a different interface: setup must reject it, inspection must return `lanAddressInactive` or `interfaceCoverageMissing` without duplicate diagnostics, and runtime start must fail before replacing the previous applied configuration.
- A new hardware interface appears after setup and its address is selected: inspection must not silently widen PF coverage; it must return `interfaceCoverageMissing` with a one-time setup remedy. An unused new interface does not invalidate an otherwise active covered selection.
- A covered interface temporarily has no IPv4 address or disappears: the anchor must remain valid and inert rather than failing setup or blocking unrelated address changes.
- A schema 1 manifest, unknown schema, corrupt JSON, or tampered artifact hash: setup and uninstall must remain safe, and runtime inspection must ask for one setup rather than reporting address drift.
- PF or manifest tampering: the inspector must verify the exact dynamic anchor, manifest artifact hashes/modes, PF composition, and active anchor; a discovery command failure must report `inspectionFailed`, not falsely claim `lanAddressInactive`.
- Coverage precedence: if the selected address belongs to an uncovered eligible interface, report `interfaceCoverageMissing` even when every covered interface is temporarily addressless. Otherwise report `lanAddressInactive`.

---

### Task 1: Dynamic interface model, anchor rendering, and schema 2 serialization

**Files:**
- Modify: `crates/own-domains-daemon/src/platform/macos/host.rs`
- Modify: `crates/own-domains-daemon/src/platform/macos/mod.rs`
- Test: `crates/own-domains-daemon/tests/macos_host.rs`

**Interfaces:**
- Consumes: existing `CommandRunner`, `HostCommand`, `HostCommandOutput`, `HostSetupPlan`, `HostSetupError`, `ManifestArtifact`, `SidecarManifest`.
- Produces: `NetworkInterface { name: String, primary_ipv4: Option<Ipv4Addr> }`, `discover_network_interfaces(&dyn CommandRunner) -> Result<Vec<NetworkInterface>, String>`, `MacOsSetupOptions.covered_interfaces: Vec<NetworkInterface>`, schema 2 `HostInstallManifest { schema_version, user_name, user_id, backup_directory, interfaces, artifacts, sidecars }`, and `render_anchor_for(&[String]) -> String`.

- [ ] **Step 1: Replace the setup-options and manifest tests in `tests/macos_host.rs` with the new contract**

Add `NetworkInterface` and `discover_network_interfaces` to the import list. Replace the `options` constructor's `interface_addresses` field with:

```rust
        covered_interfaces: vec![NetworkInterface {
            name: "en0".into(),
            primary_ipv4: Some(Ipv4Addr::new(192, 168, 1, 20)),
        }],
```

Replace `renders_exact_resolver_pf_manifest_and_fixed_commands` with a two-interface version:

```rust
#[test]
fn renders_exact_resolver_dynamic_anchor_and_schema_two_manifest() {
    let mut options = options(MacOsArchitecture::Arm64);
    options.covered_interfaces.push(NetworkInterface {
        name: "en1".into(),
        primary_ipv4: Some(Ipv4Addr::new(192, 168, 1, 21)),
    });
    let plan = build_host_setup_plan(&options).unwrap();
    let resolver = plan.artifact("/etc/resolver/own").unwrap();
    let anchor = plan.artifact("/etc/pf.anchors/dev.owndomains").unwrap();
    let pf_block = plan.artifact("/etc/pf.conf").unwrap();

    assert_eq!(resolver.content, "nameserver 127.0.0.1\nport 5354\n");
    assert_eq!(resolver.mode, 0o644);
    assert_eq!(resolver.owner_id, 0);
    assert_eq!(
        anchor.content,
        concat!(
            "rdr pass on lo0 inet proto udp from any to 127.0.0.1 port 53 -> 127.0.0.1 port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 53 -> 127.0.0.1 port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port 8080\n",
            "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 443 -> 127.0.0.1 port 8443\n",
            "rdr pass on lo0 inet proto udp from any to (en0:0) port 53 -> (en0:0) port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to (en0:0) port 53 -> (en0:0) port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to (en0:0) port 80 -> (en0:0) port 8080\n",
            "rdr pass on lo0 inet proto tcp from any to (en0:0) port 443 -> (en0:0) port 8443\n",
            "rdr pass on en0 inet proto udp from any to (en0:0) port 53 -> (en0:0) port 5354\n",
            "rdr pass on en0 inet proto tcp from any to (en0:0) port 53 -> (en0:0) port 5354\n",
            "rdr pass on en0 inet proto tcp from any to (en0:0) port 80 -> (en0:0) port 8080\n",
            "rdr pass on en0 inet proto tcp from any to (en0:0) port 443 -> (en0:0) port 8443\n",
            "rdr pass on lo0 inet proto udp from any to (en1:0) port 53 -> (en1:0) port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to (en1:0) port 53 -> (en1:0) port 5354\n",
            "rdr pass on lo0 inet proto tcp from any to (en1:0) port 80 -> (en1:0) port 8080\n",
            "rdr pass on lo0 inet proto tcp from any to (en1:0) port 443 -> (en1:0) port 8443\n",
            "rdr pass on en1 inet proto udp from any to (en1:0) port 53 -> (en1:0) port 5354\n",
            "rdr pass on en1 inet proto tcp from any to (en1:0) port 53 -> (en1:0) port 5354\n",
            "rdr pass on en1 inet proto tcp from any to (en1:0) port 80 -> (en1:0) port 8080\n",
            "rdr pass on en1 inet proto tcp from any to (en1:0) port 443 -> (en1:0) port 8443\n",
        )
    );
    assert!(!anchor.content.contains("192.168.1.20"));
    assert!(!anchor.content.contains("192.168.1.21"));
    assert_eq!(
        pf_block.content,
        concat!(
            "# BEGIN OWN DOMAINS\n",
            "rdr-anchor \"dev.owndomains\"\n",
            "load anchor \"dev.owndomains\" from \"/etc/pf.anchors/dev.owndomains\"\n",
            "# END OWN DOMAINS\n",
        )
    );
    assert!(pf_block.managed_block);
    assert_eq!(plan.manifest.schema_version, 2);
    assert_eq!(plan.manifest.user_name, "kien");
    assert_eq!(plan.manifest.interfaces, vec!["en0".to_string(), "en1".to_string()]);
    let manifest = serde_json::to_value(&plan.manifest).unwrap();
    assert_eq!(manifest["schemaVersion"], 2);
    assert_eq!(manifest["interfaces"], serde_json::json!(["en0", "en1"]));
    assert!(manifest.get("lanAddress").is_none());
    assert!(manifest.get("interface").is_none());
    assert!(
        plan.manifest
            .artifacts
            .iter()
            .all(|artifact| artifact.sha256.len() == 64)
    );
    assert_eq!(plan.commands.iter().map(|command| (command.program.to_string_lossy().into_owned(), command.args.clone())).collect::<Vec<_>>(), vec![
        ("/usr/bin/security".into(), vec!["add-trusted-cert", "-d", "-r", "trustRoot", "-k", "/Library/Keychains/System.keychain", "/Users/kien/Library/Application Support/OwnDomains/caddy-data/pki/authorities/local/root.crt"].into_iter().map(String::from).collect()),
        ("/sbin/pfctl".into(), vec!["-nf", "/etc/pf.conf"].into_iter().map(String::from).collect()),
        ("/sbin/pfctl".into(), vec!["-f", "/etc/pf.conf"].into_iter().map(String::from).collect()),
        ("/sbin/pfctl".into(), vec!["-a", "dev.owndomains", "-f", "/etc/pf.anchors/dev.owndomains"].into_iter().map(String::from).collect()),
        ("/sbin/pfctl".into(), vec!["-E"].into_iter().map(String::from).collect()),
    ]);
}
```

Add a test proving that a present interface without a primary IPv4 address is still covered and rendered:

```rust
#[test]
fn covers_present_interfaces_without_a_primary_ipv4_address() {
    let mut options = options(MacOsArchitecture::Arm64);
    options.covered_interfaces.push(NetworkInterface {
        name: "bridge3".into(),
        primary_ipv4: None,
    });
    let plan = build_host_setup_plan(&options).unwrap();
    let anchor = plan.artifact("/etc/pf.anchors/dev.owndomains").unwrap();

    assert!(anchor.content.contains("(bridge3:0)"));
    assert_eq!(
        plan.manifest.interfaces,
        vec!["bridge3".to_string(), "en0".to_string()]
    );
}
```

Add coverage and alias rejection to `rejects_untrusted_setup_inputs_and_duplicate_artifacts`:

```rust
    let mut empty = options(MacOsArchitecture::Arm64);
    empty.covered_interfaces.clear();
    assert!(
        build_host_setup_plan(&empty)
            .unwrap_err()
            .to_string()
            .contains("eligible")
    );

    let mut uncovered = options(MacOsArchitecture::Arm64);
    uncovered.covered_interfaces[0].name = "en1".into();
    assert!(
        build_host_setup_plan(&uncovered)
            .unwrap_err()
            .to_string()
            .contains("not covered")
    );

    let mut duplicate = options(MacOsArchitecture::Arm64);
    duplicate
        .covered_interfaces
        .push(duplicate.covered_interfaces[0].clone());
    assert!(
        build_host_setup_plan(&duplicate)
            .unwrap_err()
            .to_string()
            .contains("duplicate network interface")
    );

    let mut ineligible = options(MacOsArchitecture::Arm64);
    ineligible.covered_interfaces.push(NetworkInterface {
        name: "utun0".into(),
        primary_ipv4: None,
    });
    assert!(
        build_host_setup_plan(&ineligible)
            .unwrap_err()
            .to_string()
            .contains("interface")
    );

    let mut oversized = options(MacOsArchitecture::Arm64);
    oversized.covered_interfaces[0].name = "en00000000000000".into();
    oversized.interface = "en00000000000000".into();
    assert!(
        build_host_setup_plan(&oversized)
            .unwrap_err()
            .to_string()
            .contains("interface")
    );

    let mut alias = options(MacOsArchitecture::Arm64);
    alias.covered_interfaces[0].primary_ipv4 = Some(Ipv4Addr::new(192, 168, 1, 21));
    assert!(
        build_host_setup_plan(&alias)
            .unwrap_err()
            .to_string()
            .contains("address")
    );
```

Keep the existing invalid interface, relative path, bad digest, and duplicate artifact assertions. Replace the old `mismatched` block, including its `interface_addresses` assignment, with the `alias` block above; it preserves the address-mismatch assertion.

Add a discovery test:

```rust
#[test]
fn discovers_only_present_eligible_interfaces_in_stable_order() {
    struct FakeIfconfig;

    impl CommandRunner for FakeIfconfig {
        fn run(&self, command: &HostCommand) -> Result<HostCommandOutput, String> {
            assert_eq!(command.program, PathBuf::from("/sbin/ifconfig"));
            assert_eq!(command.timeout, Duration::from_secs(2));
            let output = if command.args.len() == 1 && command.args[0] == "-l" {
                "lo0 en1 bridge3 utun0 en0 awdl0 en8 en9".to_string()
            } else {
                match command.args.first().map(String::as_str) {
                    Some("en0") => {
                        "en0: flags=8863\n\tinet 192.168.1.20 netmask 0xffffff00\n".to_string()
                    }
                    Some("en1") => {
                        "en1: flags=8863\n\tinet 192.168.1.21 netmask 0xffffff00\n".to_string()
                    }
                    Some("bridge3") => "bridge3: flags=8863\n".to_string(),
                    Some("en8") => concat!(
                        "en8: flags=8863\n",
                        "\tinet 192.0.2.210 netmask 0xffffff00\n",
                        "\tinet 192.0.2.211 netmask 0xffffff00 alias\n",
                    )
                    .to_string(),
                    // An alias is not a primary address. An interface with only
                    // an alias is covered but currently addressless.
                    Some("en9") => {
                        "en9: flags=8863\n\tinet 192.0.2.212 netmask 0xffffff00 alias\n"
                            .to_string()
                    }
                    other => panic!("unexpected interface {other:?}"),
                }
            };
            Ok(HostCommandOutput {
                status: 0,
                stdout: output.into_bytes(),
                stderr: vec![],
            })
        }
    }

    let interfaces = discover_network_interfaces(&FakeIfconfig).unwrap();
    assert_eq!(
        interfaces,
        vec![
            NetworkInterface {
                name: "bridge3".into(),
                primary_ipv4: None,
            },
            NetworkInterface {
                name: "en0".into(),
                primary_ipv4: Some(Ipv4Addr::new(192, 168, 1, 20)),
            },
            NetworkInterface {
                name: "en1".into(),
                primary_ipv4: Some(Ipv4Addr::new(192, 168, 1, 21)),
            },
            NetworkInterface {
                name: "en8".into(),
                primary_ipv4: Some(Ipv4Addr::new(192, 0, 2, 210)),
            },
            NetworkInterface {
                name: "en9".into(),
                primary_ipv4: None,
            },
        ]
    );
}
```

- [ ] **Step 2: Run the focused test and verify it fails to compile**

Run: `cargo test -p own-domains-daemon --test macos_host`

Expected: FAIL because `NetworkInterface`, `covered_interfaces`, `discover_network_interfaces`, and manifest `interfaces` do not exist.

- [ ] **Step 3: Add the interface model and constants to `host.rs`**

Add beside the existing constants:

```rust
const RESOLVER_CONTENT: &str = "nameserver 127.0.0.1\nport 5354\n";
const PF_CONF_BLOCK: &str = concat!(
    "# BEGIN OWN DOMAINS\n",
    "rdr-anchor \"dev.owndomains\"\n",
    "load anchor \"dev.owndomains\" from \"/etc/pf.anchors/dev.owndomains\"\n",
    "# END OWN DOMAINS\n",
);
```

Add beside `MacOsSetupOptions`:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkInterface {
    pub name: String,
    pub primary_ipv4: Option<Ipv4Addr>,
}
```

Change `MacOsSetupOptions` to remove `interface_addresses` and add:

```rust
    pub covered_interfaces: Vec<NetworkInterface>,
```

Add error variants to `HostSetupError`:

```rust
    #[error("no eligible network interfaces were found")]
    EmptyInterfaceCoverage,
    #[error("duplicate network interface: {0}")]
    DuplicateInterface(String),
    #[error("selected interface is not covered: {0}")]
    InterfaceNotCovered(String),
```

Change the existing `AddressMismatch` message to describe the primary-address rule:

```rust
    #[error("LAN address {address} is not the primary IPv4 address of interface {interface}")]
    AddressMismatch {
        interface: String,
        address: Ipv4Addr,
    },
```

- [ ] **Step 4: Add discovery and validation helpers**

Add after `HostSetupError`:

```rust
pub fn discover_network_interfaces(
    commands: &dyn CommandRunner,
) -> Result<Vec<NetworkInterface>, String> {
    let list = commands.run(&inspect_command("/sbin/ifconfig", ["-l"]))?;
    if list.status != 0 {
        return Err(format!(
            "ifconfig -l failed: {}",
            command_detail(&list.stderr)
        ));
    }
    let mut interfaces = Vec::new();
    for name in String::from_utf8_lossy(&list.stdout).split_ascii_whitespace() {
        if !is_eligible_interface(name) {
            continue;
        }
        let output = commands.run(&inspect_command("/sbin/ifconfig", [name]))?;
        if output.status != 0 {
            return Err(format!(
                "ifconfig {name} failed: {}",
                command_detail(&output.stderr)
            ));
        }
        interfaces.push(NetworkInterface {
            name: name.into(),
            primary_ipv4: parse_primary_ipv4(&String::from_utf8_lossy(&output.stdout)),
        });
    }
    interfaces.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(interfaces)
}

fn is_eligible_interface(name: &str) -> bool {
    (name.starts_with("en") || name.starts_with("bridge"))
        && name.len() <= 15
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn parse_primary_ipv4(output: &str) -> Option<Ipv4Addr> {
    for line in output.lines() {
        let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
        // macOS marks secondary addresses with a trailing `alias` token.
        // Only the first non-alias IPv4 address is the interface's primary.
        if fields.iter().any(|field| *field == "alias") {
            continue;
        }
        if let Some(index) = fields.iter().position(|field| *field == "inet") {
            return fields
                .get(index + 1)
                .and_then(|value| value.parse().ok());
        }
    }
    None
}

fn command_detail(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .take(240)
        .collect()
}
```

- [ ] **Step 5: Replace schema 1 manifest fields with schema 2**

Replace `HostInstallManifest` with:

```rust
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInstallManifest {
    pub schema_version: u32,
    pub user_name: String,
    pub user_id: u32,
    pub backup_directory: PathBuf,
    pub interfaces: Vec<String>,
    pub artifacts: Vec<ManifestArtifact>,
    pub sidecars: SidecarManifest,
}
```

- [ ] **Step 6: Update plan construction and anchor rendering**

In `build_host_setup_plan`, replace the artifacts and manifest setup with:

```rust
    let mut interfaces = options
        .covered_interfaces
        .iter()
        .map(|interface| interface.name.clone())
        .collect::<Vec<_>>();
    interfaces.sort();
    let anchor = render_anchor_for(&interfaces);
    let mut artifacts = vec![
        system_artifact("/etc/resolver/own", RESOLVER_CONTENT, false),
        system_artifact(ANCHOR_PATH, &anchor, false),
        system_artifact("/etc/pf.conf", PF_CONF_BLOCK, true),
    ];
    let manifest = HostInstallManifest {
        schema_version: 2,
        user_name: options.user_name.clone(),
        user_id: options.user_id,
        backup_directory: options.backup_directory.clone(),
        interfaces,
        artifacts: artifacts
            .iter()
            .map(|artifact| ManifestArtifact {
                path: artifact.path.clone(),
                sha256: sha256(artifact.content.as_bytes()),
                mode: artifact.mode,
                owner_id: artifact.owner_id,
                managed_block: artifact.managed_block,
            })
            .collect(),
        sidecars: options.sidecars.clone(),
    };
```

Keep the existing code below that pushes the serialized manifest artifact onto `artifacts`, builds `commands`, constructs `HostSetupPlan`, and validates it.

Delete the old `render_anchor` helper and replace `render_anchor_for` with:

```rust
fn render_anchor_for(interfaces: &[String]) -> String {
    const SERVICES: [(&str, u16, u16); 4] = [
        ("udp", 53, 5354),
        ("tcp", 53, 5354),
        ("tcp", 80, 8080),
        ("tcp", 443, 8443),
    ];
    let mut output = String::new();
    for (protocol, source, target) in SERVICES {
        output.push_str(&format!(
            "rdr pass on lo0 inet proto {protocol} from any to 127.0.0.1 port {source} -> 127.0.0.1 port {target}\n"
        ));
    }
    for interface in interfaces {
        for (protocol, source, target) in SERVICES {
            // Redirect to the interface's current address instead of loopback.
            output.push_str(&format!(
                "rdr pass on lo0 inet proto {protocol} from any to ({interface}:0) port {source} -> ({interface}:0) port {target}\n"
            ));
        }
        for (protocol, source, target) in SERVICES {
            output.push_str(&format!(
                "rdr pass on {interface} inet proto {protocol} from any to ({interface}:0) port {source} -> ({interface}:0) port {target}\n"
            ));
        }
    }
    output
}
```

Update `validate_options` to:

```rust
    if !is_eligible_interface(&options.interface) {
        return Err(HostSetupError::InvalidInterface(options.interface.clone()));
    }
    if options.covered_interfaces.is_empty() {
        return Err(HostSetupError::EmptyInterfaceCoverage);
    }
    let mut interface_names = HashSet::new();
    for interface in &options.covered_interfaces {
        if !is_eligible_interface(&interface.name) {
            return Err(HostSetupError::InvalidInterface(interface.name.clone()));
        }
        if !interface_names.insert(interface.name.as_str()) {
            return Err(HostSetupError::DuplicateInterface(interface.name.clone()));
        }
    }
    let selected = options
        .covered_interfaces
        .iter()
        .find(|interface| interface.name.as_str() == options.interface.as_str())
        .ok_or_else(|| HostSetupError::InterfaceNotCovered(options.interface.clone()))?;
    if options.lan_address.is_unspecified()
        || options.lan_address.is_loopback()
        || options.lan_address.is_multicast()
        || selected.primary_ipv4 != Some(options.lan_address)
    {
        return Err(HostSetupError::AddressMismatch {
            interface: options.interface.clone(),
            address: options.lan_address,
        });
    }
```

Replace the old `options.interface_addresses.contains(&options.lan_address)` validation with this block.

- [ ] **Step 7: Export the new types and run the focused test**

Update `mod.rs`'s `host` export list to include `NetworkInterface` and `discover_network_interfaces`:

```rust
pub use host::{
    CommandRunner, HostArtifact, HostCommand, HostCommandOutput, HostFileKind, HostFileMetadata,
    HostFileSystem, HostInstallManifest, HostIntegrationInspector, HostSetupError, HostSetupPlan,
    MacOsSetupOptions, ManifestArtifact, NetworkInterface, SidecarManifest, SidecarSpec,
    SystemCommandRunner, SystemHostFileSystem, build_host_setup_plan,
    discover_network_interfaces, validate_host_setup_plan,
};
```

Run: `cargo test -p own-domains-daemon --test macos_host`

Expected: PASS. Other tests still fail to compile until Tasks 2-4 update their constructors and inspector fixtures.

- [ ] **Step 8: Commit**

```bash
git add crates/own-domains-daemon/src/platform/macos/host.rs crates/own-domains-daemon/src/platform/macos/mod.rs crates/own-domains-daemon/tests/macos_host.rs
git commit -m "feat: render dynamic per-interface pf rules"
```

---

### Task 2: Schema 2 setup plans, schema 1 migration, and uninstall

**Files:**
- Modify: `crates/own-domains-daemon/src/platform/macos/host.rs`
- Modify: `crates/own-domains-daemon/src/platform/macos/setup.rs`
- Test: `crates/own-domains-daemon/tests/macos_setup.rs`

**Interfaces:**
- Consumes: `NetworkInterface`, `HostInstallManifest`, schema 2 plan output from Task 1.
- Produces: `LegacyHostInstallManifest`, `InstalledHostManifest`, `parse_host_install_manifest(&[u8]) -> Result<InstalledHostManifest, String>`, and setup/uninstall helpers that read either schema version through `backup_directory()`, `artifacts()`, `sidecars()`, and `user_id()`.

- [ ] **Step 1: Update the setup test fixtures and add migration tests**

Add `HostSetupPlan`, `ManifestArtifact`, and `NetworkInterface` to the daemon test imports, and add `use sha2::{Digest, Sha256};`. Change `setup_options` to:

```rust
fn setup_options(address: Ipv4Addr) -> MacOsSetupOptions {
    MacOsSetupOptions {
        user_name: "kien".into(),
        user_id: 501,
        home_directory: "/Users/kien".into(),
        state_directory: "/Users/kien/Library/Application Support/OwnDomains".into(),
        resource_directory: "/Applications/Own Domains.app/Contents/Resources".into(),
        backup_directory: "/Users/kien/Library/Application Support/OwnDomains/backups/setup-1"
            .into(),
        lan_address: address,
        interface: "en0".into(),
        covered_interfaces: vec![NetworkInterface {
            name: "en0".into(),
            primary_ipv4: Some(address),
        }],
        architecture: MacOsArchitecture::Arm64,
        sidecars: SidecarManifest {
            schema_version: 1,
            coredns: SidecarSpec {
                version: "1.14.7".into(),
                sha256: "a".repeat(64),
            },
            caddy: SidecarSpec {
                version: "2.11.7".into(),
                sha256: "b".repeat(64),
            },
        },
    }
}
```

Replace `setup_is_idempotent_and_owned_updates_create_backups` with three tests:

```rust
#[test]
fn setup_is_idempotent_and_address_changes_do_not_rewrite_privileged_artifacts() {
    let first = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    let second = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 21))).unwrap();
    let anchor = first
        .artifact("/etc/pf.anchors/dev.owndomains")
        .unwrap();
    assert!(!anchor.content.contains("192.168.1.20"));
    assert_eq!(
        anchor.content,
        second
            .artifact("/etc/pf.anchors/dev.owndomains")
            .unwrap()
            .content
    );
    assert_eq!(first.manifest, second.manifest);

    let host = FakeHost::new(initial_state(), None);
    let applied = apply_setup(&first, &host).unwrap();
    assert_eq!(applied.written.len(), 4);
    assert!(
        preview_setup(&first, &host)
            .unwrap()
            .actions
            .iter()
            .all(|action| action.kind == "unchanged")
    );
    let preview = preview_setup(&second, &host).unwrap();
    assert!(
        preview.actions.iter().all(|action| action.kind == "unchanged"),
        "an address change must not rewrite the anchor or manifest"
    );
}

#[test]
fn adding_a_present_interface_updates_the_anchor_and_manifest() {
    let first = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    let host = FakeHost::new(initial_state(), None);
    apply_setup(&first, &host).unwrap();

    let mut options = setup_options(Ipv4Addr::new(192, 168, 1, 20));
    options.covered_interfaces.push(NetworkInterface {
        name: "en1".into(),
        primary_ipv4: Some(Ipv4Addr::new(192, 168, 1, 21)),
    });
    let second = build_host_setup_plan(&options).unwrap();
    let updated = apply_setup(&second, &host).unwrap();

    assert!(
        updated
            .written
            .iter()
            .any(|path| path == Path::new("/etc/pf.anchors/dev.owndomains"))
    );
    assert!(
        updated
            .backups
            .iter()
            .any(|path| path.ends_with("etc/pf.anchors/dev.owndomains"))
    );
    let anchor = host
        .snapshot()
        .files
        .get(Path::new("/etc/pf.anchors/dev.owndomains"))
        .unwrap()
        .content
        .clone();
    assert!(String::from_utf8(anchor).unwrap().contains("(en1:0)"));
    assert_eq!(second.manifest.interfaces, vec!["en0", "en1"]);
}

#[test]
fn migrates_a_schema_one_manifest_without_the_old_address() {
    let plan = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    let host = FakeHost::new(schema_one_state(&plan), None);

    apply_setup(&plan, &host).unwrap();

    let manifest = host
        .snapshot()
        .files
        .get(&plan.paths.install_manifest)
        .unwrap()
        .content
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(value["schemaVersion"], 2);
    assert_eq!(value["interfaces"], serde_json::json!(["en0"]));
    assert!(value.get("lanAddress").is_none());
    assert!(value.get("interface").is_none());
    let anchor = host
        .snapshot()
        .files
        .get(Path::new("/etc/pf.anchors/dev.owndomains"))
        .unwrap()
        .content
        .clone();
    assert!(String::from_utf8(anchor).unwrap().contains("(en0:0)"));
    let backup = host
        .snapshot()
        .files
        .get(&plan.manifest.backup_directory.join("etc/pf.anchors/dev.owndomains"))
        .unwrap()
        .content
        .clone();
    assert_eq!(backup, LEGACY_ANCHOR.as_bytes());
}
```

Add the legacy fixture helpers above the tests:

```rust
const LEGACY_ANCHOR: &str = "rdr pass on lo0 inet proto udp from any to 192.168.1.21 port 53 -> 192.168.1.21 port 5354\n";

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn schema_one_state(plan: &HostSetupPlan) -> HostState {
    let mut state = initial_state();
    state.files.insert(
        PathBuf::from("/etc/resolver/own"),
        SetupFile {
            content: b"nameserver 127.0.0.1\nport 5354\n".to_vec(),
            mode: 0o644,
            owner_id: 0,
            kind: HostFileKind::Regular,
        },
    );
    state.files.insert(
        PathBuf::from("/etc/pf.anchors/dev.owndomains"),
        SetupFile {
            content: LEGACY_ANCHOR.as_bytes().to_vec(),
            mode: 0o644,
            owner_id: 0,
            kind: HostFileKind::Regular,
        },
    );
    let pf_block = plan.artifact("/etc/pf.conf").unwrap();
    let artifacts = vec![
        ManifestArtifact {
            path: PathBuf::from("/etc/resolver/own"),
            sha256: digest(b"nameserver 127.0.0.1\nport 5354\n"),
            mode: 0o644,
            owner_id: 0,
            managed_block: false,
        },
        ManifestArtifact {
            path: PathBuf::from("/etc/pf.anchors/dev.owndomains"),
            sha256: digest(LEGACY_ANCHOR.as_bytes()),
            mode: 0o644,
            owner_id: 0,
            managed_block: false,
        },
        ManifestArtifact {
            path: PathBuf::from("/etc/pf.conf"),
            sha256: digest(pf_block.content.as_bytes()),
            mode: 0o644,
            owner_id: 0,
            managed_block: true,
        },
    ];
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "userName": plan.manifest.user_name,
        "userId": plan.manifest.user_id,
        "backupDirectory": plan.manifest.backup_directory,
        "interface": "en0",
        "lanAddress": "192.168.1.21",
        "artifacts": artifacts,
        "sidecars": plan.manifest.sidecars,
    });
    state.files.insert(
        plan.paths.install_manifest.clone(),
        SetupFile {
            content: serde_json::to_vec_pretty(&manifest).unwrap(),
            mode: 0o600,
            owner_id: 501,
            kind: HostFileKind::Regular,
        },
    );
    state
}
```

Add an uninstall test for schema 1:

```rust
#[test]
fn uninstall_reads_a_schema_one_manifest_and_restores_the_pf_backup() {
    let plan = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();
    let mut state = schema_one_state(&plan);
    state
        .files
        .get_mut(Path::new("/etc/pf.conf"))
        .unwrap()
        .content = b"current\n".to_vec();
    state.files.insert(
        plan.manifest
            .backup_directory
            .join("etc/pf.conf"),
        SetupFile {
            content: b"original\n".to_vec(),
            mode: 0o644,
            owner_id: 0,
            kind: HostFileKind::Regular,
        },
    );
    let host = FakeHost::new(state, None);

    let preview = preview_uninstall(&plan.paths, false, &host).unwrap();
    assert!(preview.actions.iter().any(|action| {
        action.path == Path::new("/etc/pf.conf") && action.kind == "restore"
    }));

    apply_uninstall(&plan.paths, false, &host).unwrap();
    let state = host.snapshot();
    assert_eq!(
        state.files.get(Path::new("/etc/pf.conf")).unwrap().content,
        b"original\n"
    );
    assert!(!state.files.contains_key(Path::new("/etc/resolver/own")));
    assert!(!state.files.contains_key(&plan.paths.install_manifest));
}

#[test]
fn setup_and_uninstall_reject_a_tampered_manifest() {
    let plan = build_host_setup_plan(&setup_options(Ipv4Addr::new(192, 168, 1, 20))).unwrap();

    let mut tampered = schema_one_state(&plan);
    let manifest = tampered
        .files
        .get_mut(&plan.paths.install_manifest)
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&manifest.content).unwrap();
    value["artifacts"][0]["path"] = "/etc/shadow".into();
    manifest.content = serde_json::to_vec_pretty(&value).unwrap();
    let error = preview_setup(&plan, &FakeHost::new(tampered, None)).unwrap_err();
    assert!(error.to_string().contains("invalid"), "{error}");

    let mut wrong_owner = schema_one_state(&plan);
    wrong_owner
        .files
        .get_mut(&plan.paths.install_manifest)
        .unwrap()
        .owner_id = 0;
    let error = preview_uninstall(&plan.paths, false, &FakeHost::new(wrong_owner, None))
        .unwrap_err();
    assert!(error.to_string().contains("ownership"), "{error}");
}
```

- [ ] **Step 2: Run the focused test and verify it fails**

Run: `cargo test -p own-domains-daemon --test macos_setup`

Expected: FAIL because `MacOsSetupOptions`/tests still mismatch and setup cannot read schema 1.

- [ ] **Step 3: Add legacy manifest parsing to `host.rs`**

Add below `HostInstallManifest`:

```rust
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LegacyHostInstallManifest {
    pub user_id: u32,
    pub backup_directory: PathBuf,
    pub artifacts: Vec<ManifestArtifact>,
    pub sidecars: SidecarManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum InstalledHostManifest {
    V1(LegacyHostInstallManifest),
    V2(HostInstallManifest),
}

impl InstalledHostManifest {
    pub(crate) fn user_id(&self) -> u32 {
        match self {
            Self::V1(manifest) => manifest.user_id,
            Self::V2(manifest) => manifest.user_id,
        }
    }

    pub(crate) fn backup_directory(&self) -> &Path {
        match self {
            Self::V1(manifest) => &manifest.backup_directory,
            Self::V2(manifest) => &manifest.backup_directory,
        }
    }

    pub(crate) fn artifacts(&self) -> &[ManifestArtifact] {
        match self {
            Self::V1(manifest) => &manifest.artifacts,
            Self::V2(manifest) => &manifest.artifacts,
        }
    }

    pub(crate) fn sidecars(&self) -> &SidecarManifest {
        match self {
            Self::V1(manifest) => &manifest.sidecars,
            Self::V2(manifest) => &manifest.sidecars,
        }
    }
}

pub(crate) fn parse_host_install_manifest(
    bytes: &[u8],
) -> Result<InstalledHostManifest, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Header {
        schema_version: u32,
    }

    let header: Header = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    match header.schema_version {
        1 => serde_json::from_slice::<LegacyHostInstallManifest>(bytes)
            .map(InstalledHostManifest::V1)
            .map_err(|error| error.to_string()),
        2 => serde_json::from_slice::<HostInstallManifest>(bytes)
            .map(InstalledHostManifest::V2)
            .map_err(|error| error.to_string()),
        version => Err(format!("unsupported install manifest schema {version}")),
    }
}

pub(crate) fn validate_installed_manifest(
    paths: &RuntimePaths,
    manifest: &InstalledHostManifest,
) -> Result<(), String> {
    let backup = manifest.backup_directory();
    if !backup.is_absolute()
        || !backup.starts_with(&paths.state_directory)
        || backup
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        return Err("install manifest backup directory is invalid".into());
    }
    let artifacts = manifest.artifacts();
    if artifacts.len() != 3 {
        return Err("install manifest artifact list is invalid".into());
    }
    for (path, managed) in [
        (Path::new("/etc/resolver/own"), false),
        (Path::new(ANCHOR_PATH), false),
        (Path::new("/etc/pf.conf"), true),
    ] {
        let Some(artifact) = artifacts.iter().find(|artifact| artifact.path == path) else {
            return Err(format!(
                "install manifest artifact list is invalid: missing {}",
                path.display()
            ));
        };
        if artifact.managed_block != managed
            || artifact.mode != 0o644
            || artifact.owner_id != 0
            || artifact.sha256.len() != 64
            || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!(
                "install manifest artifact is invalid: {}",
                path.display()
            ));
        }
    }
    if let InstalledHostManifest::V2(manifest) = manifest {
        let mut names = HashSet::new();
        if manifest.schema_version != 2
            || manifest.interfaces.is_empty()
            || !manifest
                .interfaces
                .iter()
                .all(|name| is_eligible_interface(name) && names.insert(name.as_str()))
        {
            return Err("install manifest interface list is invalid".into());
        }
    }
    Ok(())
}
```

Re-export the crate-private manifest API from `mod.rs`:

```rust
pub(crate) use host::{
    InstalledHostManifest, parse_host_install_manifest, validate_installed_manifest,
};
```

- [ ] **Step 4: Update `setup.rs` to use schema-neutral accessors**

Change the `super` import to include `InstalledHostManifest`, `parse_host_install_manifest`, and `validate_installed_manifest`, and remove `HostInstallManifest` if it becomes unused.

Change `backup_path` to:

```rust
fn backup_path(backup_directory: &Path, path: &Path) -> PathBuf {
    backup_directory.join(path.strip_prefix("/").unwrap_or(path))
}
```

In `preview_uninstall`, use `manifest.artifacts()` and `manifest.backup_directory()`:

```rust
    let artifacts = manifest.artifacts();
    let mut actions = artifacts
        .iter()
        .rev()
        .map(|artifact| {
            let backup = backup_path(manifest.backup_directory(), &artifact.path);
            let has_backup = host.inspect_file(&backup).ok().flatten().is_some();
            SetupAction {
                kind: if has_backup { "restore" } else { "remove" }.into(),
                path: artifact.path.clone(),
                backup_path: has_backup.then_some(backup),
            }
        })
        .collect::<Vec<_>>();
```

In `plan_setup`, use `manifest.artifacts()`:

```rust
            manifest
                .artifacts()
                .iter()
                .map(|item| item.path.clone())
                .collect::<HashSet<_>>()
```

Change the backup calculation to:

```rust
        let backup_path = previous
            .as_ref()
            .filter(|_| artifact.backup_existing)
            .map(|_| backup_path(&plan.manifest.backup_directory, &artifact.path));
```

Change `read_manifest` to return `Option<InstalledHostManifest>`:

```rust
fn read_manifest(
    paths: &RuntimePaths,
    host: &dyn SetupHost,
) -> Result<Option<InstalledHostManifest>, SetupFailure> {
    let Some(file) = host
        .inspect_file(&paths.install_manifest)
        .map_err(failure)?
    else {
        return Ok(None);
    };
    if file.kind != HostFileKind::Regular {
        return Err(failure("setup manifest is not a regular file"));
    }
    let manifest = parse_host_install_manifest(&file.content)
        .map_err(|error| failure(format!("setup manifest is corrupt: {error}")))?;
    if file.owner_id != manifest.user_id() || file.mode != 0o600 {
        return Err(failure("setup manifest ownership or mode is invalid"));
    }
    validate_installed_manifest(paths, &manifest)
        .map_err(|error| failure(format!("setup manifest is invalid: {error}")))?;
    Ok(Some(manifest))
}
```

- [ ] **Step 5: Run the focused tests**

Run: `cargo test -p own-domains-daemon --test macos_setup`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/own-domains-daemon/src/platform/macos/host.rs crates/own-domains-daemon/src/platform/macos/setup.rs crates/own-domains-daemon/tests/macos_setup.rs
git commit -m "feat: migrate macos install manifest to schema 2"
```

---

### Task 3: Inspector coverage diagnostics and schema migration behavior

**Files:**
- Modify: `crates/own-domains-daemon/src/platform/macos/host.rs`
- Modify: `crates/own-domains-daemon/tests/macos_host.rs`
- Modify: `crates/own-domains-daemon/tests/macos_runtime.rs`

**Interfaces:**
- Consumes: `discover_network_interfaces`, `parse_host_install_manifest`, schema 2 anchor rendering from Tasks 1-2.
- Produces: manifest validation that reports `setupRequired` for missing/incompatible/schema 1, `lanAddressInactive`, `interfaceCoverageMissing`, and `inspectionFailed`; exact dynamic-anchor validation; one network diagnostic per failed condition.

- [ ] **Step 1: Replace the inspector fixture commands and add failing tests**

In `tests/macos_host.rs`, import `NetworkInterface` and update `FakeCommands` to:

```rust
struct FakeCommands {
    healthy: bool,
    interfaces: Vec<NetworkInterface>,
    interface_error: bool,
    oversized: bool,
    pf_permission_denied: bool,
    calls: Mutex<Vec<HostCommand>>,
}

impl CommandRunner for FakeCommands {
    fn run(&self, command: &HostCommand) -> Result<HostCommandOutput, String> {
        self.calls.lock().unwrap().push(command.clone());
        let program = command.program.to_string_lossy();
        if program == "/sbin/ifconfig" {
            if self.interface_error {
                return Ok(HostCommandOutput {
                    status: 1,
                    stdout: vec![],
                    stderr: b"ifconfig: operation failed".to_vec(),
                });
            }
            if command.args.len() == 1 && command.args[0] == "-l" {
                return Ok(HostCommandOutput {
                    status: 0,
                    stdout: self
                        .interfaces
                        .iter()
                        .map(|interface| interface.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                        .into_bytes(),
                    stderr: vec![],
                });
            }
            let name = command.args.first().map(String::as_str).unwrap_or_default();
            let found = self
                .interfaces
                .iter()
                .find(|interface| interface.name == name);
            return Ok(HostCommandOutput {
                status: if found.is_some() { 0 } else { 1 },
                stdout: found
                    .and_then(|interface| interface.primary_ipv4)
                    .map(|address| format!("inet {address}"))
                    .unwrap_or_default()
                    .into_bytes(),
                stderr: vec![],
            });
        }
        if self.oversized && program == "/usr/sbin/lsof" {
            return Ok(HostCommandOutput {
                status: 0,
                stdout: vec![b'x'; 65_537],
                stderr: vec![],
            });
        }
        if self.pf_permission_denied && program == "/sbin/pfctl" {
            return Ok(HostCommandOutput {
                status: 1,
                stdout: vec![],
                stderr: b"pfctl: /dev/pf: Permission denied".to_vec(),
            });
        }
        let (status, stdout) = if program.ends_with("coredns-aarch64-apple-darwin") {
            (
                0,
                if self.healthy {
                    "CoreDNS-1.14.7"
                } else {
                    "CoreDNS-1.13.0"
                },
            )
        } else if program.ends_with("caddy-aarch64-apple-darwin") {
            (0, "v2.11.7")
        } else if program == "/sbin/pfctl" {
            (0, if self.healthy { "rdr pass on en0" } else { "" })
        } else if program == "/usr/bin/security" {
            (if self.healthy { 0 } else { 1 }, "")
        } else if program == "/usr/sbin/lsof" {
            (
                if self.healthy { 1 } else { 0 },
                if self.healthy {
                    ""
                } else {
                    "nginx 42 root TCP *:80 (LISTEN)"
                },
            )
        } else {
            return Err(format!("unexpected executable: {program}"));
        };
        Ok(HostCommandOutput {
            status,
            stdout: stdout.as_bytes().to_vec(),
            stderr: vec![],
        })
    }
}
```

Add helpers after `installed_host`:

```rust
fn interface(name: &str, address: &str) -> NetworkInterface {
    NetworkInterface {
        name: name.into(),
        primary_ipv4: Some(address.parse().unwrap()),
    }
}

fn commands(healthy: bool, interfaces: Vec<NetworkInterface>) -> FakeCommands {
    FakeCommands {
        healthy,
        interfaces,
        interface_error: false,
        oversized: false,
        pf_permission_denied: false,
        calls: Mutex::new(vec![]),
    }
}

fn legacy_manifest_bytes(plan: &HostSetupPlan) -> Vec<u8> {
    serde_json::to_vec_pretty(&serde_json::json!({
        "schemaVersion": 1,
        "userName": plan.manifest.user_name,
        "userId": plan.manifest.user_id,
        "backupDirectory": plan.manifest.backup_directory,
        "interface": "en0",
        "lanAddress": "192.168.1.20",
        "artifacts": plan.manifest.artifacts,
        "sidecars": plan.manifest.sidecars,
    }))
    .unwrap()
}
```

Add `HostSetupPlan` to the imports.

Update `inspector_treats_unprivileged_pf_inspection_as_a_warning` to start from `commands(true, vec![interface("en0", "192.168.1.20")])` and then set `pf_permission_denied = true`. Update `inspector_rejects_oversized_command_output` to start from the same healthy commands and then set `oversized = true`. Every `FakeCommands` literal must now provide `interfaces`, `interface_error`, `oversized`, `pf_permission_denied`, and `calls`.

Update the valid-inspector test to use `commands(true, vec![interface("en0", "192.168.1.20")])` and expect this call order:

```rust
        vec![
            "/sbin/ifconfig",
            "/sbin/ifconfig",
            "/sbin/pfctl",
            "/usr/bin/security",
            "/Applications/Own Domains.app/Contents/Resources/coredns-aarch64-apple-darwin",
            "/Applications/Own Domains.app/Contents/Resources/caddy-aarch64-apple-darwin",
            "/usr/sbin/lsof",
        ]
```

In `inspector_reports_stable_codes_for_host_drift_and_conflicts`, use `commands(false, vec![interface("en0", "192.168.1.98")])`, keep the file mutations, set settings to `192.168.1.99`, and replace `"lanIntegrationDrift"` in the expected codes with `"lanAddressInactive"`.

Refactor `installed_host` into `installed_host_for(options: MacOsSetupOptions)` and keep `installed_host()` as a wrapper that calls it with `options(MacOsArchitecture::Arm64)`. Then add these tests:

```rust
#[test]
fn inspector_accepts_an_address_change_without_setup() {
    let (plan, files) = installed_host();
    let commands = commands(true, vec![interface("en0", "192.168.1.21")]);
    let settings = Settings {
        lan_address: "192.168.1.21".into(),
        ..Default::default()
    };

    let diagnostics =
        HostIntegrationInspector::new(&plan.paths, &plan.manifest.sidecars, &files, &commands)
            .inspect(&settings);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "setupRequired"
                && diagnostic.code != "lanIntegrationDrift")
    );
}

#[test]
fn inspector_accepts_switching_to_another_covered_interface() {
    let mut options = options(MacOsArchitecture::Arm64);
    options.covered_interfaces.push(NetworkInterface {
        name: "en1".into(),
        primary_ipv4: Some(Ipv4Addr::new(192, 168, 1, 21)),
    });
    let (plan, files) = installed_host_for(options);
    let commands = commands(
        true,
        vec![interface("en0", "192.168.1.20"), interface("en1", "192.168.1.21")],
    );
    let settings = Settings {
        lan_address: "192.168.1.21".into(),
        ..Default::default()
    };

    let diagnostics =
        HostIntegrationInspector::new(&plan.paths, &plan.manifest.sidecars, &files, &commands)
            .inspect(&settings);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn inspector_reports_one_diagnostic_for_an_uncovered_selected_interface() {
    let (plan, files) = installed_host();
    let commands = commands(
        true,
        vec![interface("en0", "192.168.1.20"), interface("en1", "192.168.1.99")],
    );
    let settings = Settings {
        lan_address: "192.168.1.99".into(),
        ..Default::default()
    };

    let diagnostics =
        HostIntegrationInspector::new(&plan.paths, &plan.manifest.sidecars, &files, &commands)
            .inspect(&settings);

    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "interfaceCoverageMissing")
            .count(),
        1,
        "{diagnostics:?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .remedy
            .as_deref()
            .is_some_and(|remedy| remedy.contains("Run setup once"))),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "lanAddressInactive")
    );
}

#[test]
fn inspector_prefers_coverage_missing_when_covered_interfaces_are_addressless() {
    let (plan, files) = installed_host();
    let commands = commands(
        true,
        vec![
            NetworkInterface {
                name: "en0".into(),
                primary_ipv4: None,
            },
            interface("en1", "192.168.1.99"),
        ],
    );
    let settings = Settings {
        lan_address: "192.168.1.99".into(),
        ..Default::default()
    };

    let diagnostics =
        HostIntegrationInspector::new(&plan.paths, &plan.manifest.sidecars, &files, &commands)
            .inspect(&settings);

    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "interfaceCoverageMissing")
            .count(),
        1,
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "lanAddressInactive")
    );
}

#[test]
fn inspector_reports_schema_one_as_setup_required() {
    let (plan, mut files) = installed_host();
    files.content.insert(
        plan.paths.install_manifest.clone(),
        legacy_manifest_bytes(&plan),
    );
    let commands = commands(true, vec![interface("en0", "192.168.1.20")]);
    let settings = Settings {
        lan_address: "192.168.1.20".into(),
        ..Default::default()
    };

    let diagnostics =
        HostIntegrationInspector::new(&plan.paths, &plan.manifest.sidecars, &files, &commands)
            .inspect(&settings);

    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "setupRequired");
    assert!(
        diagnostics[0]
            .remedy
            .as_deref()
            .is_some_and(|remedy| remedy.contains("Run setup once")),
        "{diagnostics:?}"
    );
}

#[test]
fn inspector_rejects_a_manifest_artifact_digest_mismatch() {
    let (plan, mut files) = installed_host();
    let bytes = files
        .content
        .get(&plan.paths.install_manifest)
        .unwrap()
        .clone();
    let mut manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    manifest["artifacts"][0]["sha256"] = serde_json::Value::String("0".repeat(64));
    files.content.insert(
        plan.paths.install_manifest.clone(),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    );
    let commands = commands(true, vec![interface("en0", "192.168.1.20")]);
    let settings = Settings {
        lan_address: "192.168.1.20".into(),
        ..Default::default()
    };

    let diagnostics =
        HostIntegrationInspector::new(&plan.paths, &plan.manifest.sidecars, &files, &commands)
            .inspect(&settings);

    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "setupRequired");
}

#[test]
fn inspector_reports_discovery_failure_instead_of_inactive_address() {
    let (plan, files) = installed_host();
    let mut commands = commands(true, vec![interface("en0", "192.168.1.20")]);
    commands.interface_error = true;
    let settings = Settings {
        lan_address: "192.168.1.20".into(),
        ..Default::default()
    };

    let diagnostics =
        HostIntegrationInspector::new(&plan.paths, &plan.manifest.sidecars, &files, &commands)
            .inspect(&settings);

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "inspectionFailed")
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "lanAddressInactive")
    );
}
```

- [ ] **Step 2: Run the focused tests and verify they fail**

Run: `cargo test -p own-domains-daemon --test macos_host`

Expected: FAIL because the inspector still uses schema 1 fields, `scutil`, and `lanIntegrationDrift`.

- [ ] **Step 3: Validate schema 2 and artifact hashes in `read_manifest`**

Replace `read_manifest` with:

```rust
    fn read_manifest(&self, diagnostics: &mut Vec<Diagnostic>) -> Option<HostInstallManifest> {
        let metadata = match self.files.metadata(&self.paths.install_manifest) {
            Ok(Some(metadata)) if metadata.kind == HostFileKind::Regular => metadata,
            Ok(Some(_)) => {
                diagnostics.push(error(
                    "setupManifestInvalid",
                    "The setup manifest is not a regular file.",
                    "Run privileged setup again.",
                ));
                return None;
            }
            Ok(None) => {
                diagnostics.push(error(
                    "setupRequired",
                    "Own Domains host setup is missing.",
                    "Run privileged setup.",
                ));
                return None;
            }
            Err(message) => {
                diagnostics.push(error(
                    "setupManifestInvalid",
                    &format!("Unable to inspect the setup manifest: {message}"),
                    "Check state-directory permissions, then rerun setup.",
                ));
                return None;
            }
        };
        let bytes = match self
            .files
            .read(&self.paths.install_manifest, MAX_FILE_BYTES)
        {
            Ok(bytes) => bytes,
            Err(message) => {
                diagnostics.push(error(
                    "setupManifestInvalid",
                    &format!("Unable to read the setup manifest: {message}"),
                    "Run privileged setup again.",
                ));
                return None;
            }
        };
        let installed = match parse_host_install_manifest(&bytes) {
            Ok(manifest) => manifest,
            Err(message) => {
                diagnostics.push(error(
                    "setupRequired",
                    &format!("The setup manifest is incompatible with this app build: {message}"),
                    "Run setup once to enable network-independent routing.",
                ));
                return None;
            }
        };
        if metadata.owner_id != installed.user_id()
            || metadata.mode != 0o600
            || installed.sidecars() != &self.sidecars
        {
            diagnostics.push(error(
                "setupRequired",
                "The setup manifest is incompatible with this app build.",
                "Run setup once to enable network-independent routing.",
            ));
            return None;
        }
        match installed {
            InstalledHostManifest::V1(_) => {
                diagnostics.push(error(
                    "setupRequired",
                    "The privileged setup uses the previous install manifest format.",
                    "Run setup once to enable network-independent routing.",
                ));
                None
            }
            InstalledHostManifest::V2(manifest) if valid_manifest(&manifest) => Some(manifest),
            InstalledHostManifest::V2(_) => {
                diagnostics.push(error(
                    "setupRequired",
                    "The setup manifest does not match the owned host artifacts.",
                    "Run setup once to enable network-independent routing.",
                ));
                None
            }
        }
    }
```

Add the helper:

```rust
fn valid_manifest(manifest: &HostInstallManifest) -> bool {
    if manifest.schema_version != 2 || manifest.interfaces.is_empty() {
        return false;
    }
    let mut names = HashSet::new();
    if !manifest
        .interfaces
        .iter()
        .all(|name| is_eligible_interface(name) && names.insert(name.as_str()))
    {
        return false;
    }
    let anchor = render_anchor_for(&manifest.interfaces);
    let expected: [(PathBuf, String, u32, u32, bool); 3] = [
        (
            PathBuf::from("/etc/resolver/own"),
            sha256(RESOLVER_CONTENT.as_bytes()),
            0o644,
            0,
            false,
        ),
        (
            PathBuf::from(ANCHOR_PATH),
            sha256(anchor.as_bytes()),
            0o644,
            0,
            false,
        ),
        (
            PathBuf::from("/etc/pf.conf"),
            sha256(PF_CONF_BLOCK.as_bytes()),
            0o644,
            0,
            true,
        ),
    ];
    manifest.artifacts.len() == expected.len()
        && expected.iter().all(|(path, digest, mode, owner, managed)| {
            manifest.artifacts.iter().any(|artifact| {
                artifact.path == *path
                    && artifact.sha256 == *digest
                    && artifact.mode == *mode
                    && artifact.owner_id == *owner
                    && artifact.managed_block == *managed
            })
        })
}
```

- [ ] **Step 4: Replace the network probe and anchor validation**

Replace the resolver literal in `inspect` with `Some(RESOLVER_CONTENT)`. Replace the local `BLOCK` constant in `inspect_pf_conf` with `PF_CONF_BLOCK.as_bytes()` so the renderer, inspector, and manifest hash all use one definition.

Change the anchor check in `inspect` to:

```rust
        let expected_anchor = render_anchor_for(&manifest.interfaces);
        self.inspect_managed_file(
            Path::new(ANCHOR_PATH),
            0,
            Some(&expected_anchor),
            "pfAnchorInvalid",
            &mut diagnostics,
        );
```

Replace `inspect_network` with:

```rust
    fn inspect_network(
        &self,
        settings: &Settings,
        manifest: &HostInstallManifest,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let interfaces = match discover_network_interfaces(self.commands) {
            Ok(interfaces) => interfaces,
            Err(message) => {
                diagnostics.push(error(
                    "inspectionFailed",
                    &format!("Unable to inspect network interfaces: {message}"),
                    "Verify macOS command-line tools and retry.",
                ));
                return;
            }
        };
        let covered = manifest
            .interfaces
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let selected = settings.lan_address.parse::<Ipv4Addr>().ok();
        let owner = selected.and_then(|address| {
            interfaces
                .iter()
                .find(|interface| interface.primary_ipv4 == Some(address))
        });
        // Success requires the selected address to belong to a covered
        // interface, which also proves that a covered interface currently has
        // an address. When the selected address belongs to a newly appeared
        // eligible interface, coverageMissing takes precedence even if every
        // covered interface is temporarily addressless.
        match owner {
            Some(interface) if covered.contains(interface.name.as_str()) => {}
            Some(interface) => diagnostics.push(error(
                "interfaceCoverageMissing",
                &format!(
                    "The selected LAN address {} belongs to interface {}, which privileged setup does not cover.",
                    settings.lan_address, interface.name
                ),
                "Run setup once to add the interface.",
            )),
            None => diagnostics.push(error(
                "lanAddressInactive",
                "The selected LAN address is not the primary IPv4 address of a covered interface.",
                "Select an active LAN address in Network Settings and save.",
            )),
        }
    }
```

- [ ] **Step 5: Update the runtime blocking test**

In `tests/macos_runtime.rs`, rename `port_conflict_and_lan_drift_block_start` to `port_conflict_and_inactive_lan_address_block_start`, replace the injected diagnostic with:

```rust
    fixture.inspector.set(vec![error_diagnostic(
        "lanAddressInactive",
        "The selected LAN address is not the primary IPv4 address of a covered interface.",
    )]);
    let error = backend.apply(&settings(1, "192.168.1.20")).unwrap_err();
    assert!(error.to_string().contains("primary IPv4"), "{error}");
    assert!(fixture.host.spawn_attempts().is_empty());
```

Keep the existing port-conflict half unchanged.

Also add a test proving that an inactive address fails before the applied revision changes:

```rust
#[test]
fn inactive_lan_address_blocks_start_without_replacing_the_applied_revision() {
    let fixture = Fixture::new();
    let backend = fixture.backend();

    let applied = backend.apply(&settings(1, "192.168.1.20")).unwrap();
    assert_eq!(applied.applied_revision, 1);
    let spawns = fixture.host.spawn_attempts().len();

    fixture.inspector.set(vec![error_diagnostic(
        "lanAddressInactive",
        "The selected LAN address is not the primary IPv4 address of a covered interface.",
    )]);
    let error = backend.apply(&settings(2, "192.168.1.21")).unwrap_err();

    assert!(error.to_string().contains("primary IPv4"), "{error}");
    assert_eq!(fixture.applied_revision(), "1");
    assert_eq!(fixture.host.spawn_attempts().len(), spawns);
}
```

- [ ] **Step 6: Run the focused tests**

Run:

```bash
cargo test -p own-domains-daemon --test macos_host
cargo test -p own-domains-daemon --test macos_runtime
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/own-domains-daemon/src/platform/macos/host.rs crates/own-domains-daemon/tests/macos_host.rs crates/own-domains-daemon/tests/macos_runtime.rs
git commit -m "feat: report macos interface coverage"
```

---

### Task 4: CLI interface discovery and selected-primary validation

**Files:**
- Modify: `crates/own-domains-cli/src/platform/macos.rs`

**Interfaces:**
- Consumes: `discover_network_interfaces`, `NetworkInterface` from Task 1.
- Produces: `validate_selected_interface(&[NetworkInterface], &str, Ipv4Addr) -> Result<(), String>` and `MacOsSetupOptions.covered_interfaces` populated from the live host.

- [ ] **Step 1: Add failing unit tests**

Add `NetworkInterface` to the daemon import list. Add to the test module:

```rust
    fn address(value: &str) -> Ipv4Addr {
        value.parse().unwrap()
    }

    #[test]
    fn requires_the_selected_interface_to_be_present_and_primary() {
        let interfaces = vec![NetworkInterface {
            name: "en0".into(),
            primary_ipv4: Some(address("192.168.1.20")),
        }];

        assert!(
            validate_selected_interface(&interfaces, "en0", address("192.168.1.20")).is_ok()
        );
        assert!(
            validate_selected_interface(&interfaces, "en1", address("192.168.1.20"))
                .unwrap_err()
                .contains("present")
        );
        assert!(
            validate_selected_interface(&interfaces, "en0", address("192.168.1.21"))
                .unwrap_err()
                .contains("primary")
        );
    }
```

- [ ] **Step 2: Run the test and verify it fails to compile**

Run: `cargo test -p own-domains-cli requires_the_selected_interface_to_be_present_and_primary`

Expected: FAIL because `validate_selected_interface` does not exist and `covered_interfaces` is not populated.

- [ ] **Step 3: Replace the single-interface query with discovery**

Add `NetworkInterface` and `discover_network_interfaces` to the daemon import list. Delete `interface_addresses` and add:

```rust
fn validate_selected_interface(
    interfaces: &[NetworkInterface],
    interface: &str,
    lan_address: Ipv4Addr,
) -> Result<(), String> {
    let selected = interfaces
        .iter()
        .find(|candidate| candidate.name == interface)
        .ok_or_else(|| {
            format!("interface {interface} is not an eligible present LAN interface")
        })?;
    if selected.primary_ipv4 != Some(lan_address) {
        return Err(format!(
            "LAN address {lan_address} is not the primary IPv4 address of interface {interface}"
        ));
    }
    Ok(())
}
```

Remove `CommandRunner` and `HostCommand` from the daemon import list; both were only used by `interface_addresses`. Keep `Duration`, which is still used by the readiness wait.

In `setup_options`, replace:

```rust
    let addresses = interface_addresses(interface)?;
```

with:

```rust
    let covered_interfaces = discover_network_interfaces(&SystemCommandRunner)
        .map_err(|error| format!("unable to discover LAN interfaces: {error}"))?;
    validate_selected_interface(&covered_interfaces, interface, lan_address)?;
```

Replace `interface_addresses: addresses` with `covered_interfaces` in the `MacOsSetupOptions` construction.

- [ ] **Step 4: Run the focused tests**

Run: `cargo test -p own-domains-cli`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/own-domains-cli/src/platform/macos.rs
git commit -m "feat: discover macos interfaces during setup"
```

---

### Task 5: Frontend proof that an address save stays unprivileged

**Files:**
- Modify: `tests/web/native-management.test.tsx`

**Interfaces:**
- Consumes: existing `NativeApp`, `RuntimeStore`, `NativeClient`, and the `updateNetwork` protocol command.
- Produces: a DOM-level regression test that saves a new LAN address, observes the refreshed setting without an error alert, and sees no setup/privilege message.

- [ ] **Step 1: Add the failing test**

Append:

```tsx
it('saves a LAN address change without asking for privileged setup', async () => {
  const state: StateSnapshot = {
    settings: { schemaVersion: 1, revision: 3, lanAddress: '192.168.1.20', upstreamDns: ['1.1.1.1'], mappings: [] },
    runtimeStatus: 'running',
    desiredRevision: 3,
    appliedRevision: 3,
    diagnostics: [],
    latestEventSequence: 0,
  };
  const request = vi.fn<NativeClient['request']>(async (command) => {
    if (command.type === 'snapshot') return { type: 'snapshot', payload: structuredClone(state) };
    if (command.type === 'updateNetwork') {
      state.settings = { ...state.settings, lanAddress: command.payload.lan_address, revision: 4 };

      return {
        type: 'mutation',
        payload: {
          settings: structuredClone(state.settings),
          apply: { desiredRevision: 4, appliedRevision: 4, status: 'applied' as const },
        },
      };
    }

    throw new Error(`Unexpected command: ${command.type}`);
  });
  const client: NativeClient = { subscribe: async () => async () => {}, request };
  const user = userEvent.setup();
  const store = new RuntimeStore(client);
  const view = render(<NativeApp store={store} onStart={async () => {}} onStop={async () => {}} onQuit={async () => {}} />);

  await screen.findByText('Runtime: running');
  const input = screen.getByLabelText('Mac LAN IPv4');
  await user.clear(input);
  await user.type(input, '192.168.1.21');
  await user.click(screen.getByRole('button', { name: 'Save network settings' }));

  await waitFor(() => expect(screen.getByLabelText('Mac LAN IPv4')).toHaveValue('192.168.1.21'));
  expect(request).toHaveBeenCalledWith(expect.objectContaining({
    type: 'updateNetwork',
    payload: { lan_address: '192.168.1.21', upstream_dns: ['1.1.1.1'], expected_revision: 3 },
  }));
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  expect(screen.queryByText(/setupRequired|privileged setup/i)).not.toBeInTheDocument();
  view.unmount();
});
```

- [ ] **Step 2: Run the test and verify the assertion**

Run: `pnpm exec vitest run tests/web/native-management.test.tsx`

Expected: PASS. The test drives the existing `NativeApp`; no production frontend change is required. The editor remounts when the settings revision changes, so assert the refreshed input value rather than the transient local save message. Do not add setup messaging to `NetworkSettingsEditor`.

- [ ] **Step 3: Commit**

```bash
git add tests/web/native-management.test.tsx
git commit -m "test: cover unprivileged lan address saves"
```

---

### Task 6: Documentation and manual acceptance checklist

**Files:**
- Modify: `README.md:5`
- Modify: `docs/desktop.md:66`
- Modify: `docs/cli.md:52`
- Modify: `docs/platform-support.md:14`
- Modify: `docs/macos-acceptance.md` (insert before `## 3. Manual GUI checks`, currently line 105, and renumber the following sections)
- Modify: `docs/superpowers/specs/2026-10-21-macos-runtime-activation-design.md:1` (add a superseded note)

**Interfaces:**
- Consumes: the implemented behavior and acceptance evidence from the PF gate.
- Produces: user-facing statements about one-time setup, covered interfaces, schema 2, no recurring root prompt, and the new manual checks.

- [ ] **Step 1: Update the README native paragraph**

Replace the native macOS beta paragraph with:

```markdown
The native client packages pinned CoreDNS and Caddy sidecars, verifies their digests, and resolves them from the app bundle. A one-time privileged setup installs the `.own` resolver, a dedicated PF anchor, and the Caddy root certificate. Setup covers every eligible LAN interface present at that time, including interfaces without a current IPv4 address, and writes dynamic `(if:0)` PF rules. Changing the selected LAN address or switching between covered interfaces needs no administrator access; adding a previously absent hardware interface may require one more setup. The install manifest stores covered interface names but no LAN address. An existing schema 1 setup is migrated by running setup once. The [macOS acceptance gate](docs/macos-acceptance.md) passed for an ad-hoc-signed arm64 package, including automated routing, GUI lifecycle, and physical-phone checks. Do not remove the legacy source or its migration/uninstall support while beta users still depend on it.
```

- [ ] **Step 2: Update desktop docs**

After the paragraph ending “Normal desktop and CLI launches never request administrator credentials.”, add:

```markdown
Setup records every eligible `en*` or `bridge*` interface present at apply time, including interfaces with no current IPv4 address, and writes dynamic `(if:0)` PF rules. The schema 2 install manifest stores the covered interface names and artifact hashes but no LAN address. Changing the selected address or switching between covered interfaces is handled by the unprivileged runtime. A hardware interface that did not exist during setup requires one additional setup. A schema 1 installation is migrated by the next setup run; the old address does not need to be active.
```

- [ ] **Step 3: Update CLI docs**

After the setup paragraph, add:

```markdown
Setup covers every eligible interface present at apply time. The privileged anchor references `(if:0)` and contains no selected LAN address; the manifest stores only covered interface names. Re-running setup with the same interface set is a no-op. Re-run setup only when a new hardware interface must be added. Normal LAN address changes are saved by the app and do not require `setup`. A schema 1 installation is migrated by the next `setup --apply`; it does not require the old address to be active.
```

- [ ] **Step 4: Update platform-support docs**

Append to the host-integration paragraph:

```markdown
On macOS, the privileged anchor and schema 2 install manifest contain no selected LAN address. Address changes and switching between covered interfaces stay unprivileged; only initial setup and covering a newly appeared interface require root.
```

- [ ] **Step 5: Mark the earlier design as superseded**

Add immediately after the title in `docs/superpowers/specs/2026-10-21-macos-runtime-activation-design.md`:

```markdown
> **Superseded for network selection:** The dynamic PF and schema 2 design in [Network-Independent macOS Host Integration](2026-10-23-network-independent-macos-integration-design.md) replaces the pinned address/interface and `setupRequired` behavior described here. The remaining runtime, setup, trust, and acceptance decisions still apply.
```

- [ ] **Step 6: Add manual acceptance checks**

Insert as `## 3. Network-independent routing checks` before the current `## 3. Manual GUI checks`, then renumber the current sections 3, 4, and 5 to 4, 5, and 6.

```markdown
## 3. Network-independent routing checks

After setup, record these checks without repeating privileged setup:

1. If upgrading from schema 1, run setup once and confirm it succeeds without the old address being active. Inspect `install-manifest.json`; it must report `schemaVersion: 2`, list covered `interfaces`, and contain no `lanAddress` field or singular `interface` field. Record `sudo shasum -a 256 /etc/pf.anchors/dev.owndomains`.
2. Change the Mac's LAN address on a covered interface (new DHCP reservation or lease), save it in **Network settings**, and confirm DNS, HTTP, HTTPS, `setup.own`, and phone access without an administrator prompt. The anchor hash must remain unchanged.
3. Switch to another interface that was present during setup and confirm the same routes without an administrator prompt.
4. Connect a hardware interface that was absent during setup and select its address in **Network settings**. Confirm the app reports `interfaceCoverageMissing`, run setup once, and confirm the diagnostic clears.
5. Run `sudo pfctl -a dev.owndomains -s nat` and confirm the rules show symbolic dynamic targets such as `(en0:0)` / `(en8:0)`, not a literal selected address.
6. Stop routing and confirm phone `.own` access fails; start routing again and confirm it returns without setup. Uninstall must restore the prior resolver and PF files.
```

- [ ] **Step 7: Run documentation-adjacent checks**

Run:

```bash
pnpm run lint
pnpm run typecheck
pnpm run build
```

Expected: PASS. Then run the full Rust gates from the Global Constraints.

- [ ] **Step 8: Commit**

```bash
git add README.md docs/desktop.md docs/cli.md docs/platform-support.md docs/macos-acceptance.md docs/superpowers/specs/2026-10-21-macos-runtime-activation-design.md
git commit -m "docs: document network-independent macos setup"
```

---

## Final Verification

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Run `cargo test --workspace`.
- [ ] Run `pnpm exec vitest run tests/web/native-management.test.tsx`.
- [ ] Run `pnpm run lint`.
- [ ] Run `pnpm run typecheck`.
- [ ] Run `pnpm run build`.
- [ ] Review `git diff main...HEAD` for accidental shared-contract, protocol, settings-schema, or non-macOS changes.
- [ ] With the current `en0` address (`192.168.1.4` at plan time), apply setup once through the administrator prompt, confirm schema 2 migration from the existing schema 1 manifest, and verify the anchor contains dynamic rules for all eligible present interfaces.
- [ ] Change the selected LAN address or covered interface and verify routing without another administrator prompt; record evidence in `docs/macos-acceptance.md` only when the manual check is actually performed.
