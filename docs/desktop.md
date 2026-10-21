# Native desktop development

The Tauri 2 client reuses the React components and manages the Rust daemon through private Unix IPC. On macOS it activates the bundled CoreDNS and Caddy runtime after a one-time privileged setup. Linux and Windows still return `runtimeUnavailable`. The legacy TypeScript product remains available until the macOS acceptance gate passes.

## Build prerequisites

Use Rust 1.90 or newer and pnpm. Install the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/) for your development host. macOS 13 is the first acceptance target; Linux requires GTK 3, WebKitGTK 4.1, and AppIndicator development libraries. Windows IPC is not implemented.

Approved pinned dependencies: Rust `tauri` 2.12.4, `tauri-build` 2.7.3, `tauri-plugin-single-instance` 2.5.4; JavaScript `@tauri-apps/api` 2.12.4 and development CLI 2.12.3; CoreDNS 1.14.7 and Caddy 2.11.7 sidecars.

```bash
pnpm install --frozen-lockfile
pnpm run sidecars:fetch -- --verify-only
cargo build -p own-domains-daemon -p own-domains-cli
```

`sidecars:fetch` downloads the pinned macOS release assets and verifies archive checksums, Caddy's signed checksum manifest, and each extracted executable's SHA-256 digest. Build output lives under ignored `sidecars/build/<target>/`. Run the download once without `--verify-only` to populate a new checkout.

Select the existing native settings directory explicitly. There is no desktop default relative to the launch directory and no automatic migration from legacy application data. GUI and CLI must receive the same directory. Do not run the legacy TypeScript settings writer against that directory concurrently: it does not participate in the Rust process lock.

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

`desktop:prepare` builds the daemon and stages it for Tauri's `externalBin` entry. `desktop:build` creates a release executable with the staged daemon; use the architecture-specific scripts when cross-building on macOS:

```bash
pnpm run desktop:prepare
pnpm run desktop:build
pnpm run desktop:build:arm64
pnpm run desktop:build:x86_64
```

The Tauri configuration declares both architecture sidecars and per-target runtime manifests as resources, places the daemon beside the desktop executable, and sets the macOS minimum version to 13.0. The app does not yet configure signing, notarization, or a distributable installer. Signed release verification is documented in [macOS acceptance](macos-acceptance.md).

On Linux installations where Homebrew appears before `/usr/bin` in `PATH`, select the system `pkg-config` so Cargo can find the APT-installed GTK/WebKit libraries:

```bash
PKG_CONFIG=/usr/bin/pkg-config pnpm run desktop:build
```

`desktop:build` explicitly builds without a bundle. [macOS acceptance](macos-acceptance.md) verifies signed arm64 and x86_64 app bundles.

## First macOS activation

Setup is the only privileged operation. Preview it first, then apply it from a signed release CLI:

```bash
sudo own-domains --state-dir "$OWN_DOMAINS_STATE_DIR" setup --apply \
  --lan-address 192.168.1.20 --interface en0
```

If no Caddy CA exists, setup starts the bundled Caddy as the target user with trust installation disabled, waits for the persistent root certificate, then installs only that public certificate. Setup writes `/etc/resolver/own`, the dedicated PF anchor, the PF composition block, and an owner-only install manifest with backups. Normal desktop and CLI launches never request administrator credentials.

Start routing from the GUI or CLI. For a standalone CLI, point `OWN_DOMAINS_DAEMON` at the signed daemon shipped in the app bundle; downloads and sidecar resolution remain resource-relative:

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

`release:preflight` verifies both architecture sidecar digests without executing macOS binaries. `release:check` is macOS-only: it requires `APPLE_SIGNING_IDENTITY`, runs both sidecar version commands, builds signed arm64 and x86_64 app bundles, and fails on missing sidecars, digest mismatches, writable nested binaries, or failed `codesign`/`spctl` assessment. The `spctl` check therefore also requires a notarized Developer ID package.

## Lifecycle and manual acceptance

1. Launch against a temporary native state directory. Verify the window and tray appear, an absent daemon is launched once, and the unavailable runtime error is visible on non-macOS.
2. On macOS, run the preflight and acceptance sequence in [macOS acceptance](macos-acceptance.md). Verify DNS over UDP and TCP, HTTP/HTTPS routes, the setup-page allowlist, `registry.own` management-route absence, and complete stop/restart.
3. Add/edit/enable/disable/remove a mapping through the GUI and CLI. Verify the open GUI updates mappings and status from the CLI change, and that a stale revision does not overwrite newer settings.
4. Close the window; verify the tray remains and the daemon still answers. Restore it from the tray and launch a second GUI instance; verify it focuses the existing window.
5. Run CLI stop while the window is hidden. Verify the daemon exits and the GUI does not restart it while attempting reconnection. Restore the window and use Start routing explicitly to restart.
6. Use tray Quit or window Quit. Verify successful stop removes the daemon socket before the GUI exits. With an injected failing shutdown backend, verify the window restores and displays a retryable error.
7. Record platform/version and actual output before claiming native acceptance. The physical-phone section remains manual.

## Current verification limits

Automated Unix IPC, sidecar downloader, setup-host, process, runtime, and frontend tests pass on Linux. The feature-enabled Linux Tauri shell compiles with the documented system `pkg-config` selection. macOS window/tray behavior, real resolver/PF/trust changes, signed app verification, interactive Linux GUI behavior, and physical-device acceptance remain unverified. No DNS/proxy service changes were performed in this repository session.

## Release gate

Before retiring browser management, complete the signed arm64 and x86_64 acceptance runs and one physical-phone pass, including HTTP, trusted HTTPS, profile installation, fingerprint comparison, restart, shutdown, and uninstall restoration. Then remove `registry.own` and the legacy server/CLI entry points in a separate reviewed change with explicit deletion authorization. Do not remove the existing installer or onboarding server prematurely.
