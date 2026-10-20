# Native CLI

The Rust CLI and daemon are additive during migration; the existing TypeScript commands remain available.

```sh
cargo run -p own-domains-daemon -- --state-dir .own-domains
cargo run -p own-domains-cli --bin own-domains -- --state-dir .own-domains status
cargo run -p own-domains-cli --bin own-domains -- --state-dir .own-domains add shop.own --port 3000 --revision 0 --https
```

The command surface is `start`, `stop`, `status`, `list`, `add`, `edit`, `enable`, `disable`, `remove`, and `doctor`. Use `--json` for one machine-readable protocol result on stdout. Mutation commands require `--revision` to prevent stale writes.

When no daemon is running, reads report a stopped runtime and mutations take the exclusive state lock, save through the same daemon command handler, print `saved; runtime stopped`, and exit without starting DNS or proxy services. `start` launches `own-domains-daemon`; set `OWN_DOMAINS_DAEMON` to an explicit binary path when it is not on `PATH`. Until a platform runtime backend is added, activation intentionally returns `runtimeUnavailable`.

Exit codes:

- `0`: success
- `2`: invalid input or missing mapping
- `3`: stale settings revision
- `4`: daemon, IPC, storage, or protocol failure
- `5`: runtime unavailable or runtime operation failure
