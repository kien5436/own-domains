# macOS runtime acceptance

This checklist is the release gate for the native macOS arm64 runtime. The 2026-10-21 run passed automated arm64 activation and the physical-phone checks, including profile removal and DNS restoration. GUI parity checks passed on 2026-10-22: lifecycle, Delete, live routing restoration, forced candidate-failure route retention, and privileged uninstall restoration below. Caddy-specific rejection remains covered by the automated runtime tests. The 2026-10-25 network-independent routing record covers schema 2 migration and address/interface switching. The 2026-10-26 physical-phone record covers the section 5 DNS, HTTP, trusted HTTPS, setup-route, fingerprint, and stop/start checks against the schema 2 anchor. Live uninstall and profile removal were not repeated against schema 2; the 2026-10-21 uninstall record and the automated setup/uninstall tests remain the evidence for that path. Neither record covers distribution signing.

## Required hardware and inputs

- Apple silicon Mac running macOS 15 or newer with a code-signed arm64 app and CLI.
- A physical phone on the same LAN.
- A reserved Mac LAN IPv4 address and its interface, for example `192.168.1.20` and `en0`.
- Test HTTP and HTTPS upstream servers on IPv4 loopback ports. The acceptance runner only records the HTTP port; use an HTTPS mapping only after the test server serves the expected host.
- A disposable absolute state directory under the logged-in user's home. The default runner path is under `~/Library/Application Support`.
- The legacy TypeScript runtime and installer unloaded before the run. Preflight rejects an existing owned resolver, PF block, or fixed-port listener.

Ad-hoc signing is sufficient for local acceptance and does not require an Apple Developer Program account. `codesign --verify` must pass for the app, CLI, and sidecars. Gatekeeper `spctl`, Developer ID signing, and notarization are distribution requirements, not acceptance gates. Intel macOS is buildable but is not part of this gate.

## 1. Build and code-sign the arm64 package

Run on the Apple silicon host from the repository checkout:

```bash
pnpm test -- tests/scripts/fetch-sidecars.test.ts
pnpm run sidecars:fetch -- --arch all
pnpm run release:preflight
pnpm run desktop:build:arm64
(
  cd crates/own-domains-desktop
  pnpm exec tauri build --target aarch64-apple-darwin --bundles app
)
export APP="$PWD/target/aarch64-apple-darwin/release/bundle/macos/Own Domains.app"
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
```

Do not modify the bundle after signing. Either pass this path with `--app` or copy the signed bundle to `/Applications` before preflight; copying preserves the signature.

`release:preflight` verifies both sidecar digest sets without executing either architecture; downloading and checking the Intel sidecars is not an Intel acceptance run. `release:check` is the separate all-architecture Developer ID distribution check: it requires `APPLE_SIGNING_IDENTITY`, and its `spctl` step requires notarization and a paid Apple Developer Program account. It is not required for local acceptance.

Record the commands, package paths, architecture, macOS version, and output. The runner also records machine-readable JSON lines.

### Standalone CLI package

Assemble the code-signed CLI used by the runner from the same target:

```bash
export TARGET="aarch64-apple-darwin"
export CLI_DIR="$PWD/target/$TARGET/release/own-domains-cli"
cargo build --release --target "$TARGET" -p own-domains-cli
mkdir -p "$CLI_DIR/sidecars"
cp "target/$TARGET/release/own-domains" "$CLI_DIR/own-domains"
cp "sidecars/build/$TARGET/coredns-$TARGET" "sidecars/build/$TARGET/caddy-$TARGET" "$CLI_DIR/"
cp "sidecars/build/$TARGET/manifest.json" "$CLI_DIR/sidecars/manifest-$TARGET.json"
codesign --force --sign - "$CLI_DIR/own-domains"
codesign --verify --strict "$CLI_DIR/own-domains"
codesign --verify --strict "$CLI_DIR/coredns-$TARGET"
codesign --verify --strict "$CLI_DIR/caddy-$TARGET"
```

The upstream CoreDNS and Caddy binaries are already ad-hoc signed. Do not re-sign them: changing their signature bytes also changes the SHA-256 digest pinned in the manifest, and setup will reject the sidecars. Sign only the CLI executable and verify the sidecar signatures.

