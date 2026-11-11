# Own Domains

Own Domains gives your computer private `.own` web addresses (like `my-site.own`) for the apps you build. It runs a small DNS server and a web proxy on your machine, so browsers on your computer — and phones on the same Wi-Fi — can open your local projects by name, with optional HTTPS.

## Contents

- [Supported platforms](#supported-platforms)
- [Requirements](#requirements)
- [Prerequisites](#prerequisites)
- [Install](#install)
- [Use on your computer](#use-on-your-computer)
- [Use on your phone](#use-on-your-phone)
- [Update and uninstall](#update-and-uninstall)
- [Troubleshooting](#troubleshooting)
- [Learn more](#learn-more)

## Supported platforms

| Platform | Status |
| --- | --- |
| Apple silicon macOS 15+ | Stable, fully working |
| Intel macOS 15+ | Builds; not acceptance-tested |
| Windows 10/11 (x64) | Stable (NSIS installer) |
| Ubuntu 24+ | Working native runtime (`.deb` + `setup`) |

Local testing uses an ad-hoc-signed macOS app; no paid Apple Developer account is needed. Signed, notarized releases are distribution work only. See `docs/platform-support.md` for the full matrix.

## Requirements

- A reserved LAN IPv4 address for your machine, so it does not change.
- Ports 53 (DNS), 80 (HTTP), and 443 (HTTPS) free on that address.
- Your phone and computer on the same Wi-Fi (only needed for phone access).
- Administrator access, one time, for the network setup step.

## Prerequisites

Building from source needs:

- [Rust](https://www.rust-lang.org/tools/install) 1.90 or newer
- [pnpm](https://pnpm.io/installation) with Node.js
- [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/) for your system (Ubuntu also needs GTK 3, WebKitGTK 4.1, and AppIndicator dev libraries)

The installer runs `pnpm install` for you. Full build details, sidecar fetching, and per-architecture commands are in `docs/desktop.md`; Ubuntu specifics are in `docs/linux-install.md`.

## Install

Run the installer from the repository root with your machine's reserved LAN IPv4 address:

```sh
# macOS (interface required, usually en0 for Wi-Fi)
./scripts/install.sh --interface=en0 <LAN_IPv4>

# Ubuntu (interface may be omitted; all interfaces are then covered)
./scripts/install.sh <LAN_IPv4>

# Windows (PowerShell)
powershell -ExecutionPolicy Bypass -File scripts\install.ps1 <LAN_IPv4>
```

The script installs dependencies, builds the desktop app and CLI, installs the app (`/Applications/Own Domains.app` on macOS, the `.deb` package on Ubuntu, the per-user NSIS installer on Windows), previews the host setup, and then applies it once. On macOS and Ubuntu that means `sudo`; on Windows it means one UAC elevation. That setup installs the `.own` resolver rule, firewall entries, and the local root certificate. Everything else — builds, previews, and running the app — stays unprivileged.

The app and the CLI install independently. By default you get both; pass `--cli-only` for just the self-contained CLI prefix (`<prefix>/lib/own-domains` with an `own-domains` symlink on PATH, `/usr/local` by default, overridable with `--prefix=DIR`), or `--app-only` for just the desktop bundle. Either one works on its own afterwards. The same flags exist on `scripts/uninstall.sh`.

Preview without changing the system (builds only the CLI, no elevation):

```sh
./scripts/install.sh --dry-run --interface=en0 <LAN_IPv4>
powershell -ExecutionPolicy Bypass -File scripts\install.ps1 -DryRun <LAN_IPv4>
```

Re-running setup is only needed when a new network interface appears; changing the LAN address afterwards does not need administrator access.

Check everything is healthy:

```sh
own-domains status
sudo own-domains doctor
```

`doctor` run without administrator shows `*Unverifiable` warnings for items it cannot read — that is normal, not a failure.

## Use on your computer

**With the app:** open Own Domains, add a site (for example `my-site.own`, port `3000`), and choose **Start**. Open `http://my-site.own` in your browser.

**With the command line:**

```sh
own-domains add my-site.own --port 3000 --revision 0
own-domains start
own-domains status
own-domains list
```

Use the revision number shown by `status` for every change (`edit`, `enable`, `disable`, `remove`). Add `--https` when registering if you want `https://my-site.own`. The app you are building must be running on that port on your computer, for example:

```sh
pnpm dev -- --host 127.0.0.1 --port 3000
```

Some dev servers need to allow the custom `.own` Host header.

Stop routing any time with `own-domains stop` or the **Stop** button. The app's settings live in the platform folder (`~/Library/Application Support/OwnDomains` on macOS, `%LOCALAPPDATA%\OwnDomains` on Windows, `~/.local/state/own-domains` on Linux); pass the same `--state-dir` to the app and CLI if you use a custom one.

## Use on your phone

Your phone must be on the same Wi-Fi as your computer. Point the phone's Wi-Fi DNS at your computer's LAN address, then open `http://setup.own` on the phone to install the profile/certificate and enable trust. See `docs/setup.md` for the per-system steps.

If `.own` names stop working on the phone, switch its DNS back to automatic.

## Update and uninstall

To update, re-run the install script with the same LAN address and interface (the Windows script takes the same address), then open the app.

To remove the host setup and the installed app (settings are kept for a later reinstall):

```sh
# macOS / Ubuntu
./scripts/uninstall.sh

# Windows
powershell -ExecutionPolicy Bypass -File scripts\uninstall.ps1
```

To also delete settings, backups, logs, and the local certificate authority:

```sh
# macOS / Ubuntu
./scripts/uninstall.sh --purge

# Windows
powershell -ExecutionPolicy Bypass -File scripts\uninstall.ps1 -Purge
```

Preview the removal without changing anything (no elevation):

```sh
# macOS / Ubuntu
./scripts/uninstall.sh --dry-run

# Windows
powershell -ExecutionPolicy Bypass -File scripts\uninstall.ps1 -DryRun
```

Certificates you trusted on phones or other computers must be removed on those devices by hand.

## Troubleshooting

- **Dashboard says DNS is not listening:** open Network settings in the app, enter the same LAN address used during setup, save, and refresh.
- **Site does not resolve:** the computer and phone must be on the same Wi-Fi, the phone must use only your computer as its DNS, and no VPN or private-DNS feature may override it.
- **`502` error:** start your app on the registered port and let it accept the `.own` Host header.
- **HTTPS warning on a device:** install the root certificate from `http://setup.own` and explicitly enable trust for it.
- **Works on the computer but not the phone:** allow ports 53, 80, 443 through the computer firewall and check the LAN address has not changed.
- **Still stuck:** run `sudo own-domains doctor` and read `docs/troubleshooting.md`.

## Learn more

- `docs/setup.md` — setup details and phone DNS steps
- `docs/troubleshooting.md` — full problem list
- `docs/desktop.md` — building and developing the desktop app
- `docs/cli.md` — every CLI command, flags, and exit codes
- `docs/platform-support.md` — platform matrix and signing policy
- `docs/linux-install.md` — Ubuntu install walkthrough
- `docs/macos-acceptance.md`, `docs/windows-acceptance.md` — acceptance records
