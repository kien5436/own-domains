# Own Domains

Own Domains registers private `.own` hostnames for web applications running on your Mac. It provides a React dashboard, dnsmasq DNS, Caddy reverse proxying, optional local HTTPS, and access from devices on the same network.

The native Tauri client and Rust CLI are documented in [Desktop development](docs/desktop.md) and [Native CLI](docs/cli.md). The macOS runtime can activate bundled CoreDNS and Caddy after privileged setup; Linux and Windows remain `runtimeUnavailable`. The legacy TypeScript runtime stays available until the [macOS acceptance gate](docs/macos-acceptance.md) passes.

## Native macOS runtime (in progress)

The native client packages pinned CoreDNS and Caddy sidecars, verifies their digests, and resolves them from the app bundle. A one-time privileged setup installs the `.own` resolver, a dedicated PF anchor, and the Caddy root certificate; normal launches do not need administrator access. See [macOS acceptance](docs/macos-acceptance.md) for the signed-package and physical-phone gate. Do not remove the legacy installation before that gate is recorded.

## Requirements

- macOS with [Homebrew](https://brew.sh/)
- A reserved LAN IPv4 address for the Mac
- Ports 53, 80, and 443 available on that address

Install the required tools:

```bash
brew install node pnpm caddy dnsmasq
```

## Installation

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