The CLI resolves sidecars relative to its own directory, and the runner points `OWN_DOMAINS_DAEMON` at the code-signed daemon in the app bundle. Do not copy an unsigned development daemon into the acceptance path.

## 2. Automated activation sequence

Use a code-signed CLI from the matching package. Run preflight first; it does not change the host:

```bash
scripts/verify-macos-runtime.sh \
  --app "/Applications/Own Domains.app" \
  --cli "/path/to/own-domains" \
  --state-dir "$HOME/Library/Application Support/OwnDomains-acceptance" \
  --lan-address 192.168.1.20 \
  --interface en0
```

The preflight records OS/architecture, checks the app and CLI signatures (ad-hoc or Developer ID), verifies CoreDNS/Caddy hashes and executable modes, checks the macOS 15.0 minimum, confirms no conflicting resolver/PF block or fixed-port listeners, and runs the setup dry-run when the LAN inputs are present. It exits nonzero on a conflict.

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

## 3. Network-independent routing checks

After setup, record these checks without repeating privileged setup:

1. If upgrading from schema 1, run setup once and confirm it succeeds without the old address being active. Inspect `install-manifest.json`; it must report `schemaVersion: 2`, list covered `interfaces`, and contain no `lanAddress` field or singular `interface` field. Record `sudo shasum -a 256 /etc/pf.anchors/dev.owndomains`.
2. Change the Mac's LAN address on a covered interface (new DHCP reservation or lease), save it in **Network settings**, and confirm DNS, HTTP, HTTPS, `setup.own`, and phone access without an administrator prompt. The anchor hash must remain unchanged.
3. Switch to another interface that was present during setup and confirm the same routes without an administrator prompt.
4. Connect a hardware interface that was absent during setup and select its address in **Network settings**. Confirm the app reports `interfaceCoverageMissing`, run setup once, and confirm the diagnostic clears.
5. Run `sudo pfctl -a dev.owndomains -s nat` and confirm the rules show symbolic dynamic targets such as `(en0:0)` / `(en8:0)`, not a literal selected address.
6. Stop routing and confirm phone `.own` access fails; start routing again and confirm it returns without setup. Uninstall must restore the prior resolver and PF files.

## 4. Manual GUI checks

1. Launch the app against the same state directory and confirm the daemon starts once.
2. Compare mapping and status displays with `own-domains --state-dir ... --json status`.
3. Add, edit, disable, enable, and remove a mapping in the GUI; repeat through the CLI and confirm the open GUI updates without manual refresh.
4. Try a stale revision through the CLI; confirm the GUI keeps the newer settings.
5. Force a candidate failure with an injected test backend or a deliberately invalid test configuration in a disposable state; confirm the previous route remains, the runtime reports degraded, and **Stop routing** remains available in the GUI.
6. Close the window; confirm the tray stays and the daemon answers. Restore from the tray and verify a second launch focuses the existing window.
7. Run CLI stop while hidden; confirm the GUI does not restart the daemon automatically. Use Start routing explicitly.
8. Quit from the tray and confirm the daemon socket and owned listeners disappear before the app exits, or an actionable retry error appears.

## 5. Physical-phone checks

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

## 6. Development-host verification record

Recorded 2026-10-21 on macOS 15.2 (24C101), Apple M3 Pro (arm64), Rust 1.99.0, Node 26.10.0, and pnpm 12.8.1.

The signing scope changed after the unsigned smoke test: acceptance is now arm64-only, macOS 15+, and ad-hoc signed. The earlier unsigned result remains historical evidence.

Passed:

