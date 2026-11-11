# Native desktop development

The Tauri 2 client reuses the React components and manages the Rust daemon through private IPC. On macOS it activates the bundled CoreDNS and Caddy runtime after a one-time privileged setup. See [Platform support](platform-support.md) for the current matrix and signing policy.

## Build prerequisites

Use Rust 1.90 or newer and pnpm. Install the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/) for your development host. Apple silicon macOS 15+ is the native acceptance target; Intel macOS is buildable but not acceptance-required. Ubuntu 24+ requires GTK 3, WebKitGTK 4.1, and AppIndicator development libraries. Windows 10/11 ships as a beta NSIS build over the named-pipe IPC backend.

Approved pinned dependencies: Rust `tauri` 2.12.4, `tauri-build` 2.7.3, `tauri-plugin-single-instance` 2.5.4; JavaScript `@tauri-apps/api` 2.12.4 and development CLI 2.12.3; CoreDNS 1.14.7 and Caddy 2.11.7 sidecars.

```bash
pnpm install --frozen-lockfile
pnpm run sidecars:fetch -- --verify-only
cargo build -p own-domains-daemon -p own-domains-cli
```

`sidecars:fetch` downloads the pinned macOS release assets and verifies archive checksums, Caddy's signed checksum manifest, and each extracted executable's SHA-256 digest. Build output lives under ignored `sidecars/build/<target>/`. Run the download once without `--verify-only` to populate a new checkout.

Select the existing native settings directory explicitly. A bare GUI launch uses the platform default (`%LOCALAPPDATA%\OwnDomains` on Windows, `~/Library/Application Support/OwnDomains` on macOS, `$XDG_STATE_HOME/own-domains` or `~/.local/state/own-domains` on Linux); there is no default relative to the launch directory and no automatic migration from older application data. GUI and CLI must receive the same directory, so pass the same explicit `--state-dir` to the CLI when it differs from the default.

```bash
export OWN_DOMAINS_STATE_DIR="$HOME/Library/Application Support/OwnDomains"
export OWN_DOMAINS_DAEMON=/absolute/path/to/own-domains/target/debug/own-domains-daemon
pnpm run desktop:dev
```

The daemon executable override is a trusted launcher setting, never a WebView argument. Without it, the desktop expects `own-domains-daemon` beside its executable. The desktop binary also accepts `--state-dir /absolute/path` and optional `--socket /absolute/path/daemon.sock`; do not point the socket at another state directory's daemon.

```bash
cargo run -p own-domains-cli -- --state-dir "$OWN_DOMAINS_STATE_DIR" status
cargo run -p own-domains-cli -- --state-dir "$OWN_DOMAINS_STATE_DIR" add shop.own --port 3000 --revision 0
```

Use the actual revision reported by status for edits. A conflict reloads state without retrying or overwriting the competing change.

## Packaging

`desktop:prepare` builds the daemon and CLI and stages both for Tauri's `externalBin` entries. `desktop:build` creates a release executable with the staged binaries; use the architecture-specific scripts when cross-building on macOS:

```bash
pnpm run desktop:prepare
pnpm run desktop:build
pnpm run desktop:build:arm64
pnpm run desktop:build:x86_64
```

The Tauri configuration declares both architecture sidecars and per-target runtime manifests as resources, places the daemon beside the desktop executable, and sets the macOS minimum version to 15.0. The Windows install additionally carries `own-domains.exe` next to `own-domains-daemon.exe`, so the CLI runs with no build tree present. Local acceptance uses an ad-hoc-signed arm64 app bundle; Developer ID signing, notarization, and a distributable installer remain distribution work. See [macOS acceptance](macos-acceptance.md) and [Platform support](platform-support.md).

On Linux installations where Homebrew appears before `/usr/bin` in `PATH`, select the system `pkg-config` so Cargo can find the APT-installed GTK/WebKit libraries:

```bash
PKG_CONFIG=/usr/bin/pkg-config pnpm run desktop:build
```

