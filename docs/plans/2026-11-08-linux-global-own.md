# Linux Global `~own` Check Plan (Milestone E2)

## Goal

Detect a global `Domains=~own` resolved setting (as opposed to the per-link split DNS the installer owns): setup preflight refuses with the exact fix, and the inspector reports `resolvedDomainConflict` naming the global scope. Also align the existing per-link conflict code with the design (`resolvedDomainConflict`, not `ownDomainConflict`).

## Constraints

- Parse `resolvectl status` output: the global block runs from the `Global` line to the first `^Link \d+` line (or end of output); it conflicts when that block contains `~own`. No other global content matters. `resolvectl status` itself never mutates.
- Reuse the `resolvedDomainConflict` code from the design doc for both scopes; the message names the scope (`global resolved configuration` vs `link <name>`). Rename the existing `ownDomainConflict` to `resolvedDomainConflict` in inspector + tests (no other code references it; GUI/doctor matching does not exist yet).
- Setup preflight runs the check on fixed `resolvectl` with no shell, before any mutation, with the remedy: remove the global `Domains=~own` (resolved.conf/drop-in), `systemctl restart systemd-resolved`, rerun setup. The check is read-only; setup never edits resolved configuration.
- VM negative testing is invasive (editing resolved.conf + restarting resolved on the test VM); unit fakes plus the healthy-host e2e silence check suffice, and the plan records the skipped negative explicitly.
- No CLI output changes; no other diagnostic codes change (the remaining design-code drift — `unitDisabled` vs `systemdUnitDisabled`, `caUntrusted` vs `caNotTrusted`, etc. — stays a follow-up).

## Out of Scope

Docker detection (still deferred), `.deb` packaging (separate plan), legacy iptables, browser trust stores.

## Verification Commands

All from `/Users/kien/workspace/domains` with `export PATH=$HOME/.cargo/bin:$PATH`:

- test: `cargo test -p own-domains-daemon --test linux_setup_unit` and `--test linux_inspector_unit`
- full suite: `cargo test -p own-domains-daemon -p own-domains-cli`
- lint: `cargo clippy -p own-domains-daemon -p own-domains-cli --all-targets -- -D warnings`
- fmt: `cargo fmt --all -- --check`
- targets: `cargo check -p own-domains-daemon -p own-domains-cli --all-targets` for `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`
- VM: inspector e2e on a healthy host (silence) per task 4

### Task 1: Align the conflict code with the design

Files:
- modify `crates/own-domains-daemon/src/platform/linux/inspector.rs` (`ownDomainConflict` → `resolvedDomainConflict`, message keeps the link name)
- modify `crates/own-domains-daemon/tests/linux_inspector_unit.rs` (same rename in the two assertions)

Steps:
1. Write test change first: rename the asserted code to `resolvedDomainConflict` in the per-link conflict test; run `cargo test -p own-domains-daemon --test linux_inspector_unit`; confirm it fails (impl still emits the old code).
2. Implement: rename the emission site.
3. Run the test; confirm it passes.
4. Run `cargo test -p own-domains-daemon --test linux_setup_unit`; confirm green.

Done when: the design code is used for link conflicts and the suite is green.

### Task 2: Global `~own` inspector diagnostic

Files:
- modify `crates/own-domains-daemon/src/platform/linux/inspector.rs`
- modify `crates/own-domains-daemon/tests/linux_inspector_unit.rs`

Steps:
1. Write test: `resolvectl status` stub whose global block contains `DNS Domain: ~own` yields `resolvedDomainConflict` with `global` in the message; a global block without `~own` (plus a clean link) yields nothing new; `resolvectl status` failing yields no global diagnostic (covered by existing conflict scan, no new code).
2. Run; confirm it fails (no global parsing).
3. Implement `inspect_global_domain`: run `resolvectl status` (reuse the existing `resolvectl` helper), slice the global block, check for `~own`.
4. Run the test; confirm it passes.

Done when: global claims are fake-covered and clean output stays silent.

### Task 3: Setup preflight rejection

Files:
- modify `crates/own-domains-daemon/src/platform/linux/setup.rs`
- modify `crates/own-domains-daemon/tests/linux_setup_unit.rs`

Steps:
1. Write test: `resolvectl status` stub with `~own` in the global block makes `apply_setup` fail naming the global config and the resolved.conf remedy, creating no receipts and no host files; clean status proceeds.
2. Run; confirm it fails.
3. Implement `check_resolved_global` in the preflight chain (after `check_binaries`, before discovery is fine — order among read-only checks is free) reusing the same block-slice rule; extract the shared slice helper if both call sites need it (small private `fn` in `linux/mod.rs` or duplicated 6-line logic — prefer a `pub(crate) fn global_resolved_block` in `inspector.rs` reused by `setup.rs`).
4. Run the test; confirm it passes.

Done when: the preflight refuses with the remedy and leaves zero traces.

### Task 4: Ubuntu VM verification (non-invasive)

Files: none (VM execution; fix fallout as found).

Steps:
1. On the Ubuntu VM, pull the branch, rebuild, run `sudo setup --apply` if needed, then the healthy-host inspector e2e (`sudo -E … cargo test -p own-domains-daemon --test linux_inspector_e2e`) — expect zero diagnostics (proves the global parse does not false-positive on a real `resolvectl status`).
2. Do NOT set a global `Domains=~own` on the VM (invasive); record the negative case as fake-covered only.
3. Paste all output.

Done when: the e2e is green on real output and the skipped negative is recorded.

### Task 5: Full verification and review

Files: none (verification only).

Steps:
1. Host: full suite, clippy `-D warnings`, three target checks, `cargo fmt --check`, `pnpm test`; report results.
2. `git diff --stat` against `main`; present for review. Do not merge, do not push; merging is the user's decision.

Done when: all checks reported with real output and the diff is up for review.
"