- Full automated arm64 activation acceptance on 2026-10-21T15:25:02Z: setup/start, loopback and LAN DNS, setup page, HTTP/HTTPS mappings, stale-revision rejection, restart, stop, uninstall, resolver/PF restoration, and CA trust removal. Evidence: `artifacts/macos-acceptance/20261021T152502Z.jsonl` and `.commands.log`.
- `cargo test --workspace --locked`: 127 tests across 39 targets.
- `cargo test -p own-domains-desktop --features desktop --test lifecycle`: 6 tests.
- `cargo fmt --all -- --check`, workspace and desktop-feature `cargo clippy ... -D warnings`, `pnpm test` (129 tests, 31 files), `pnpm run lint`, `pnpm run typecheck`, `pnpm run build`, and `git diff --check`.
- `pnpm run sidecars:fetch -- --arch all`, `pnpm run release:preflight` (arm64 and x86_64 digests), and an unsigned `tauri build --bundles app`.
- Development setup preview with staged sidecars and a disposable state directory rejected the existing unowned `/etc/resolver/own` without writing state.
- Unsigned GUI smoke test: the daemon socket appeared; closing the window left the process and daemon running; a second launch exited and focused the existing window; a left click on the tray status item restored the window; tray-menu Quit, app-menu Quit, and `Cmd+Q` removed the socket before exit; AppleScript Quit also left no daemon after the `RunEvent::Exit` fallback.
- Live IPC status/add/list/start/stop: `start` reported `setupMissing`, and `stop` removed the socket.
- Ad-hoc packaging and non-destructive preflight on 2026-10-21: `sidecars:fetch -- --arch all`, `release:preflight` (both architectures), `desktop:build:arm64`, the Tauri app bundle, ad-hoc `codesign` of the app and CLI, and preflight with LAN `192.168.1.219`/`en0` passed. The pinned CoreDNS/Caddy sidecars were left as shipped because re-signing changes their SHA-256.

### GUI lifecycle verification record

Recorded 2026-10-22 against a disposable state directory shared by the GUI and CLI (revision 7, `gui.own` and `cli.own`, runtime `error` with `setupMissing`):

- GUI and CLI agreed on revision, mappings, `runtimeStatus`, and diagnostics.
- **Stop routing** removed the daemon socket, kept the app running, changed the GUI to `Runtime: disconnected`, and disabled Stop. CLI status reported `stopped`.
- **Start routing** after Stop now always reconnects, including when `daemon_start` reports the runtime failure. The daemon restarted, the GUI reconnected and showed `Runtime: error`/`setupMissing`, and Stop became available again. Actual route restoration could not be tested because host setup is missing.
- **Delete** uses an in-app two-step confirmation instead of `window.confirm`, which is unsupported in the Tauri WebView. Confirming removal of `gui.own` updated the CLI to revision 8 with only `cli.own` remaining.
- Closing the window left the desktop process running; tray **Show Own Domains** restored the window. A second launch exited and focused the existing window.
- CLI `stop` while the window was hidden stopped the daemon but left the app running. Tray **Show Own Domains** displayed `Runtime: disconnected`; **Start routing** restarted the daemon.
- Tray **Quit** exited the app and removed the daemon process and socket before exit; CLI status reported `stopped`.
- Injected degraded daemon: the GUI showed `Runtime: degraded`, revision 3, Stop enabled, and the candidate diagnostic. Clicking **Stop routing** sent Stop, removed the daemon socket, kept the app running, changed the UI to `Runtime: disconnected`, and disabled Stop. The injected daemon did not own a real route, so previous-route retention was not exercised.

### Live GUI routing verification record

Recorded 2026-10-22 against `~/Library/Application Support/OwnDomains-gui-live` with LAN `192.168.1.219`/`en0`, `live.own` mapped to `127.0.0.1:9090`, and the privileged setup applied:

- The GUI auto-start showed `Runtime: running`, applied revision 1, with no error banner.
- The initial `curl -H 'Host: live.own' http://127.0.0.1:8080/` returned HTTP 200.
- **Stop routing** stopped the daemon, closed ports 5354, 8080, 8443, and 4781, and made the same request fail. The app stayed open, showed `Runtime: disconnected`, disabled Stop, and displayed no error banner.
- **Start routing** restarted the daemon and listeners, restored the request to HTTP 200, and returned the GUI to `Runtime: running` with Stop enabled and no error banner.
- The first live attempt exposed a desktop-only false `EAGAIN` from a 5-second Start timeout. The desktop now uses the same 30-second Start timeout as the CLI, and intentional frontend disconnect clears transport errors; the repeat passed.

### Forced candidate-failure record

With `live.own` running at applied revision 3, the disposable state's `runtime/candidate` directory was replaced by a regular file to force candidate staging to fail. Adding `fail3.own` saved desired revision 4, but the runtime reported `degraded` with desired 4 and applied 3. The existing `live.own` route continued to return HTTP 200, and the GUI kept **Stop routing** enabled while showing the pending revision.

