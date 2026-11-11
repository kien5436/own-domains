# Linux install guide (Ubuntu 24.04 x86_64, GUI + CLI)

This installs the Tauri desktop app (bundled daemon, CLI, and sidecars),
then runs privileged host setup once. Re-running the script updates the app;
re-run setup only when a new network interface appears.

`scripts/install.sh` performs the build, package install, and host setup in one
step (see the [README](../README.md#install)). The manual steps below are the
same flow spelled out for debugging the build on a fresh VM.

## 1. Build and install the `.deb`

All commands run from the repository root:

```sh
# System deps for Tauri v2 (record the exact set that works on 24.04):
sudo apt update && sudo apt install -y \
  build-essential curl wget file libssl-dev \
  libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
  libwebkit2gtk-4.1-dev

pnpm install
cargo build --release -p own-domains-daemon -p own-domains-cli

# Stage ALL FIVE sidecar targets. tauri.conf.json lists darwin, Windows,
# and Linux resources unconditionally, and the Tauri build script fails
# when any entry is missing ("resource path ... doesn't exist").
# Fetching is download-only; non-host archives are never executed.
pnpm run sidecars:fetch                  # darwin (both archs)
pnpm run sidecars:fetch -- --platform windows
pnpm run sidecars:fetch -- --platform linux

If GitHub release downloads fail with 5xx (CDN outage — reproducible
from multiple networks, not a repo bug), skip downloading: copy the
already-verified `sidecars/build/<triple>/` trees from any machine that
has them (same bytes, same pinned digests) and verify offline instead:

```sh
# on the source machine, for each missing triple:
scp -r sidecars/build/<triple> <user>@<vm>:~/own-domains/sidecars/build/
# on the VM (add --platform windows|linux to cover each):
pnpm run sidecars:fetch -- --verify-only
```

pnpm run desktop:build:linux
ls target/x86_64-unknown-linux-gnu/release/bundle/deb/
```

Then install:

```sh
sudo dpkg -i target/x86_64-unknown-linux-gnu/release/bundle/deb/*.deb
sudo apt-get install -f   # only if dpkg reports missing dependencies
```

Verify the bundle contents:

```sh
dpkg -L own-domains | grep -E "own-domains-daemon|own-domains$|coredns|caddy" | head
```

## 2. Run privileged setup once

Find the guest LAN address and interface:

```sh
ip -brief addr show
# example: enp3s0 UP 192.168.64.5/24 → interface enp3s0, address 192.168.64.5
```

Preview (read-only with `--dry-run`, no sudo needed), then apply:

```sh
own-domains setup --dry-run --lan-address <guest-ip> --interface <iface>
sudo -E own-domains setup --lan-address <guest-ip> --interface <iface>
```

Re-running setup is idempotent: unchanged artifacts report
`unchanged`, drifted ones are backed up and rewritten.

## 3. Verify the runtime

```sh
own-domains status
sudo -E own-domains doctor   # expect zero diagnostics on a healthy host
```

`doctor` runs the same diagnostics as the GUI: manifest, nftables table,
systemd units, resolved link, trust, sidecars, ports, and interface
coverage. Anything it reports names the fix; warnings (e.g.
`nftablesServiceConflict`) never block.

Run `doctor` without sudo and you will see `*Unverifiable` warnings
instead (`nftablesUnverifiable`, `firewallUnverifiable`): the table and
the firewall policy cannot be read without privilege, so the inspector
declines to claim they are safe. That is the designed unprivileged
behavior, not a failure.

## 4. Uninstall

Remove host integration first, then the package:

```sh
own-domains uninstall --dry-run          # preview, no sudo
sudo -E own-domains uninstall [--purge]
sudo dpkg -r own-domains
```

Without `--purge`, settings and the CA remain (untrusted) so a later
setup reuses the same identity. With `--purge`, the state directory
goes too.

## Files and state

- App + sidecars: the `.deb` payload (`dpkg -L own-domains`).
- State dir: `~/.local/state/own-domains` (or `$XDG_STATE_HOME/own-domains`).
- Host artifacts: `/etc/own-domains/nftables.conf`, the two
  `own-domains-*.service` units, `/usr/local/share/ca-certificates/own-domains.crt`.
