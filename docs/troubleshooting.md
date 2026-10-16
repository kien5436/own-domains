# Troubleshooting

- **dnsmasq listener warning after first setup:** open **Network settings**, enter the same Mac LAN address used by `install.sh`, save it, and refresh the dashboard. If the warning remains, run `dig +short @127.0.0.1 healthcheck.own A` and `dig +short @<MAC_LAN_IP> healthcheck.own A`; both must return the Mac LAN address.
- **Domain does not resolve:** confirm the phone can reach the Mac, uses the Mac as its only Wi-Fi DNS server, and is not on an isolated guest network or overriding DNS through a VPN/private DNS feature.
- **502 response:** start the target app on the configured IPv4 loopback port. Some development servers must allow the custom `.own` Host header.
- **HTTPS warning:** install Caddy's public root certificate on the device and explicitly enable SSL trust. Certificate issuance alone does not establish device trust.
- **Works on the Mac only:** check firewall access to TCP 80/443 and UDP/TCP 53, then confirm the Mac's LAN address has not changed.
- **Configuration pending:** keep the previous working route, inspect the reported Caddy or DNS validation error, fix it, and reconcile again.

Run `pnpm run doctor` for a local prerequisite report. Run `./scripts/uninstall.sh --dry-run` to preview reversible removal steps. A `PRESERVE` result means the file changed after installation and must be reviewed manually. Remove the Caddy root certificate separately from every phone or computer where it was trusted; use `./scripts/uninstall.sh --purge` to remove local CA data.