Clicking **Stop routing** in that degraded state stopped the daemon and listeners, made the route fail, kept the app open, changed the UI to `Runtime: disconnected`, disabled Stop, and displayed no error banner. The candidate path was restored after the check. This exercises previous-route retention with a real route; it is a staging failure rather than a Caddy-rejected configuration, which remains covered by automated runtime tests.

### Uninstall restoration record

The matching privileged `uninstall --apply` run removed `/etc/resolver/own`, `/etc/pf.anchors/dev.owndomains`, the managed `/etc/pf.conf` block, and the state directory's `install-manifest.json`; it preserved the disposable state directory and its settings. No daemon, socket, or owned listener remained on ports 5354, 8080, 8443, or 4781. The state CA is not trusted (`security verify-cert -c ... -p ssl` returned `CSSMERR_TP_NOT_TRUSTED`); pre-existing Caddy Local Authority roots in the System keychain were not installed by this run and were left untouched.

Not exercised interactively:

- Direct Caddy rejection; the automated runtime test `caddy_rejection_keeps_previous_applied_revision_and_running_children` covers the rollback.

### Network-independent routing record

Recorded 2026-10-25 on the same macOS 15.2 arm64 host against `OwnDomains-native`:

- The schema 1 migration completed with one privileged `setup --apply`. A later privileged apply added the loopback `no rdr` exemptions after live testing exposed a macOS PF state-corruption bug. The manifest reports `schemaVersion: 2`, `interfaces: ["bridge0","en0","en1","en10","en2","en3","en4","en5","en6","en8"]`, and no `lanAddress` or `interface` field.
- `pfctl -a dev.owndomains -s nat` shows the four `no rdr` loopback exemptions and symbolic `(if:0)` targets for every covered interface, with no literal LAN address.
- The anchor SHA-256 was `96f9643ef74afd20568a2f6edbbe7ae626799f50129c9bb27b15a8aa6c9f0316` before and after switching `en0`/`192.168.1.4` to `en10`/`169.254.164.65` and back. Both `start` calls reached `running` without an administrator prompt.
- For both addresses, direct and redirected DNS over UDP and TCP on loopback and the selected address returned NOERROR; `/etc/resolver/own` resolved `setup.own` to the selected address; `http://setup.own` and direct loopback/LAN HTTP returned 200.
- Five consecutive direct TCP connections to 5354, 8080, and 8443 and redirected connections to 53, 80, and 443 succeeded. This is the regression check for the same-address loopback state corruption fixed by the `no rdr` rules.
- The 2026-10-26 physical-phone record below covers the section 5 DNS, HTTP, trusted HTTPS, setup-route, fingerprint, and stop/start checks against the schema 2 anchor.

### Physical-phone record (schema 2)

Recorded 2026-10-26 on the same macOS 15.2 arm64 host against `OwnDomains-native`, schema 2 anchor SHA-256 `96f9643ef74afd20568a2f6edbbe7ae626799f50129c9bb27b15a8aa6c9f0316`:

- The phone was on `Kien mesh` at `192.168.1.82` with manual DNS `192.168.1.4`. The Mac could reach the phone at that address.
- `http://setup.own` loaded. Installed the configuration profile, enabled the root certificate in **Certificate Trust Settings**, and confirmed `http://setup.own/fingerprint` returned `SHA256:50EADDEFB68DD08853568BDE6C12DAAD1882CEC9D4CFDEDD04B11C4955F2C169`, matching the Mac CA.
- `http://telegram.own/` loaded through the HTTP mapping.
- `https://phone.own/` loaded with a trusted padlock and the test upstream page; Safari showed no certificate warning. This used a temporary HTTPS mapping to the `127.0.0.1:4177` test upstream.
- `http://registry.own` returned `Unknown .own domain` (HTTP 404); `http://setup.own/settings` returned 404. This is the expected retired-management/no-management result.
- CLI stop removed the daemon, socket, and all owned listeners. Both `http://setup.own/?stopped=1` and `http://telegram.own/?stopped=1` failed to connect from the phone.
- CLI start returned `ApplyResult { desired_revision: 19, applied_revision: 19, status: Applied }` with no administrator prompt. The anchor SHA-256 remained `96f9643ef74afd20568a2f6edbbe7ae626799f50129c9bb27b15a8aa6c9f0316`; `http://telegram.own/?started=1` loaded again, and `http://setup.own` loaded after the restart.
- After the checks, the temporary `phone.own` mapping and the `127.0.0.1:4177` test upstream were removed. The `telegram.own` mapping was kept; it returns 502 until a service listens on `4177`. Removing the temporary mapping advanced settings to revision 22; the anchor hash remained unchanged.
- Direct high-port access from the phone (`http://192.168.1.4:8080/`) is not a gate item and was not used as a pass criterion. A both-direction capture showed the first SYN-ACK misdirected to the phone's port 80 by macOS PF state; the supported phone routes are the redirected 53/80/443 ports, which passed.
- Live uninstall and profile removal were not repeated against schema 2. The 2026-10-21 uninstall record and the automated setup/uninstall tests remain the evidence for that path.

