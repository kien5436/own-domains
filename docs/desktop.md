# Native desktop development

The Tauri 2 client reuses the React components and manages the Rust daemon through private Unix IPC. It is an experimental management client: the platform runtime still returns `runtimeUnavailable`, so it cannot activate DNS or Caddy yet. The legacy TypeScript product remains available.

## Build prerequisites

Use Rust 1.90 or newer and pnpm. Install the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/) for your development host. macOS is the first acceptance target; Linux requires GTK 3, WebKitGTK 4.1, and AppIndicator development libraries. Windows IPC is not implemented.

Approved pinned dependencies: Rust `tauri` 2.12.4, `tauri-build` 2.7.3, `tauri-plugin-single-instance` 2.5.4; JavaScript `@tauri-apps/api` 2.12.4 and development CLI 2.12.3.

```bash
pnpm install --frozen-lockfile
cargo build -p own-domains-daemon -p own-domains-cli
```

Select the existing native settings directory explicitly. There is no desktop default relative to the launch directory and no automatic migration from legacy application data. GUI and CLI must receive the same directory. Do not run the legacy TypeScript settings writer against that directory concurrently: it does not participate in the Rust process lock.

```bash
export OWN_DOMAINS_STATE_DIR=/absolute/path/to/native-state
export OWN_DOMAINS_DAEMON=/absolute/path/to/own-domains/target/debug/own-domains-daemon
pnpm run desktop:dev
```

The daemon executable override is a trusted launcher setting, never a WebView argument. Without it, the desktop expects `own-domains-daemon` beside its executable. The desktop binary also accepts `--state-dir /absolute/path` and optional `--socket /absolute/path/daemon.sock`; do not point the socket at another state directory's daemon.

```bash
cargo run -p own-domains-cli -- --state-dir "$OWN_DOMAINS_STATE_DIR" status
cargo run -p own-domains-cli -- --state-dir "$OWN_DOMAINS_STATE_DIR" add shop.own --port 3000 --revision 0
```

Use the actual revision reported by status for edits. A conflict reloads state without retrying or overwriting the competing change.

## Build and verification

```bash
pnpm run desktop:build
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
pnpm test
pnpm run lint
pnpm run typecheck
pnpm run build
```

On Linux installations where Homebrew appears before `/usr/bin` in `PATH`, select the system `pkg-config` so Cargo can find the APT-installed GTK/WebKit libraries:

```bash
PKG_CONFIG=/usr/bin/pkg-config pnpm run desktop:build
```

`desktop:build` explicitly enables the `desktop` feature and builds the native executable without a distributable installer. The default Rust workspace tests exercise the bridge, subscriptions, and lifecycle without requiring a WebView. They do not prove the shell builds; run `cargo check -p own-domains-desktop --features desktop` on a host with the system libraries installed. Release builds must place the matching release daemon next to the desktop executable (or configure its absolute path). Installer packaging, signing, and service deployment are Phase 4 work.

The Vite page outside Tauri continues to use the legacy HTTP adapter. Inside Tauri there is no HTTP fallback: all domain, network, and status operations use the local bridge. Capabilities permit only the main local window's daemon commands and event listeners. Release CSP has no management HTTP origin. See [Tauri capability scoping](https://v2.tauri.app/security/capabilities/).

## Lifecycle and manual acceptance

1. Launch against a temporary native state directory. Verify the window and tray appear, an absent daemon is launched once, and the unavailable runtime error is visible.
2. Add/edit/enable/disable/remove a mapping, toggle TLS, and update network settings. Verify changes persist without claiming routing is active.
3. Mutate settings through the CLI with the current revision; verify the open GUI updates both mappings and status. Try a stale revision and confirm it does not overwrite the newer settings.
4. Close the window; verify the tray remains and the daemon still answers. Restore it from the tray and launch a second GUI instance; verify it focuses the existing window.
5. Run CLI stop while the window is hidden. Verify the daemon exits and the GUI does not restart it while attempting reconnection. Restore the window and use Start routing explicitly to restart.
6. Use tray Quit or window Quit. Verify successful stop removes the daemon socket before the GUI exits. With an injected failing shutdown backend, verify the window restores and displays a retryable error.
7. On macOS, repeat the checks using a release binary. Record platform/version and actual output before claiming native acceptance.

## Current verification limits

Automated Unix IPC and frontend tests are available, and the feature-enabled Linux Tauri shell compiles with the documented system `pkg-config` selection. macOS window/tray behavior, interactive Linux GUI behavior, and physical-device acceptance remain unverified. No DNS/proxy service changes were performed.

## Phase 4 cutover gate

Before retiring browser management, implement daemon-owned `setup.own` with an explicit GET/HEAD route allowlist for phone instructions, public CA certificate, installation profile, and public fingerprint. Reject all management/private-key/local-path/arbitrary-proxy requests. Prove host-service shutdown, restart, settings/CA preservation, and physical-phone HTTP/trusted HTTPS operation. Then remove `registry.own` and legacy server/CLI entry points with explicit deletion authorization. This client does not remove the existing installer or onboarding server prematurely.
