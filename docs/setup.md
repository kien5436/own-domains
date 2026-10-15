# Setup

Own Domains requires Caddy, dnsmasq, and administrator-approved macOS service configuration. Start the API and dashboard during development with `pnpm run dev:server` and `pnpm run dev` in separate terminals.

Run `pnpm run setup -- --dry-run` to preview host changes and `pnpm run doctor` to inspect prerequisites. Automated privileged installation is intentionally disabled until its generated files and rollback behavior have been reviewed on the target Mac.

For phone access, reserve the Mac's LAN IPv4 address in the router, configure that address as the phone's only DNS server for the Wi-Fi network, and allow ports 53, 80, and 443 through the Mac firewall. Install Caddy's public local root CA certificate on the phone and enable full SSL trust. Never copy the CA private key.

`.own` is private local convention rather than an IANA-reserved special-use suffix. Restore automatic phone DNS whenever the Mac is unavailable.
