# Linux Coexistence Checks Plan (Milestone E1)

## Goal

Detect active UFW/firewalld default-deny policy and `nftables.service` flush-ruleset conflicts — as setup preflight errors (firewall) or warnings (flush-ruleset), and as inspector diagnostics with unprivileged `*Unverifiable` fallbacks. Never modifies UFW, firewalld, or `/etc/nftables.conf`. (Docker publication detection deferred — see Out of Scope.)

## Constraints

- Fixed commands only, no shell: `docker ps --format`, `ufw status`, `firewall-cmd --state/--get-default-zone/--list-ports`, `systemctl is-enabled nftables.service`, file read of `/etc/nftables.conf`. New binaries are *probed*, never required: absence means "not installed", not an error.
- Preflight severity per design: Docker publication on a covered port/interface and unallowed high ports under an active default-deny firewall are **errors** (refuse before mutating, zero FS traces); `nftables.service` + `flush ruleset` is a **warning** (`nftablesServiceConflict`).
- Warnings need a channel: add `warnings: Vec<String>` to `SetupApplyResult` (daemon-internal; update the Windows installer constructors), rendered by CLI `render_apply` after the written/unchanged lines. Inspector warnings use `DiagnosticStatus::Warning` with codes `nftablesServiceConflict`, `firewallUnverifiable`, `dockerUnverifiable`.
- Inspector runs as the target user: unreadable `nft`/`ufw`/`docker` state yields `nftablesUnverifiable` / `firewallUnverifiable` / `dockerUnverifiable` (warning), never a safe-claim. `ss`-visible occupancy is still `portConflict`.
UFW allowance parsing reads `ufw status` (not numbered): active + `Default: deny (incoming)` requires explicit `ALLOW` lines for `5354/tcp`, `5354/udp`, `8080/tcp`, `8443/tcp`; anything missing fails with exactly the four `sudo ufw allow …` commands. firewalld: `firewall-cmd --state` == `running` requires the same four `port/proto` entries in the default zone's `--list-ports`; missing entries fail with the matching `firewall-cmd --permanent --add-port=…` + `--reload` commands.
- No macOS/Windows/CLI-command behavior changes except additive `render_apply` warning lines and the new `warnings` field (Windows installer sets it empty).

## Out of Scope

Docker publication detection (preflight + inspector) — deferred; the parser seam (`parse_docker_ports`) is left for that slice. `.deb`/AppImage packaging. Trust-store browser gaps (documented, never modified). Legacy-iptables systems (out of design acceptance). Global `Domains=~own` resolved setting (separate check, later).

## Verification Commands

All from `/Users/kien/workspace/domains` with `export PATH=$HOME/.cargo/bin:$PATH`:

- test (host, per task): `cargo test -p own-domains-daemon --test linux_setup_unit` and `--test linux_inspector_unit`
- full suite: `cargo test -p own-domains-daemon -p own-domains-cli`
- lint: `cargo clippy -p own-domains-daemon -p own-domains-cli --all-targets -- -D warnings`
- fmt: `cargo fmt --all -- --check`
- targets: `cargo check -p own-domains-daemon -p own-domains-cli --all-targets` for `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`
- VM: scenario runs per task 8

### Task 1: UFW preflight error with exact commands

Files:
- modify `crates/own-domains-daemon/src/platform/linux/setup.rs`
- modify `crates/own-domains-daemon/tests/linux_setup_unit.rs`

Steps:
1. Write test: `ufw status` stub reporting `Status: active` + `Default: deny (incoming)` without `5354/udp` ALLOW yields failure naming `ufw` plus the exact four `sudo ufw allow 5354/tcp`, `5354/udp`, `8080/tcp`, `8443/tcp` lines; `Status: inactive` passes; `ufw` missing passes.
2. Run; confirm it fails.
3. Implement `check_ufw`: parse status/default/ALLOW lines (case-sensitive match on `ALLOW` + `port/proto` tokens); inactive or absent binary passes silently.
4. Run the test; confirm it passes.

Done when: active default-deny with a gap fails with the exact remedy; inactive/absent passes.

### Task 2: firewalld preflight error with exact commands

Files:
- modify `crates/own-domains-daemon/src/platform/linux/setup.rs`
- modify `crates/own-domains-daemon/tests/linux_setup_unit.rs`

Steps:
1. Write test: `firewall-cmd --state` → `running`, `--get-default-zone` → `public`, `--zone=public --list-ports` missing `8443/tcp` yields failure naming `firewall-cmd` with `--permanent --zone=public --add-port=` lines for each missing port plus `--reload`; `not running` passes.
2. Run; confirm it fails.
3. Implement `check_firewalld` with the same required set as UFW (`5354/tcp`, `5354/udp`, `8080/tcp`, `8443/tcp`).
4. Run the test; confirm it passes.

Done when: running firewalld with a gap fails with the exact remedy; stopped/absent passes.

### Task 3: UFW/firewalld inspector checks + unverifiable fallbacks

