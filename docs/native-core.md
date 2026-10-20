# Portable Rust Core

`own-domains-core` is the platform-neutral behavior shared by the future daemon, CLI, and Tauri desktop client.

## Public modules

- Models preserve the existing schema-version-1 camelCase JSON format.
- Validation normalizes `.own` hostnames, rejects reserved names and ports, validates IPv4 network settings, and classifies conflicts.
- `SettingsStore` preserves unknown schema-version-1 fields and uses an owner-only, flushed, atomic replacement.
- `DomainService` serializes revision-checked settings mutations within a process.
- Caddy generation creates exact enabled routes plus the read-only `setup.own` route and never creates a `registry.own` route.
- CoreDNS generation creates deterministic authoritative `.own` and public forwarding blocks while rejecting listener exposure and forwarding loops.

## Compatibility guarantees

Existing `settings.json` files retain mapping IDs, revisions, network settings, mappings, and unknown top-level fields. Invalid or unsupported files remain untouched and return a typed recovery error. This phase does not move application-data or CA paths.

## Atomicity boundary

The store serializes threads using the same canonical settings path and atomically replaces settings on disk. Cross-process exclusion, desired/applied runtime reconciliation, and service ownership belong to the daemon phase.

## Deferred responsibilities

This crate does not provide IPC, CLI parsing, Tauri integration, subprocess execution, CoreDNS/Caddy installation or validation, platform service managers, privileged setup, runtime events, or shutdown orchestration. The existing TypeScript backend and CLI remain the active product until those later phases reach parity.
