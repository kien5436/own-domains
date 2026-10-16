# Setup

Own Domains requires Caddy, dnsmasq, and administrator-approved macOS service configuration. Start the API and dashboard during development with `pnpm run dev:server` and `pnpm run dev` in separate terminals.

After building and applying setup, open the installed dashboard at `http://registry.own`. Under **Network settings**, enter the same LAN address used for setup, save it, and refresh the page. This persists the address required by dashboard DNS diagnostics. Direct access at `http://127.0.0.1:4780` remains available on the Mac.

Run `pnpm run setup -- --dry-run --lan-address 192.168.1.20` to preview host changes and `pnpm run doctor` to inspect prerequisites. Replace the example address with the Mac's reserved LAN IPv4 address. Applying the reviewed files requires the explicit `--apply` flag and administrator approval; setup creates backups and a hash-based installation manifest before replacing managed files.

For phone access, reserve the Mac's LAN IPv4 address in the router, configure that address as the phone's only DNS server for the Wi-Fi network, and allow ports 53, 80, and 443 through the Mac firewall. Install Caddy's public local root CA certificate on the phone and enable full SSL trust. Never copy the CA private key.

`.own` is private local convention rather than an IANA-reserved special-use suffix. Restore automatic phone DNS whenever the Mac is unavailable.

Preview removal with `./scripts/uninstall.sh --dry-run` and apply it with `./scripts/uninstall.sh`. Uninstall restores backed-up files or removes files that still match the installation manifest; files changed after setup are preserved for manual review. Independent cleanup continues after errors and returns a nonzero status after reporting every failure. Pass `--purge` to also remove application data and Caddy CA files. Device trust settings are never removed automatically.
