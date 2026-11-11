# Setup

Own Domains runs its DNS and proxy through the native daemon plus the Rust CLI (`own-domains setup`, `own-domains doctor`). This page covers the manual steps that still apply around it: router, phone DNS, and firewall.

Start the desktop app during development with `pnpm run desktop:dev` (`pnpm run dev` serves the dashboard in a browser, without a daemon connection).

After building and applying setup, open the installed app. Under **Network settings**, enter the same LAN address used for setup, save it, and refresh the page. This persists the address required by the app's DNS diagnostics. The setup page remains reachable at `http://127.0.0.1:4781` on the local machine.

Preview host changes before applying them (`--dry-run` without administrator rights):

```sh
own-domains setup --dry-run --lan-address <LAN_IPv4> --interface en0
sudo own-domains setup --lan-address <LAN_IPv4> --interface en0
```

Replace `<LAN_IPv4>` with the machine's reserved LAN IPv4 address. Applying the changes requires administrator approval; setup creates backups and a hash-based installation manifest before replacing managed files. (`--interface` may be omitted on Windows and Ubuntu; on macOS pass your interface, usually `en0`. On Ubuntu use `sudo -E` to preserve the environment.)

For phone access, reserve the machine's LAN IPv4 address in the router, configure that address as the phone's only DNS server for the Wi-Fi network, and allow ports 53, 80, and 443 through the machine firewall. Open `http://setup.own` on the phone to install the public local root certificate and enable full SSL trust. Never copy the CA private key.

`.own` is private local convention rather than an IANA-reserved special-use suffix. Restore automatic phone DNS whenever the machine running Own Domains is unavailable.

Preview removal with `own-domains uninstall --dry-run` and apply it with `sudo own-domains uninstall` (add `-E` on Ubuntu). Uninstall restores backed-up files or removes files that still match the installation manifest; files changed after setup are preserved for manual review. Independent cleanup continues after errors and returns a nonzero status after reporting every failure. Pass `--purge` to also remove application data and Caddy CA files; `--purge` cannot be combined with `--dry-run`. Device trust settings are never removed automatically.
