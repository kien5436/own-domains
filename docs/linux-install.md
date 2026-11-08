# Linux install guide (Ubuntu 24.04 x86_64, GUI + CLI)

This installs the Tauri desktop app (bundled daemon, CLI, and sidecars),
then runs privileged host setup once. Greenfield installs only; reinstalls
and upgrades are not covered yet.

## 1. Install the `.deb`

Build from the repo (see Task 4 of `docs/plans/2026-11-08-linux-deb.md`
for the exact build commands), then:

```sh
sudo dpkg -i own-domains_0.1.0-beta.3_amd64.deb
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

Preview (read-only, no sudo needed), then apply:

```sh
own-domains setup --lan-address <guest-ip> --interface <iface>
sudo -E own-domains setup --apply --lan-address <guest-ip> --interface <iface>
```

Re-running `--apply` is idempotent: unchanged artifacts report
`unchanged`, drifted ones are backed up and rewritten.

## 3. Verify the runtime

```sh
own-domains status
own-domains doctor          # expect zero diagnostics on a healthy host
```

`doctor` runs the same diagnostics as the GUI: manifest, nftables table,
systemd units, resolved link, trust, sidecars, ports, and interface
coverage. Anything it reports names the fix; warnings (e.g.
`nftablesServiceConflict`) never block.

## 4. Uninstall

Remove host integration first, then the package:

```sh
sudo -E own-domains uninstall --apply [--purge]
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
