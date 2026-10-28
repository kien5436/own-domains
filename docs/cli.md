# Native CLI

The Rust CLI and daemon are additive during migration; the existing TypeScript commands remain available until the macOS acceptance gate passes.

```sh
cargo run -p own-domains-daemon -- --state-dir .own-domains
cargo run -p own-domains-cli --bin own-domains -- --state-dir .own-domains status
cargo run -p own-domains-cli --bin own-domains -- --state-dir .own-domains add shop.own --port 3000 --revision 0 --https
```

The command surface is `start`, `stop`, `status`, `list`, `add`, `edit`, `enable`, `disable`, `remove`, `doctor`, `setup`, and `uninstall`. Use `--json` for one machine-readable protocol result on stdout. Mutation commands require `--revision` to prevent stale writes. `--state-dir` may be omitted: the CLI then uses the platform default (`%LOCALAPPDATA%\OwnDomains` on Windows, `~/Library/Application Support/OwnDomains` on macOS, `$XDG_STATE_HOME/own-domains` or `~/.local/state/own-domains` on Linux), the same directory the desktop app uses by default.

When no daemon is running, reads report a stopped runtime and mutations take the exclusive state lock, save through the same daemon command handler, print `saved; runtime stopped`, and exit without starting DNS or proxy services. `start` launches `own-domains-daemon`; set `OWN_DOMAINS_DAEMON` to an explicit binary path when it is not on `PATH`.

On macOS, `start` activates the bundled CoreDNS and Caddy sidecars behind the host integration installed by privileged setup. Other platforms intentionally return `runtimeUnavailable`.

## macOS sidecar layout

The daemon resolves `own-domains-daemon` beside the desktop executable. Packaged resources use this layout:

```text
Contents/Resources/
  coredns-aarch64-apple-darwin
  caddy-aarch64-apple-darwin
  sidecars/manifest-aarch64-apple-darwin.json
```

An x86_64 package uses the matching `x86_64-apple-darwin` names. A standalone CLI release uses the same resource-relative layout, with `own-domains` as the executable:

```text
own-domains
own-domains-daemon
coredns-aarch64-apple-darwin
caddy-aarch64-apple-darwin
sidecars/manifest-aarch64-apple-darwin.json
```

Set `OWN_DOMAINS_RESOURCES` to an absolute resource directory for local development. Before setup or uninstall, the CLI verifies the target manifest and both executable SHA-256 digests; it never falls back to a `PATH` binary.

## Privileged setup

Run setup previews as the logged-in user. Applying changes requires `sudo`, so the CLI receives the target account in `SUDO_USER` and `SUDO_UID`:

```sh
own-domains --state-dir "$HOME/Library/Application Support/OwnDomains" \
  setup --lan-address 192.168.1.20 --interface en0

sudo own-domains --state-dir "$HOME/Library/Application Support/OwnDomains" \
  setup --apply --lan-address 192.168.1.20 --interface en0
```

Setup validates the selected interface address, writes only the owned resolver and PF anchor, backs up replaced host files, installs the Caddy root certificate, and reloads PF. If no Caddy CA exists, it generates the persistent CA as the target user before installing trust; it never installs trust itself during CA generation.

Setup covers every eligible interface present at apply time. The privileged anchor references `(if:0)` and contains no selected LAN address; the manifest records covered interface names without storing a LAN address. Re-running setup with the same interface set is a no-op. Re-run setup only when a new hardware interface must be added. Normal LAN address changes are saved by the app and do not require `setup`. A schema 1 installation is migrated by the next `setup --apply`; it does not require the old address to be active.

Uninstall is reversible and preserves settings unless `--purge` is explicit:

```sh
sudo own-domains --state-dir "$HOME/Library/Application Support/OwnDomains" uninstall --apply
sudo own-domains --state-dir "$HOME/Library/Application Support/OwnDomains" uninstall --apply --purge
```

Only use `--purge` for a disposable acceptance state: it removes settings, backups, logs, and the Caddy CA.

Exit codes:

- `0`: success
- `2`: invalid input, missing mapping, or unsafe privileged invocation
- `3`: stale settings revision or unsupported platform
- `4`: daemon, IPC, storage, host setup, or protocol failure
- `5`: saved mutation whose runtime application failed
