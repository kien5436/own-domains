# Cross-Platform Local .own Domain Manager Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task by task. Steps use checkboxes for progress. Implementation is a separate task; this document does not authorize package installation, system changes, commits, or publishing.

**Goal:** Manage local websites through a dashboard, assigning `.own` domains and subdomains that work on macOS, Linux, and Windows hosts plus phones on the same local network, with optional trusted HTTPS.

**Architecture:** A platform-neutral application core stores domain mappings and configures dedicated DNS and Caddy services. A typed host-platform adapter owns filesystem paths, privilege checks, service management, split-DNS configuration, port inspection, and reversible setup for each OS. The administration interface stays on loopback; registered websites are reachable over the LAN.

**Tech stack:** React with TypeScript and Vite for the frontend, Tailwind CSS for styling, and ESLint configured through `eslint-config-pk`. The backend uses Node.js; Caddy provides the reverse proxy and CoreDNS provides consistent authoritative `.own` answers and upstream forwarding on every supported host. Use pnpm for JavaScript tooling, Vitest for unit/integration tests, React Testing Library for component tests, and Playwright for browser tests. Use a versioned JSON settings file with serialized writes for this single-user MVP; a database is unnecessary initially. Confirm any additional production dependencies or managed binaries before installation and pin compatible supported versions.

**Spec:** The agreed requirements and design decisions are recorded below; no separate specification exists. Tasks 1–5 and the initial macOS setup safety foundation have been implemented. Remaining work must preserve those interfaces while extracting platform-specific behavior.

## Requirements and Scope

- Register names such as `shop.own`, `api.shop.own`, and `admin.api.shop.own`, each with an independent local destination and HTTPS setting.
- Create, edit, disable, enable, and delete mappings; preserve settings across restarts.
- Serve registered websites on the host computer and on phones connected to the same reachable LAN.
- Keep existing development servers running independently; this application routes traffic but does not launch projects.
- Provide setup guidance, service diagnostics, and understandable failure messages.
- Support Apple silicon macOS 15+ as the native acceptance target, Intel macOS 15+ as build-only, and Ubuntu 24+ and Windows 10/11 as implementation targets. Current native runtime support and signing policy are in [Platform support](docs/platform-support.md). Detect unsupported operating systems before planning or changing host files.
- Provide iPhone and Android browser setup instructions; verify a physical phone against each host family before declaring that platform's LAN workflow complete.
- Defer wildcard registrations, IPv6 serving, remote administration, public hosting, accounts, billing, Docker-only deployment, non-systemd Linux service installation, and older Windows versions.

## Global Constraints

