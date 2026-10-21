# macOS runtime acceptance

This checklist is the release gate for the Rust runtime. It has not been executed on macOS hardware in this repository session; treat every item as unverified until the recorded log and manual results exist.

## Required hardware and inputs

- Apple silicon macOS 13 or newer and Intel macOS 13 or newer, each with a signed matching-architecture package.
- A physical phone on the same LAN.
- A reserved Mac LAN IPv4 address and its interface, for example `192.168.1.20` and `en0`.
- Test HTTP and HTTPS upstream servers on IPv4 loopback ports. The acceptance runner only records the HTTP port; use an HTTPS mapping only after the test server serves the expected host.
- A disposable absolute state directory under the logged-in user's home. The default runner path is under `~/Library/Application Support`.
- The legacy TypeScript runtime and installer unloaded before the run. Preflight rejects an existing owned resolver, PF block, or fixed-port listener.
- `APPLE_SIGNING_IDENTITY` for the release-signature check.

## 1. Release preflight

Run on each architecture from the repository checkout:

```bash
pnpm test -- tests/scripts/fetch-sidecars.test.ts
pnpm run sidecars:fetch -- --verify-only
pnpm run release:preflight
PKG_CONFIG=/usr/bin/pkg-config pnpm run desktop:build
pnpm run release:check
```

`release:check` runs CoreDNS and Caddy version commands, builds signed arm64 and x86_64 app bundles, compares sidecar digests with the packaged manifests, verifies nested executables are not writable, and runs `codesign` and `spctl`; the latter requires a notarized Developer ID package. Do not continue if either architecture is absent or any signature check fails.

Record the commands, package paths, architecture, macOS version, and output. The runner also records machine-readable JSON lines.

### Standalone CLI package

The release check verifies sidecars and app bundles; assemble the signed CLI used by the runner from the same target:

```bash
export TARGET="aarch64-apple-darwin"   # or x86_64-apple-darwin
export CLI_DIR="$PWD/target/$TARGET/release/own-domains-cli"
cargo build --release --target "$TARGET" -p own-domains-cli
mkdir -p "$CLI_DIR/sidecars"
cp "target/$TARGET/release/own-domains" "$CLI_DIR/own-domains"
cp "sidecars/build/$TARGET/coredns-$TARGET" "sidecars/build/$TARGET/caddy-$TARGET" "$CLI_DIR/"
cp "sidecars/build/$TARGET/manifest.json" "$CLI_DIR/sidecars/manifest-$TARGET.json"
codesign --force --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" \
  "$CLI_DIR/own-domains" "$CLI_DIR/coredns-$TARGET" "$CLI_DIR/caddy-$TARGET"
codesign --verify --strict "$CLI_DIR/own-domains"
```

The CLI resolves sidecars relative to its own directory, and the runner points `OWN_DOMAINS_DAEMON` at the signed daemon in the app bundle. Do not copy an unsigned development daemon into the acceptance path.

## 2. Automated activation sequence

Use a signed CLI from the matching package. Run preflight first; it does not change the host:

```bash
scripts/verify-macos-runtime.sh \
  --app "/Applications/Own Domains.app" \
  --cli "/path/to/own-domains" \
  --state-dir "$HOME/Library/Application Support/OwnDomains-acceptance" \
  --lan-address 192.168.1.20 \
  --interface en0
```

The preflight records OS/architecture, checks the app and CLI signatures, verifies CoreDNS/Caddy hashes and executable modes, checks the macOS 13 minimum, confirms no conflicting resolver/PF block or fixed-port listeners, and runs the setup dry-run when the LAN inputs are present. It exits nonzero on a conflict.

After reviewing the log, apply the reversible sequence:

```bash
scripts/verify-macos-runtime.sh \
  --accept \
  --app "/Applications/Own Domains.app" \
  --cli "/path/to/own-domains" \
  --state-dir "$HOME/Library/Application Support/OwnDomains-acceptance" \
  --hostname acceptance.own \
  --lan-address 192.168.1.20 \
  --interface en0 \
  --upstream-port 3000
```

The runner performs:

1. Setup dry-run, then privileged setup apply.
2. CLI-only runtime start and waits for TCP 5354, 8080, 8443, and 4781.
3. UDP and TCP DNS for the test hostname through `127.0.0.1:5354`, plus the `registry.own` HTTP negative check. DNS intentionally wildcard-resolves `.own` names to the Mac; the retired management route must return 404.
4. `setup.own` public routes, private-route rejection with 404, and fingerprint presence.
5. HTTP and HTTPS routes through the LAN address, including CA verification for HTTPS.
6. A stale-revision rejection and mapping add/remove through the CLI.
7. Stop with listener negatives, restart, stop again, and final uninstall restoration of `/etc/resolver/own` and the PF block.

It never removes the acceptance state unless `--purge` is supplied. Supply `--purge` only for a disposable state after the run succeeds.

The GUI parity check is intentionally manual because it needs the interactive app. The runner records `manual` for that step.

## 3. Manual GUI checks

1. Launch the app against the same state directory and confirm the daemon starts once.
2. Compare mapping and status displays with `own-domains --state-dir ... --json status`.
3. Add, edit, disable, enable, and remove a mapping in the GUI; repeat through the CLI and confirm the open GUI updates without manual refresh.
4. Try a stale revision through the CLI; confirm the GUI keeps the newer settings.
5. Force a candidate failure with an injected test backend or a deliberately invalid test configuration in a disposable state; confirm the previous route remains, the runtime reports degraded, and Stop remains available.
6. Close the window; confirm the tray stays and the daemon answers. Restore from the tray and verify a second launch focuses the existing window.
7. Run CLI stop while hidden; confirm the GUI does not restart the daemon automatically. Use Start routing explicitly.
8. Quit from the tray and confirm the daemon socket and owned listeners disappear before the app exits, or an actionable retry error appears.

## 4. Physical-phone checks

Record these results separately from the automated log:

1. Connect the phone to the same Wi-Fi and set DNS to the Mac LAN address. Disable conflicting VPN/private DNS behavior.
2. Open `http://setup.own`; verify the page, certificate, profile, and fingerprint are reachable.
3. Install the iOS profile and enable the root certificate under **Settings → General → About → Certificate Trust Settings**.
4. Compare the displayed fingerprint with the certificate downloaded from the Mac.
5. Open an HTTP mapping and a trusted HTTPS mapping from the phone.
6. Confirm `registry.own` and management/private routes return no management response.
7. Restart routing from the GUI or CLI; confirm phone DNS and routing recover without reinstalling trust.
8. Stop routing; confirm phone requests fail and no owned process or listener remains.
9. Uninstall; confirm the resolver, PF block, and trust entry are gone. Remove the phone profile manually.

## Pass/fail gate

The runtime is accepted only when all of the following are recorded:

- Sidecar and package hashes match on both architectures.
- `codesign` and `spctl` pass for both app bundles.
- Automated preflight and activation logs show every step passed on Apple silicon and Intel macOS 13+.
- GUI and physical-phone sections pass on both architectures.
- Stop leaves no owned process or listener; restart and uninstall restore the previous host state.
- The setup-page allowlist exposes only `/`, `/ca.crt`, `/ca.mobileconfig`, and `/fingerprint`; `registry.own` has no management route (HTTP 404).

Any missing item means Phase 4 remains in progress. Keep the TypeScript runtime, installer, and `registry.own` compatibility paths in place.

## Recovery

If a step fails, keep the JSONL log and command log. The state directory contains the install manifest, backups, runtime configuration, and Caddy CA. Do not delete it until privileges and routes are understood.

Preview uninstall before retrying:

```bash
sudo own-domains --state-dir "$OWN_DOMAINS_STATE_DIR" uninstall
sudo own-domains --state-dir "$OWN_DOMAINS_STATE_DIR" uninstall --apply
```

If setup failed before the install manifest existed, inspect `/etc/resolver/own`, `/etc/pf.conf`, `/etc/pf.anchors/dev.owndomains`, and the trust store; setup rollback should have restored byte-identical files, but the log is authoritative. Never hand-edit PF or trust entries without recording the original contents.
