# Cross-Platform Local .own Domain Manager Implementation Plan

**Goal:** Manage local websites through a dashboard, assigning `.own` domains and subdomains that work on macOS, Linux, and Windows hosts plus phones on the same local network, with optional trusted HTTPS.

**Architecture:** A platform-neutral application core stores domain mappings and configures dedicated DNS and Caddy services. A typed host-platform adapter owns filesystem paths, privilege checks, service management, split-DNS configuration, port inspection, and reversible setup for each OS. The administration interface stays on loopback; registered websites are reachable over the LAN.

**Tech stack:** React with TypeScript and Vite for the frontend, Tailwind CSS for styling, and ESLint configured through `eslint-config-pk`. The backend is a native Rust/Tauri runtime (`crates/own-domains-{core,protocol,daemon,client,cli,desktop}`); Caddy provides the reverse proxy and CoreDNS provides consistent authoritative `.own` answers and upstream forwarding on every supported host. Use pnpm for JavaScript tooling, Vitest for unit/component tests, React Testing Library for component tests, and Playwright for browser tests. Persist a versioned JSON settings file with serialized writes for this single-user MVP; no database.

**Status:** First stable release `0.1.0` shipped on `main` (`633bf8e`). Native Rust/Tauri runtime is complete for macOS (Apple silicon acceptance target; Intel build-only) and Linux (Ubuntu 24+ live backend, `.deb`); Windows backend is a stub with host-integration catalog pending. The legacy Node.js/TS server, CLI, and browser shell were retired (`5870fb3`). Tasks below are retained as historical record; unchecked items reflect remaining/future work, not a pending greenfield plan.

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

## Definition of Done

The React dashboard uses Tailwind CSS and focused feature/component/hook modules, with ESLint based on `eslint-config-pk`. It reliably manages independent `.own` domain/subdomain mappings on Apple silicon macOS 15+, Ubuntu 24+, and Windows 10/11; Intel macOS is buildable but not acceptance-required; a real phone reaches HTTP and trusted HTTPS websites through each accepted host family; WebSockets work; settings and certificate identity survive restart; failed updates do not silently replace working routes; administration remains local; setup and recovery are documented and reversible. Platform detection blocks unsupported hosts before side effects. All configured tests, lint, type checks, builds, platform CI jobs, and authorized physical-device checks pass with recorded evidence.

## Tasks

### Vietnamese language support
- What to do: Add `vi` locale; translate all dashboard and device-setup UI strings, error messages, and setup instructions; detect locale from OS/browser with fallback to English.
- Files modified: `src/web/locales/en.json`, `src/web/locales/vi.json` (new), `src/web/lib/i18n.ts` (new), `src/web/app/App.tsx`, `src/web/features/**`, `src/web/components/ui/**`, `crates/own-domains-desktop/src/tray.rs` (if tray labels translated), tests under `tests/web/`.
- Acceptance: Switching locale renders Vietnamese throughout dashboard/setup; fallback to English when unsupported; no hardcoded English remains in components; `pnpm test`, `pnpm run typecheck` pass.

### Tray menu start/stop
- What to do: Add Start/Stop entries to the desktop tray menu controlling daemon (CoreDNS/Caddy) services; reflect running state in labels; when routing is started, label will be Stop routing, otherwise Start routing.
- Files modified: `crates/own-domains-desktop/src/tray.rs`, `crates/own-domains-desktop/src/commands.rs`, `crates/own-domains-client/src/lib.rs`, `crates/own-domains-protocol/src/*.rs` (if new IPC messages), tests under `crates/own-domains-desktop/tests/`.
- Acceptance: Tray shows Start/Stop with correct enabled state; clicking starts/stops services and updates labels; status persists across menu reopen; unit tests pass.

### Credits on About page
- What to do: Add an About page showing app name, version, author (me), and home page (GitHub repo for now); link from dashboard footer/settings.
- Files modified: `src/web/features/about/AboutPage.tsx` (new), `src/web/features/about/api.ts` (new), `src/web/app/App.tsx`, `src/web/app/routes.tsx` (if any), `src/shared/types.ts` (version type if needed), `package.json` (homepage/author metadata), tests under `tests/web/features/about/`.
- Acceptance: About page reachable from footer/settings; displays app name, version, author, home page link, credits, third-party licenses; no management data exposed; tests pass.

### Auto-start at tray
- What to do: Launch the desktop app minimized to the system tray on OS login; add a toggle in settings to enable/disable; respect OS startup settings on macOS/Linux/Windows.
- Files modified: `crates/own-domains-desktop/src/autostart.rs` (new), `crates/own-domains-desktop/src/lib.rs`, `crates/own-domains-desktop/tauri.conf.json`, `src/web/features/settings/*`, `src/shared/types.ts`, tests under `crates/own-domains-desktop/tests/` and `tests/web/`.
- Acceptance: Toggle persists and controls OS login item; app starts minimized to tray when enabled; disabling removes login item; verified on macOS; Linux/Windows documented as pending.