`desktop:build` explicitly builds without a bundle. [macOS acceptance](macos-acceptance.md) uses an ad-hoc-signed arm64 app bundle; `desktop:build:x86_64` remains build-only.

## Install from this checkout

`scripts/install.sh [--dry-run] [--interface=NAME] <LAN_IPv4>` is the supported install path on macOS and Ubuntu; `scripts/install.ps1 [-DryRun] <LAN_IPv4>` is its Windows counterpart. Both run `pnpm install --frozen-lockfile`, build and install the app, preview privileged setup, and apply it once (sudo on Unix, UAC on Windows). `--dry-run`/`-DryRun` builds only the CLI and stops after the setup preview.

To do the same steps by hand:

```bash
# macOS (Apple silicon)
pnpm run desktop:build:arm64
sudo ditto "target/aarch64-apple-darwin/release/bundle/macos/Own Domains.app" "/Applications/Own Domains.app"
sudo install -m 755 target/aarch64-apple-darwin/release/own-domains /usr/local/bin/own-domains

# Ubuntu
pnpm run desktop:build:linux
sudo dpkg -i target/x86_64-unknown-linux-gnu/release/bundle/deb/*.deb

# Windows (produces an unsigned NSIS installer)
pnpm run desktop:build:windows
Start-Process target\x86_64-pc-windows-msvc\release\bundle\nsis\*-setup.exe -Wait
```