### Physical-phone record (schema 1)

Recorded 2026-10-21 with the preserved phone state and the arm64 package above:

- The phone was on the Mac's Wi-Fi with manual DNS set to `192.168.1.219`. The first attempt failed because PF showed HTTP traffic from the phone but zero DNS packets to `192.168.1.219`; after the phone DNS setting was corrected, `http://setup.own` loaded.
- Installed the configuration profile, enabled the root certificate in **Certificate Trust Settings**, and matched the displayed SHA-256 fingerprint to the Mac CA.
- `http://phone-http.own` and `https://phone.own` loaded with trusted TLS.
- `http://registry.own` returned `Unknown .own domain` (HTTP 404); `http://setup.own/settings` also returned 404. This is the expected retired-management/no-management result, not a failure.
- Restarting routing through the CLI restored both mappings without reinstalling the profile or trust.
- After stop, both mappings and `setup.own` failed; no owned process or listener remained.
- Uninstall removed `/etc/resolver/own`, the PF block/anchor, and the CA trust entry; ports `5354/8080/8443/4781` are closed and no owned process remains. `security verify-cert` reports `CSSMERR_TP_NOT_TRUSTED` for the preserved phone CA.
- Removed the configuration profile from the phone and restored automatic Wi-Fi DNS after uninstall.

Not verified or blocked:

- The legacy runtime was removed from this host on 2026-10-21 with its managed uninstaller after explicit approval: `/etc/resolver/own`, the launchd jobs, dnsmasq/Caddy, and the listener on `127.0.0.1:4781` are gone. Its settings and CA state under `~/Library/Application Support/OwnDomains` were preserved. The preflight passed; automated activation and the physical-phone pass are recorded above.
- GUI parity was unverified at the time of this physical-phone record; the 2026-10-22 GUI records above now cover the lifecycle, Delete, live routing, and forced candidate-failure checks.
- Developer ID signing, notarization, `spctl`, and `release:check` were not run.
- Intel macOS is build-only and is not part of this gate.

## Pass/fail gate

The runtime is accepted only when all of the following are recorded:

- Sidecar and package hashes match for the arm64 package.
- `codesign --verify` passes for the app, CLI, and sidecars. Ad-hoc signatures are acceptable; `spctl` is not required for local acceptance.
- Automated preflight and activation logs show every step passed on Apple silicon macOS 15+.
- GUI and physical-phone sections pass on the Apple silicon host.
- Stop leaves no owned process or listener; restart and uninstall restore the previous host state.
- The setup-page allowlist exposes only `/`, `/ca.crt`, `/ca.mobileconfig`, and `/fingerprint`; `registry.own` has no management route (HTTP 404).

Any missing item means Phase 4 remains in progress. Keep the TypeScript source, installer, and `registry.own` compatibility paths in place until the gate is recorded.

## Recovery

If a step fails, keep the JSONL log and command log. The state directory contains the install manifest, backups, runtime configuration, and Caddy CA. Do not delete it until privileges and routes are understood.

Preview uninstall before retrying:

```bash
sudo own-domains --state-dir "$OWN_DOMAINS_STATE_DIR" uninstall
sudo own-domains --state-dir "$OWN_DOMAINS_STATE_DIR" uninstall --apply
```

If setup failed before the install manifest existed, inspect `/etc/resolver/own`, `/etc/pf.conf`, `/etc/pf.anchors/dev.owndomains`, and the trust store; setup rollback should have restored byte-identical files, but the log is authoritative. Never hand-edit PF or trust entries without recording the original contents.