- Keep `.own` as requested. It is not an IANA special-use suffix, so document the possibility of a future naming conflict; do not silently substitute `.test`. See the [IANA registry](https://www.iana.org/assignments/special-use-domain-names).
- Work on a dedicated feature branch; preserve the already staged `AGENTS.md`. Do not commit, push, delete existing files, or install production dependencies without authorization.
- Planning creates only `PLAN.md`. Implementation must separately obtain approval for privileged setup, trust-store changes, service installation, and changes outside the workspace.
- Do not run the web backend as root. Privileged setup uses a reviewed CLI workflow with fixed operations, never arbitrary browser-provided commands.
- Keep secrets, CA private keys, generated configurations, logs, and runtime settings out of Git. Export only the CA's public certificate.
- Keep Caddy's administration endpoint on a permission-restricted Unix socket on macOS/Linux and loopback-only TCP on Windows. Bind the application management server to `127.0.0.1:4780` only.
- Put every host mutation behind a `HostPlatform` adapter selected from `process.platform`; unsupported platforms must fail before generating or applying artifacts.
- Expose DNS and proxy listeners only on loopback and the selected LAN address. Never automatically configure router port forwarding or DHCP.
- Use React, Tailwind CSS, and `eslint-config-pk` as requested. Keep frontend code in focused modules; `App.tsx` composes the application shell rather than containing all screens, state, API calls, and styling.

## Network and Product Design

### Request path

```text
Phone or host browser
  -> local DNS: shop.own resolves to the host's selected LAN IPv4 address
  -> Caddy on that host, port 80 or 443
  -> configured local app, for example http://127.0.0.1:3000

Host browser -> http://127.0.0.1:4780 -> dashboard and management API
```

Reserve `setup.own` for a read-only device setup page containing DNS instructions and the public CA certificate. It must not expose management endpoints or proxy arbitrary destinations. Provide the same page at the host's LAN IP over HTTP for initial setup before DNS or certificate trust works; unknown named hosts receive a generic 404.

### DNS and phone setup

Use CoreDNS to answer `.own` A queries with the selected LAN IPv4 address and keep the entire `.own` namespace local, including unsupported query types. Return no IPv6 address initially. Forward other domains to explicitly configured upstream DNS servers, with checks preventing forwarding back to this service. Validate generated configuration before activation. The existing dnsmasq generator remains supported only during the macOS migration and is removed after equivalent CoreDNS integration tests pass.

Configure split DNS through the host adapter: `/etc/resolver/own` on macOS, systemd-resolved/NetworkManager integration on supported Linux, and an NRPT rule on Windows. For phones, guide the user to set the Wi-Fi DNS server to the host's LAN IP. All configured phone DNS servers must understand `.own`; a public secondary server can cause intermittent failures. Normal Internet lookups continue through forwarding while the host is available. Explain how to restore automatic DNS when the host is asleep or unavailable.

Recommend a router DHCP reservation for the host. Detect IP/interface changes and show a repair action; never silently choose a VPN interface. Explain guest-network client isolation, host firewalls, VPNs, and encrypted DNS overrides when connectivity fails. Being on the same Wi-Fi name alone does not prove connectivity.

#### Deferred: encrypted DNS coexistence

Review after the local HTTP/TLS workflow is stable. Phone-level encrypted DNS clients such as NextDNS, WARP, Private DNS, or browser DoH can bypass the Wi-Fi resolver and make `.own` unreachable; listing both the host and a public resolver is not a reliable fallback because clients may query either one.

Prefer one provider-neutral integration over provider-specific adapters: the phone uses Own Domains as its only LAN DNS server, `.own` stays authoritative locally, and public queries may be forwarded through an optional RFC 8484 DNS-over-HTTPS endpoint on loopback. Keep plain IPv4 upstream DNS as an explicit fallback mode. Before implementation, review credential storage, HTTPS-only URL validation, bootstrap resolution, forwarding-loop and SSRF prevention, timeout/failure behavior, privacy implications of fallback, provider analytics/profile preservation, and behavior while the host is asleep. Do not add a production dependency or managed binary without approval.

DNS can resolve unregistered names, but only explicitly registered, enabled routes are served. Registration controls routing, not ownership in public DNS.

### Proxy and TLS behavior

- HTTP mode serves port 80 without redirecting to HTTPS. HTTPS mode uses Caddy's internal CA and redirects HTTP to HTTPS for that exact hostname.
- Use exact hostname matching, including independent settings for parent domains and subdomains. Disable HSTS by default so local TLS toggles remain practical.
- Terminate TLS at Caddy; forward to an HTTP loopback app. Preserve paths, queries, request bodies, WebSockets, and streaming responses.
- Show development-server allowed-host and hot-reload configuration guidance when the upstream rejects the custom hostname.
- Keep CA identity stable across restarts. Require separate trust installation on each device; successful certificate issuance does not prove that a phone trusts it. See [Caddy local HTTPS](https://caddyserver.com/docs/automatic-https) and [Apple certificate trust instructions](https://support.apple.com/en-ie/102390).
- Apply a complete configuration with one Caddy API load request. Caddy's request-level atomicity does not make the application's file writes atomic with the proxy; implement explicit reconciliation. See the [Caddy API](https://caddyserver.com/docs/api).

### Persistence and interfaces

Resolve runtime data through the host adapter: `~/Library/Application Support/OwnDomains/` on macOS, `$XDG_STATE_HOME/own-domains` or `~/.local/state/own-domains` on Linux, and `%LOCALAPPDATA%\OwnDomains` on Windows. Keep settings schema-versioned and user-readable; protect application secrets and CA storage with restrictive permissions or Windows ACLs.

```ts
type DomainMapping = {
  id: string;
  hostname: string;
  upstreamPort: number;
  tls: boolean;
  enabled: boolean;
};
type Settings = {
  schemaVersion: 1;
  revision: number;
  lanAddress: string;
  upstreamDns: string[];
  mappings: DomainMapping[];
};
type ApplyResult = {
  desiredRevision: number;
  appliedRevision: number;
  status: 'applied' | 'pending' | 'error';
  message?: string;
};
```

Limit upstreams to `http://127.0.0.1:<port>` for the MVP; accept a port in the UI rather than an arbitrary URL. Reserve ports 53, 80, 443, and 4780 to prevent proxy loops or access to management services. Projects must expose an HTTP listener on IPv4 loopback, including host-published Docker ports if applicable.

Normalize names to lowercase and remove one trailing dot. Accept ASCII DNS labels of 1–63 characters, letters/digits/internal hyphens only, with a maximum normalized hostname length of 253 characters and at least one label before `.own`. Reject empty labels, wildcards, Unicode, URLs, IP literals, duplicate normalized names, and `setup.own` plus its descendants. Explicit nested subdomains require no parent registration.

API: `GET/POST /api/domains`, `PATCH/DELETE /api/domains/:id`, `GET /api/status`, `GET/PUT /api/settings/network`, and `POST /api/reconcile`. Mutations carry `expectedRevision` and return the updated settings revision plus `ApplyResult`; stale revisions return 409. Validation errors return 400, missing IDs 404, and authorization failures 403. A saved change awaiting application returns 202 and stays visibly pending.

Serialize mutations and write desired state through a temporary file, flush, and atomic rename. Then apply it to Caddy, recording the applied revision separately. Retain the previous working configuration after rejection; show failed desired changes without claiming activation. On restart, reconcile durable desired state with the running proxy. Use a process lock to prevent two backend writers. Domain changes need only a proxy reload; network-setting changes stay pending until the setup CLI applies and verifies the privileged DNS/listener changes. Track those service results separately and report a revision as applied only when all required services match it.

## Proposed File Organization

| Path | Responsibility |
| --- | --- |
| `src/shared/types.ts` | Mapping, settings, diagnostics, and API contracts |
| `src/server/domains/{validation,store,service}.ts` | Validation, durable settings, serialized mutations |
| `src/server/proxy/{config,client,reconcile}.ts` | Generate Caddy configuration and apply/recover revisions |
| `src/server/network/{dns-config,diagnostics}.ts` | DNS configuration and connectivity checks |
| `src/platform/{types,detect,paths}.ts` | Platform contract, OS detection, and portable application paths |
| `src/platform/{darwin,linux,win32}/` | OS-specific resolver, service, privilege, firewall, and listener adapters |
| `src/server/http/{server,security,routes}.ts` | Loopback API, session protection, static dashboard |
| `src/web/main.tsx` | React bootstrap and global stylesheet import |
| `src/web/app/App.tsx` | Application shell and feature composition only |
| `src/web/lib/api-client.ts` | Shared HTTP transport, CSRF handling, and typed API errors |
| `src/web/features/domains/{DomainsPage,DomainForm,DomainTable,DomainRow}.tsx` | Domain page and focused presentation components |
| `src/web/features/domains/{api,use-domains,form-validation}.ts` | Domain requests, state/mutations, and pure form validation |
| `src/web/features/status/{StatusPanel.tsx,use-status.ts,api.ts}` | Service diagnostics display, state, and requests |
| `src/web/features/setup/{DeviceSetupPage,DeviceInstructions}.tsx` | Read-only phone setup page and reusable instructions |
| `src/web/features/setup/main.tsx` | Separate public setup entry without management hooks or requests |
| `src/web/components/ui/` | Small shared controls with typed props and accessible behavior |
| `src/web/styles/app.css` | Tailwind entry, shared theme tokens, and minimal global styles |
| `eslint.config.js` | `eslint-config-pk` integration and scoped project overrides |
| `src/cli/{setup,doctor,uninstall}.ts` | Platform-neutral command orchestration over the selected host adapter |
| `assets/` | Static UI resources, if needed |
| `tests/` | Mirrored unit tests, integration fixtures, and browser tests |
| `docs/{setup,troubleshooting}.md` | Per-platform setup, phone DNS/trust, and recovery instructions |

### Frontend maintainability rules

- Organize by feature; keep each component, hook, and helper focused on one responsibility. Split by responsibility rather than imposing an arbitrary file-length limit.
- Keep network access in feature `api.ts` modules using `lib/api-client.ts`; hooks own asynchronous state and mutations, while components render state and handle user interactions through typed props/callbacks.
- Keep form state near its form, share server state through feature hooks, and avoid introducing global state libraries until a concrete need exists. Use pure helpers for normalization and validation; server validation remains authoritative.
- Use Tailwind utilities for styling and shared theme tokens for recurring colors, spacing, and typography. Extract repeated UI patterns into small reusable components; avoid a monolithic stylesheet or premature design-system abstractions.
- Use semantic HTML, labeled inputs, visible focus states, and explicit loading, empty, success, and error states. Keep the public device setup entry independent of management-only logic.
- Extend `eslint-config-pk` using its documented API and supported React/TypeScript configuration. Verify compatibility and React Hooks rule coverage during scaffolding; use narrowly scoped overrides rather than blanket disables or an unrelated replacement preset.
- Mirror feature paths in frontend tests. Test meaningful form behavior, failed requests, mutation state, and accessibility interactions; avoid tests tied only to internal component structure.

## Review Focus

1. Mixed-case, trailing-dot, and nested hostnames must not create duplicates or route to another project: Task 1.
2. Concurrent updates or crashes between disk writes and proxy reloads must recover predictably: Task 2.
3. A phone using IPv6 queries, encrypted DNS, or an isolated Wi-Fi network must receive actionable diagnosis: Tasks 3 and 10.
4. TLS toggles and restarts must preserve HTTP-only behavior and stable CA identity: Tasks 2 and 10.
5. A hostile website or LAN client must not mutate local settings or reach Caddy administration: Tasks 4 and 10.

## Implementation Tasks

### Task 1: Establish tooling and the domain registry

**Files:** `package.json`, pnpm lockfile, `vite.config.ts`, TypeScript/test configuration, `eslint.config.js`, `src/web/styles/app.css`; `src/shared/types.ts`; `src/server/domains/{validation,store}.ts`; mirrored tests.

**Interfaces:** `normalizeHostname(input: string): string`, `validateMapping(input: unknown): DomainMapping`, and `loadSettings(): Promise<Settings>`. Invalid input throws a typed validation error; a corrupt settings file produces a recovery error, never a silent empty registry.

- [ ] Select compatible supported versions of React, TypeScript, Vite, Tailwind CSS, ESLint, and `eslint-config-pk`; inspect the preset's current exports and peer requirements before configuring it. Obtain approval for additional proposed production dependencies; scaffold with pnpm and add scripts for dev, build, test, lint, typecheck, and browser tests.
- [ ] Configure React/TSX, Tailwind's supported Vite integration and CSS entry, and `eslint.config.js` based on `eslint-config-pk`. Include frontend, backend, and test files with appropriate environment settings; verify React Hooks checks and ignore generated outputs. Confirm a small React component renders styled content and passes lint/type checking.
- [ ] Write failing tests: normalize `Shop.OWN.` to `shop.own`; accept `admin.api.shop.own`; reject duplicate normalized names, reserved names/ports, and label/hostname limits described above.
- [ ] Run `pnpm test -- tests/server/domains`; confirm the tests fail for missing behavior, then implement validation, versioned storage, and corruption handling.
- [ ] Repeat the focused tests and run `pnpm run lint` and `pnpm run typecheck`; require success.

### Task 2: Route requests with durable configuration reconciliation

**Files:** `src/server/domains/service.ts`; `src/server/proxy/{config,client,reconcile}.ts`; `tests/integration/proxy.test.ts`; reconciliation unit tests.

**Interfaces:** `buildProxyConfig(settings: Settings): object`, `applyProxyConfig(config: object): Promise<void>`, and `reconcile(settings: Settings): Promise<ApplyResult>`. All mutations flow through one revision-checked queue.

- [ ] Write failing tests for exact parent/subdomain routes, disabled mappings, unknown-host 404s, HTTP-only mode, HTTPS redirects, and rejected Caddy configurations.
- [ ] Add fault-injection tests for stale revisions, two concurrent mutations, a failed atomic write, a crash before/after proxy application, and a second backend instance.
- [ ] Run focused tests to confirm failures, then implement config generation, the restricted Unix-socket client, durable revisions, and startup reconciliation.
- [ ] Verify real Caddy against fixture HTTP/WebSocket/streaming apps on unprivileged test ports. Require preserved requests and a working previous route after a rejected reload.

### Task 3: Configure DNS and diagnose LAN reachability

**Files:** `src/server/network/{dns-config,diagnostics}.ts`; network unit tests; `tests/integration/dns.test.ts`.

**Interfaces:** `buildDnsConfig(settings: Settings): string` and `diagnose(settings: Settings): Promise<Diagnostic[]>`, with each diagnostic containing `code`, `status`, `message`, and `remedy`.

- [ ] Write failing tests for `.own` A answers, no unintended AAAA answer, no `.own` upstream leakage, normal public-domain forwarding, and rejected upstream forwarding loops.
- [ ] Add tests for missing/changed LAN addresses, occupied ports, missing binaries, unavailable upstream apps, and distinct DNS/proxy failures.
- [ ] Implement configuration generation and bounded-time diagnostic probes; retain `dnsmasq --test` coverage for the initial macOS provider until Task 7 replaces it with validated CoreDNS fixtures.
- [ ] Run network tests and real UDP/TCP DNS integration checks on an unprivileged test port; confirm expected answers and no forwarding loops.

### Task 4: Provide a protected management API

**Files:** `src/server/http/{server,security,routes}.ts`; `tests/integration/api.test.ts`.

**Interfaces:** Implement the API contract above; serve the dashboard and API from the same loopback origin. Define structured `Diagnostic` and API response types in `src/shared/types.ts`.

- [ ] Write failing tests for CRUD, revision conflicts, validation/error codes, pending apply results, and network-setting validation.
- [ ] Test rejection of hostile Host/Origin headers, cross-origin mutations, missing CSRF tokens, oversized bodies, and attempts to proxy the management port.
- [ ] Implement exact Host checks, no cross-origin API access, a local session cookie with HttpOnly/SameSite=Strict, and per-session CSRF tokens for mutations. Treat local processes under the user's account as trusted; do not add remote login.
- [ ] Run API tests and prove the backend listens only on loopback and does not expose the Caddy socket.

### Task 5: Build the dashboard and device setup page

**Files:** `src/web/main.tsx`, `src/web/app/App.tsx`, `src/web/lib/api-client.ts`, the domain/status/setup feature modules and shared UI components listed above, `src/web/styles/app.css`; mirrored frontend unit/component tests and browser tests.

**Interfaces:** Use the Task 4 API exclusively. Display desired and applied states separately and use `expectedRevision` for every mutation.

- [ ] Write browser tests for adding `shop.own:3000`, adding `api.shop.own:8080`, toggling TLS, editing ports, disabling, and confirmed deletion.
- [ ] Add checks for inline validation, stale-update recovery, pending/error states, keyboard operation, and narrow-screen layouts.
- [ ] Implement the typed API client, feature API modules, and hooks independently from presentation. Add component tests with React Testing Library for form submission/validation, pending/error feedback, and recovery after a failed request.
- [ ] Compose the React dashboard from the focused components in the file map; keep `App.tsx` limited to layout and composition. Build domain management, copy/open URL actions, diagnostics, and phone setup instructions with Tailwind styling and shared UI controls. Show a clear port-to-domain mapping and certificate-trust status limits.
- [ ] Implement the isolated read-only `setup.own`/LAN-IP page; expose only setup information and the public certificate with its fingerprint. Check that no settings, logs, private keys, or management API are reachable through it.
- [ ] Review component responsibilities, hook/API boundaries, repeated UI patterns, and Tailwind theme usage; extract unrelated responsibilities from oversized modules without adding unnecessary abstractions.
- [ ] Run frontend unit/component tests, `pnpm run lint`, `pnpm run typecheck`, and `pnpm run test:e2e`; require the documented flows to pass with fixture services.

### Task 6: Extract the platform-neutral host contract

**Files:** `src/platform/{types,detect,paths}.ts`; refactor `src/cli/{setup,doctor,uninstall}.ts`, `src/cli/{setup-plan,setup-preflight,setup-writer,uninstall-plan}.ts`; mirrored platform and CLI tests.

**Interfaces:** `detectHostPlatform(): HostPlatformId`, `getHostPlatform(id): HostPlatform`, and `HostPlatform` methods `paths()`, `planSetup()`, `inspect()`, `activate()`, `planUninstall()`, and `deactivate()`. Setup, doctor, and uninstall commands consume only this interface. Installation still requires explicit `--apply` and normal OS privilege approval.

- [ ] Write failing tests proving `darwin`, `linux`, and `win32` select distinct adapters while every other `process.platform` value exits before filesystem, command, or network effects.
- [ ] Add path tests for macOS Application Support, Linux XDG fallback, and Windows LocalAppData; reject missing or relative platform roots.
- [ ] Move shared dry-run, hash-manifest, backup, rollback, conflict, ownership, and port-validation behavior behind platform-neutral artifact/action types. Preserve current manifest compatibility and test idempotent upgrades.
- [ ] Add an OS guard to every executable before it builds a plan. Keep platform commands as fixed argument arrays; never interpolate browser or settings input into a shell.
- [ ] Run platform/CLI tests, lint, type checking, and existing integration tests; commit before implementing any OS adapter.

### Task 7: Complete the macOS adapter and migrate DNS to CoreDNS

**Files:** `src/platform/darwin/{index,paths,resolver,services,diagnostics}.ts`; CoreDNS/Caddy/launchd templates; macOS CLI and integration tests; `docs/setup.md`.

**Interfaces:** Implement `HostPlatform` for `darwin`; generate `/etc/resolver/own`, root LaunchDaemons for CoreDNS/Caddy, and a per-user LaunchAgent for the backend. Preserve legacy dnsmasq detection and provide a reviewed migration preview before removal.

- [ ] Write failing tests for Intel/Apple Silicon binary locations, root-versus-user ownership, launchd labels, existing dnsmasq migration, WARP listeners on non-conflicting loopback addresses, and unavailable ports 53/80/443.
- [ ] Generate and validate a CoreDNS configuration equivalent to the existing `.own` behavior before changing services. Back up resolver/service files and record hashes in the installation manifest.
- [ ] Implement `launchctl bootstrap/bootout` activation and rollback with root infrastructure services and a logged-in-user backend. Never run the backend as root.
- [ ] Extend doctor/uninstall for launchd state, resolver drift, firewall guidance, and manifest-owned restoration while preserving CA data.
- [ ] Run macOS dry runs without host mutation; perform real activation only after separate privileged authorization.

### Task 8: Add the systemd Linux adapter

**Files:** `src/platform/linux/{index,paths,resolver,services,diagnostics}.ts`; systemd/CoreDNS/Caddy templates; Linux CLI and integration tests; Linux sections in setup/troubleshooting docs.

**Interfaces:** Implement `HostPlatform` for `linux`; support systemd system services for CoreDNS/Caddy and a systemd user service for the backend. Support systemd-resolved first and NetworkManager when detected; reject unsupported resolver managers with actionable dry-run output.

- [ ] Write failing tests for XDG paths, root/user ownership, systemd unit hardening, resolver-manager detection, firewall diagnostics, occupied ports, and unsupported non-systemd hosts.
- [ ] Implement split-DNS setup for `.own`, fixed service commands, daemon reload/start/stop, manifest-safe rollback, and least-privilege service users/capabilities for ports below 1024.
- [ ] Add Ubuntu CI fixtures for CoreDNS, Caddy, setup dry runs, uninstall previews, and service-template validation without modifying the CI host resolver.
- [ ] Document `ufw`/firewalld guidance, NetworkManager/systemd-resolved recovery, CA installation, and automatic DNS restoration.
- [ ] Verify on an Ubuntu 24+ VM before claiming Linux host support.

### Task 9: Add the Windows adapter

**Files:** `src/platform/win32/{index,paths,resolver,services,diagnostics}.ts`; PowerShell/Windows Service templates; Windows CLI and integration tests; Windows sections in setup/troubleshooting docs.

**Interfaces:** Implement `HostPlatform` for `win32`; store data under `%LOCALAPPDATA%\OwnDomains`, configure `.own` through a manifest-owned NRPT rule, run CoreDNS/Caddy as restricted Windows services, and start the backend in the interactive user's context. Use argument-array process execution and PowerShell `-File`, never generated command strings.

- [ ] Write failing tests for Windows path/ACL handling, administrator detection, NRPT conflicts, service-name collisions, occupied IPv4 listeners, CRLF/escaping, and rollback after partial service installation.
- [ ] Implement reviewed PowerShell scripts with fixed parameters, Authenticode-friendly files, Windows Service creation/removal, firewall-rule previews, and manifest-scoped recovery.
- [ ] Bind Caddy administration to a loopback-only TCP port and confirm it is unreachable from LAN interfaces. Preserve the same-user local-process trust assumption explicitly.
- [ ] Add Windows CI fixtures for dry-run planning, CoreDNS/Caddy configuration validation, and uninstall previews without changing CI DNS or services.
- [ ] Verify on Windows 10 or 11 before claiming Windows host support.

### Task 10: Verify the cross-platform host-to-phone workflow

**Files:** `tests/integration/lan.test.ts`; platform integration fixtures; `tests/e2e/domains.spec.ts`; final setup/troubleshooting documentation and command updates in `AGENTS.md`.

- [ ] Run `pnpm test`, `pnpm run lint`, `pnpm run typecheck`, `pnpm run build`, and `pnpm run test:e2e` on macOS, Ubuntu, and Windows CI; record real results and platform-specific skips.
- [ ] On each supported host family, after authorized setup, open `http://shop.own` and `https://api.shop.own` locally and from a physical phone on the same LAN; install/trust the public CA and verify HTTPS without bypassing warnings.
- [ ] Verify nested subdomains, hot reload/WebSockets, HTTP-only behavior beside HTTPS routes, and public Internet DNS on the phone.
- [ ] Review the deferred encrypted-DNS coexistence design using NextDNS plus at least one other RFC 8484 provider; decide whether generic DoH forwarding belongs in scope before implementing it.
- [ ] Test application/service restart, sleep/wake or suspend/resume, changed LAN IP, stopped upstream apps, port conflicts, disabled mappings, and rejected proxy configuration; confirm accurate recovery guidance and persistent CA identity.
- [ ] From the phone, confirm that the management API and Caddy administration are inaccessible. Exercise cross-origin mutation attempts from a separate test origin.
- [ ] Record host OS/version, phone OS/browser, resolver manager, and unresolved limitations. Do not claim platform or physical-device verification from mocks, CI, or browser-only tests.

## Definition of Done

The React dashboard uses Tailwind CSS and focused feature/component/hook modules, with ESLint based on `eslint-config-pk`. It reliably manages independent `.own` domain/subdomain mappings on Apple silicon macOS 15+, Ubuntu 24+, and Windows 10/11; Intel macOS is buildable but not acceptance-required; a real phone reaches HTTP and trusted HTTPS websites through each accepted host family; WebSockets work; settings and certificate identity survive restart; failed updates do not silently replace working routes; administration remains local; setup and recovery are documented and reversible. Platform detection blocks unsupported hosts before side effects. All configured tests, lint, type checks, builds, platform CI jobs, and authorized physical-device checks pass with recorded evidence.
