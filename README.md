# Own Domains

Own Domains registers private `.own` hostnames for web applications running on your Mac. It provides a React dashboard, dnsmasq DNS, Caddy reverse proxying, optional local HTTPS, and access from devices on the same network.

The native Tauri client and Rust CLI are documented in [Desktop development](docs/desktop.md) and [Native CLI](docs/cli.md). The first macOS beta can activate bundled CoreDNS and Caddy after privileged setup on Apple silicon macOS 15+. Ad-hoc-signed arm64 acceptance and physical-phone checks passed on 2026-10-22; Developer ID signing and notarization remain distribution work. Intel macOS is buildable but not acceptance-required. Ubuntu 24+ and Windows 10/11 are support targets whose runtime adapters are not implemented, so the native runtime reports `runtimeUnavailable` there. The legacy TypeScript runtime remains available for existing installations.

## Platform support

| Platform | Native runtime status |
| --- | --- |
| Apple silicon macOS 15+ | Native beta; implemented |
| Intel macOS 15+ | Buildable; not acceptance-required |
| Ubuntu 24+ | Support target; runtime not implemented |
| Windows 10/11 | Support target; IPC and runtime not implemented |

`v0.1.0-beta.1` is a source release. No Developer ID-signed or notarized binary is published. Local acceptance uses an ad-hoc-signed arm64 package and does not require an Apple Developer Program account. Developer ID signing and notarization are required only for distribution. See [Platform support](docs/platform-support.md) for the full matrix.

## Native macOS beta

The native client packages pinned CoreDNS and Caddy sidecars, verifies their digests, and resolves them from the app bundle. A one-time privileged setup installs the `.own` resolver, a dedicated PF anchor, and the Caddy root certificate. Setup covers every eligible LAN interface present at that time, including interfaces without a current IPv4 address, and writes dynamic `(if:0)` PF rules. Changing the selected LAN address or switching between covered interfaces needs no administrator access; adding a previously absent hardware interface may require one more setup. The install manifest stores covered interface names but no LAN address. An existing schema 1 setup is migrated by running setup once. The [macOS acceptance gate](docs/macos-acceptance.md) passed for an ad-hoc-signed arm64 package, including automated routing, GUI lifecycle, and physical-phone checks. Do not remove the legacy source or its migration/uninstall support while beta users still depend on it.

## Requirements

- macOS 15+; Apple silicon is required for native acceptance, while Intel is buildable
- [Homebrew](https://brew.sh/) for the legacy runtime
- A reserved LAN IPv4 address for the Mac
- Ports 53, 80, and 443 available on that address

Install the required tools:

```bash
brew install node pnpm caddy dnsmasq
```

## Installation

The commands in this section install the legacy TypeScript runtime. Native desktop development and acceptance are documented in [Desktop development](docs/desktop.md).

Clone the repository, then run the installer with the Mac's LAN address:

```bash
git clone <repository-url>
cd domains
./scripts/install.sh 192.168.1.20
```

Find the address with `ipconfig getifaddr en0` or, when needed, `ipconfig getifaddr en1`. The script installs locked dependencies, builds the dashboard, previews system changes, and requests administrator approval before installing launchd services.

After installation:

1. Open `http://registry.own`.
2. Under **Network settings**, enter the same LAN address passed to `install.sh`.
3. Keep or change the upstream DNS addresses, then click **Save network settings**.
4. Refresh the dashboard and confirm that the LAN address and dnsmasq checks are healthy.

Use `pnpm doctor` to check prerequisites.

## Register and Run a Website

Start the website on an IPv4 loopback port, for example:

```bash
cd ~/projects/my-site
pnpm dev -- --host 127.0.0.1 --port 3000
```

Open `http://registry.own`, register `my-site.own` with port `3000`, and choose whether to enable HTTPS. Then visit `http://my-site.own` or `https://my-site.own`. The app must remain running on the registered port and accept the custom Host header.

## Phone Access and HTTPS

Connect the phone to the same Wi-Fi and set its DNS server to the Mac's LAN address. Disable conflicting VPN/private DNS handling or configure `.own` as a local-domain fallback. Visit `http://setup.own` to download the iOS profile or root certificate. On iOS, install the profile and enable it under **Settings → General → About → Certificate Trust Settings**.

## Removal

Preview removal, then uninstall the services and managed files:

```bash
./scripts/uninstall.sh --dry-run
./scripts/uninstall.sh
```

For a clean first-run test, also erase saved mappings, backups, and the local Caddy CA:

```bash
./scripts/uninstall.sh --purge
```

Uninstall is best-effort: independent cleanup steps continue after an error, every failure is reported, and the script returns a nonzero status at the end if anything failed. Trusted certificates installed on phones or other computers must be removed manually on those devices.

## Troubleshooting

If the dashboard says **dnsmasq is not listening on the required port 53 addresses** immediately after installation, first check **Network settings**. Enter the Mac LAN address used during installation, save it, and refresh the page. The dashboard cannot run its LAN DNS probe until this value is saved.

If the warning remains, run:

```bash
pnpm doctor
dig +short @127.0.0.1 healthcheck.own A
dig +short @<MAC_LAN_IP> healthcheck.own A
```

Both `dig` commands should return the Mac LAN address. If they do not, rerun `./scripts/install.sh <MAC_LAN_IP>` to validate the configuration and restart the managed services.
