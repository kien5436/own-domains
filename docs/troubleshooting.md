# Troubleshooting

- **Domain does not resolve:** confirm the phone can reach the Mac, uses the Mac as its only Wi-Fi DNS server, and is not on an isolated guest network or overriding DNS through a VPN/private DNS feature.
- **502 response:** start the target app on the configured IPv4 loopback port. Some development servers must allow the custom `.own` Host header.
- **HTTPS warning:** install Caddy's public root certificate on the device and explicitly enable SSL trust. Certificate issuance alone does not establish device trust.
- **Works on the Mac only:** check firewall access to TCP 80/443 and UDP/TCP 53, then confirm the Mac's LAN address has not changed.
- **Configuration pending:** keep the previous working route, inspect the reported Caddy or DNS validation error, fix it, and reconcile again.

Run `pnpm run doctor` for a local prerequisite report. Run `pnpm run uninstall -- --dry-run` to preview reversible removal steps.