Files:
- modify `crates/own-domains-daemon/src/platform/linux/inspector.rs`
- modify `crates/own-domains-daemon/tests/linux_inspector_unit.rs`

Steps:
1. Write test: active-UFW stub with a gap yields `firewallBlocked` (error) naming the missing port; `ufw` failing + `firewall-cmd --state` failing yields `firewallUnverifiable` (warning); clean stubs yield nothing.
2. Run; confirm it fails.
3. Implement `inspect_firewall` reusing the Task 3/4 parsers (extract shared parsing into the same helper location as Task 2); unreadable state warns instead of erroring.
4. Run the test; confirm it passes.

Done when: blocked errors, unreadable warns, clean is silent — all fake-covered.

### Task 4: Warnings plumbing (`SetupApplyResult.warnings` + CLI render)

Files:
- modify `crates/own-domains-daemon/src/platform/host.rs` (add `warnings: Vec<String>`)
- modify `crates/own-domains-daemon/src/platform/windows/setup.rs` (two constructors set empty)
- modify `crates/own-domains-daemon/src/platform/linux/setup.rs` (both constructors set empty for now)
- modify `crates/own-domains-daemon/tests/windows_setup_unit.rs` + `linux_setup_unit.rs` only if they construct the struct literally (check first; fix what breaks)
- modify `crates/own-domains-cli/src/command.rs` (`render_apply` prints `warning <text>` lines after unchanged lines)

Steps:
1. Write test (CLI): `render_apply`-equivalent via `execute` is heavy; instead unit-test in `command.rs` `#[cfg(test)]` if a module exists, else assert through the existing uninstall preview path? Minimal honest test: extend the new `warnings` field in a linux setup unit test asserting `result.warnings` is empty on the happy path (pins the field), plus a CLI-level assertion is deferred to Task 7's VM run. Prefer: add a `#[cfg(test)] mod tests` case for `render_apply` with warnings if no such module exists — check and create only if missing.
2. Run the affected test binaries; confirm they fail (field missing / render ignores warnings).
3. Implement the field, the two Windows + one Linux constructor updates, and the render lines.
4. Run `cargo test -p own-domains-daemon -p own-domains-cli`; confirm green.

Done when: the field flows from installers through CLI rendering with tests pinning both ends.

### Task 5: nftables.service flush-ruleset warning (setup + inspector)

Files:
- modify `crates/own-domains-daemon/src/platform/linux/setup.rs`
- modify `crates/own-domains-daemon/src/platform/linux/inspector.rs`
- modify `crates/own-domains-daemon/tests/linux_setup_unit.rs`
- modify `crates/own-domains-daemon/tests/linux_inspector_unit.rs`

Steps:
1. Write tests: `/etc/nftables.conf` containing `flush ruleset` + `systemctl is-enabled nftables.service` → `enabled` makes `apply_setup` succeed with `warnings == ["nftablesServiceConflict: ..."]` (no failure, no receipt change); inspector on the same stubs yields `nftablesServiceConflict` warning; absent file or `flush`-free file yields nothing.
2. Run; confirm they fail.
3. Implement: setup reads the file (missing → skip) and queries the unit; inspector mirrors with its files/runner seams.
4. Run both test binaries; confirm green.

Done when: the warning fires in both paths and silence holds otherwise — all fake-covered.

### Task 6: Ubuntu VM verification (privileged + unprivileged)

Files: none (VM execution; fix fallout in `setup.rs`/`inspector.rs`/tests as found).

Steps:
1. On the Ubuntu VM, pull the branch, rebuild, run the real Docker case if Docker exists (else record absence): publish a test container on 8080 (`docker run -p 8080:80 …`), run `sudo -E … setup --apply` expecting the Docker error, stop/remove the container.
2. UFW case only if UFW is installed (do not enable it just for the test — if absent, record absence and rely on fakes): with an active deny policy missing a high port, expect the exact-commands error.
3. `nftables.service` case: only if `/etc/nftables.conf` exists — otherwise synthesize nothing; record which cases ran for real vs fake-covered.
4. `doctor` (offline CLI) on the healthy host must stay silent; `systemctl stop own-domains-nftables` + start cycle still reports `unitInactive` + `nftablesTableMissing` (regression check that Task 8's wiring is intact).
5. Paste all output.

Done when: every runnable-on-that-VM case is evidenced; unrunnable ones are explicitly recorded as fake-covered only.

### Task 7: Full verification and review

Files: none (verification only).

Steps:
1. Host: `cargo test -p own-domains-daemon -p own-domains-cli`; report results.
2. `cargo clippy -p own-domains-daemon -p own-domains-cli --all-targets -- -D warnings`, three target checks, `cargo fmt --all -- --check`, `pnpm test`; report results.
3. `git diff --stat` against `main`; present for review. Do not merge, do not push; merging is the user's decision.

Done when: all checks reported with real output and the diff is up for review.
"