After installing, preview and apply the one-time host setup (see [First macOS activation](#first-macos-activation); Ubuntu uses the same `setup` command with `sudo -E`, Windows uses an elevated PowerShell):

```bash
own-domains setup --dry-run --lan-address <LAN_IPv4> --interface en0
sudo own-domains setup --lan-address <LAN_IPv4> --interface en0
```

```powershell
own-domains setup --dry-run --lan-address <LAN_IPv4>
own-domains setup --lan-address <LAN_IPv4>   # from an elevated prompt
```

`scripts/uninstall.sh [--dry-run] [--purge]` and `scripts/uninstall.ps1 [-DryRun] [-Purge]` remove the host integration, then the app: `/Applications/Own Domains.app` and `/usr/local/bin/own-domains` on macOS, the package on Ubuntu, the per-user NSIS install on Windows.

## First macOS activation

Setup is the only privileged operation. Preview it first, then apply it from a code-signed release CLI (ad-hoc is acceptable for local acceptance):

```bash
own-domains --state-dir "$OWN_DOMAINS_STATE_DIR" setup --dry-run \
  --lan-address <LAN_IPv4> --interface en0
sudo own-domains --state-dir "$OWN_DOMAINS_STATE_DIR" setup \
  --lan-address <LAN_IPv4> --interface en0
```

If no Caddy CA exists, setup starts the bundled Caddy as the target user with trust installation disabled, waits for the persistent root certificate, then installs only that public certificate. Setup writes `/etc/resolver/own`, the dedicated PF anchor, the PF composition block, and an owner-only install manifest with backups. Normal desktop and CLI launches never request administrator credentials.

Setup records every eligible `en*` or `bridge*` interface present at apply time, including interfaces with no current IPv4 address, and writes dynamic `(if:0)` PF rules. The schema 2 install manifest stores the covered interface names and artifact hashes but no LAN address. Changing the selected address or switching between covered interfaces is handled by the unprivileged runtime. A hardware interface that did not exist during setup requires one additional setup. A schema 1 installation is migrated by the next setup run; the old address does not need to be active.

Start routing from the GUI or CLI. For a standalone CLI, point `OWN_DOMAINS_DAEMON` at the code-signed daemon shipped in the app bundle; downloads and sidecar resolution remain resource-relative:

```bash
OWN_DOMAINS_DAEMON="/Applications/Own Domains.app/Contents/MacOS/own-domains-daemon" \
  own-domains --state-dir "$OWN_DOMAINS_STATE_DIR" start
```

The daemon validates and promotes CoreDNS/Caddy configuration, starts both sidecars and the loopback setup server, verifies readiness, and records exact process ownership. A failed mutation keeps the previous applied configuration and reports a degraded runtime. `stop` waits until every owned listener and process is gone.

## Build and verification

```bash
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
pnpm test
pnpm run lint
pnpm run typecheck
pnpm run build
pnpm run release:preflight
```

`release:preflight` verifies both architecture sidecar digests without executing macOS binaries. `release:check` is the all-architecture Developer ID distribution check: it requires `APPLE_SIGNING_IDENTITY`, runs both sidecar version commands, builds Developer ID-signed arm64 and x86_64 app bundles, and fails on missing sidecars, digest mismatches, writable nested binaries, or failed `codesign`/`spctl` assessment. The `spctl` check requires a notarized Developer ID package and a paid Apple Developer Program account. It is not required for local arm64 acceptance; use [macOS acceptance](macos-acceptance.md) for the ad-hoc path.

## Lifecycle and manual acceptance

1. Launch against a temporary native state directory. Verify the window and tray appear, an absent daemon is launched once, and the unavailable runtime error is visible on non-macOS.
2. On macOS, run the preflight and acceptance sequence in [macOS acceptance](macos-acceptance.md). Verify DNS over UDP and TCP, HTTP/HTTPS routes, the setup-page allowlist, management-route absence, and complete stop/restart.
3. Add/edit/enable/disable/remove a mapping through the GUI and CLI. Verify the open GUI updates mappings and status from the CLI change, and that a stale revision does not overwrite newer settings.
4. Use **Stop routing**; verify it stops the daemon without quitting the app, then use **Start routing** to restore it. Close the window; verify the tray remains and the daemon still answers. Restore it from the tray and launch a second GUI instance; verify it focuses the existing window.
5. Run CLI stop while the window is hidden. Verify the daemon exits and the GUI does not restart it while attempting reconnection. Restore the window and use Start routing explicitly to restart.
6. Use tray Quit or window Quit. Verify successful stop removes the daemon socket before the GUI exits. With an injected failing shutdown backend, verify the window restores and displays a retryable error.
7. Record platform/version and actual output before claiming native acceptance. The physical-phone section remains manual.

## Current verification limits

Automated Unix IPC, sidecar downloader, setup-host, process, runtime, and frontend tests pass on Linux. On macOS 15.2 arm64, the workspace suite (128 tests), desktop lifecycle suite (6 tests), frontend suite (129 tests), fmt, Clippy (including the desktop feature), lint, typecheck, build, release preflight, and desktop build also pass. An unsigned GUI smoke test confirmed window close/hide, tray Show/Quit, second-launch focus, app-menu/Cmd+Q Quit, and daemon-socket removal; the setup preview rejected an unowned `/etc/resolver/own` without host changes. The 2026-10-22 GUI check confirmed inline Delete, Stop/Start reconnection, hidden CLI stop, tray restore, second-launch focus, and tray Quit cleanup against a disposable `setupMissing` state; an injected degraded daemon confirmed Stop disconnects without quitting, a live GUI check confirmed Stop removes the real route and Start restores it with no error banner, and a forced candidate staging failure retained the previous live route while reporting degraded and keeping Stop available. Caddy-specific rejection remains covered by automated runtime tests. The live acceptance host setup was then restored with a privileged `uninstall`: the resolver, PF anchor and block, and install manifest were removed, no daemon/listeners remained, and the state CA was untrusted. An ad-hoc-signed arm64 build passed automated activation and the physical-phone pass recorded in [macOS acceptance](macos-acceptance.md), including real resolver/PF/trust changes and uninstall restoration. GUI parity manual checks are complete for this run; Intel runtime verification and interactive Linux GUI behavior remain unverified, and Intel is build-only.

## Release gate

Before any release, complete one ad-hoc-signed arm64 acceptance run and one physical-phone pass, including HTTP, trusted HTTPS, profile installation, fingerprint comparison, restart, shutdown, and uninstall restoration. Intel macOS is build-only and is not part of this gate.
