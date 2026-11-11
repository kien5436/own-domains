# Troubleshooting

Host setup and diagnostics run through the native daemon and the Rust CLI. Device-side steps (phone DNS, CA trust, firewall) are listed below.

- **DNS listener warning after first setup:** open **Network settings**, enter the same LAN address used during setup, save it, and refresh the app. If the warning remains, resolve one of your own names locally and over the LAN with `dig +short @127.0.0.1 <site>.own A` and `dig +short @<LAN_IPv4> <site>.own A`; both must return the LAN address.
- **Domain does not resolve:** confirm the phone can reach the machine, uses the machine as its only Wi-Fi DNS server, and is not on an isolated guest network or overriding DNS through a VPN/private DNS feature.
- **502 response:** start the target app on the configured IPv4 loopback port. Some development servers must allow the custom `.own` Host header.
- **HTTPS warning:** install the public root certificate from `http://setup.own` on the device and explicitly enable SSL trust. Certificate issuance alone does not establish device trust.
- **Works on the machine only:** check firewall access to TCP 80/443 and UDP/TCP 53, then confirm the LAN address has not changed.
- **Configuration pending:** keep the previous working route, inspect the reported Caddy or DNS validation error, fix it, and reconcile again.

Run `sudo own-domains doctor` for a prerequisite report (without administrator rights, items it cannot read show as `*Unverifiable`, which is normal). Run `own-domains uninstall --dry-run` to preview reversible removal steps. Remove the root certificate separately from every phone or computer where it was trusted; use `sudo own-domains uninstall --purge` to remove local CA data.
