# Local .own Domain Manager Implementation Plan

> **For agentic workers:** Use `superpowers:executing-plans` to implement this plan task by task. Steps use checkboxes for progress. Implementation is a separate task; this document does not authorize package installation, system changes, commits, or publishing.

**Goal:** Manage local websites through a dashboard, assigning `.own` domains and subdomains that work on the host Mac and phones on the same local network, with optional trusted HTTPS.

**Architecture:** A local application stores domain mappings and configures a dedicated Caddy reverse proxy. A local DNS service resolves `.own` names to the Mac's LAN address. The administration interface stays on loopback; registered websites are reachable over the LAN.

**Tech stack:** React with TypeScript and Vite for the frontend, Tailwind CSS for styling, and ESLint configured through `eslint-config-pk`. Proposed backend/infrastructure: Node.js, Caddy, and dnsmasq. Use pnpm for JavaScript tooling, Vitest for unit/integration tests, React Testing Library for component tests, and Playwright for browser tests. Use a versioned JSON settings file with serialized writes for this single-user MVP; a database is unnecessary initially. Confirm any additional production dependencies before installation and pin compatible supported versions during scaffolding.

**Spec:** The agreed requirements and design decisions are recorded below; no separate specification exists. The repository currently contains `AGENTS.md` and this plan, has no commits or remotes, and has no existing implementation or test commands.

## Requirements and Scope

- Register names such as `shop.own`, `api.shop.own`, and `admin.api.shop.own`, each with an independent local destination and HTTPS setting.
- Create, edit, disable, enable, and delete mappings; preserve settings across restarts.
- Serve registered websites on the Mac and on phones connected to the same reachable LAN.
- Keep existing development servers running independently; this application routes traffic but does not launch projects.
- Provide setup guidance, service diagnostics, and understandable failure messages.
- Support macOS hosting and IPv4 LAN connectivity first. Provide iPhone and Android browser setup instructions; verify against the user's actual phone before declaring LAN support complete.
- Defer wildcard registrations, IPv6 serving, remote administration, public hosting, accounts, billing, Docker-only deployment, and Windows/Linux host support.

## Global Constraints

- Keep `.own` as requested. It is not an IANA special-use suffix, so document the possibility of a future naming conflict; do not silently substitute `.test`. See the [IANA registry](https://www.iana.org/assignments/special-use-domain-names).
- Work on a dedicated feature branch; preserve the already staged `AGENTS.md`. Do not commit, push, delete existing files, or install production dependencies without authorization.
- Planning creates only `PLAN.md`. Implementation must separately obtain approval for privileged setup, trust-store changes, service installation, and changes outside the workspace.
- Do not run the web backend as root. Privileged setup uses a reviewed CLI workflow with fixed operations, never arbitrary browser-provided commands.
- Keep secrets, CA private keys, generated configurations, logs, and runtime settings out of Git. Export only the CA's public certificate.
- Keep Caddy's administration endpoint on a permission-restricted Unix socket. Bind the application management server to `127.0.0.1:4780` only.
- Expose DNS and proxy listeners only on loopback and the selected LAN address. Never automatically configure router port forwarding or DHCP.
- Use React, Tailwind CSS, and `eslint-config-pk` as requested. Keep frontend code in focused modules; `App.tsx` composes the application shell rather than containing all screens, state, API calls, and styling.

## Network and Product Design

### Request path

```text
Phone or Mac browser
  -> local DNS: shop.own resolves to the Mac's selected LAN IPv4 address
  -> Caddy on that Mac, port 80 or 443
  -> configured local app, for example http://127.0.0.1:3000

Mac browser -> http://127.0.0.1:4780 -> dashboard and management API
```

Reserve `setup.own` for a read-only device setup page containing DNS instructions and the public CA certificate. It must not expose management endpoints or proxy arbitrary destinations. Provide the same page at the Mac's LAN IP over HTTP for initial setup before DNS or certificate trust works; unknown named hosts receive a generic 404.

### DNS and phone setup

Use dnsmasq to answer `.own` A queries with the selected LAN IPv4 address and keep the entire `.own` namespace local, including unsupported query types. Return no IPv6 address initially. Forward other domains to explicitly configured upstream DNS servers, with checks preventing forwarding back to this service. Disable DHCP/TFTP features. Validate generated configuration before activation. See the [dnsmasq manual](https://thekelleys.org.uk/dnsmasq/docs/dnsmasq-man.html).

Configure the Mac's `/etc/resolver/own` through the privileged setup workflow. For phones, guide the user to set the Wi-Fi DNS server to the Mac's LAN IP. All configured phone DNS servers must understand `.own`; a public secondary server can cause intermittent failures. Normal Internet lookups continue through forwarding while the Mac is available. Explain how to restore automatic DNS when the Mac is asleep or unavailable.

Recommend a router DHCP reservation for the Mac. Detect IP/interface changes and show a repair action; never silently choose a VPN interface. Explain guest-network client isolation, firewall blocks, VPNs, and encrypted DNS overrides when connectivity fails. Being on the same Wi-Fi name alone does not prove connectivity.

DNS can resolve unregistered names, but only explicitly registered, enabled routes are served. Registration controls routing, not ownership in public DNS.

### Proxy and TLS behavior

- HTTP mode serves port 80 without redirecting to HTTPS. HTTPS mode uses Caddy's internal CA and redirects HTTP to HTTPS for that exact hostname.
- Use exact hostname matching, including independent settings for parent domains and subdomains. Disable HSTS by default so local TLS toggles remain practical.
- Terminate TLS at Caddy; forward to an HTTP loopback app. Preserve paths, queries, request bodies, WebSockets, and streaming responses.
- Show development-server allowed-host and hot-reload configuration guidance when the upstream rejects the custom hostname.
- Keep CA identity stable across restarts. Require separate trust installation on each device; successful certificate issuance does not prove that a phone trusts it. See [Caddy local HTTPS](https://caddyserver.com/docs/automatic-https) and [Apple certificate trust instructions](https://support.apple.com/en-ie/102390).
- Apply a complete configuration with one Caddy API load request. Caddy's request-level atomicity does not make the application's file writes atomic with the proxy; implement explicit reconciliation. See the [Caddy API](https://caddyserver.com/docs/api).

### Persistence and interfaces

Store runtime data in `~/Library/Application Support/OwnDomains/`, resolved using OS APIs. Keep settings schema-versioned and user-readable; protect application secrets and CA storage with restrictive permissions.

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
| `src/cli/{setup,doctor,uninstall}.ts` | Explicit host setup, diagnosis, reversible removal |
| `assets/` | Static UI resources, if needed |
| `tests/` | Mirrored unit tests, integration fixtures, and browser tests |
| `docs/{setup,troubleshooting}.md` | Mac setup, phone DNS/trust, recovery instructions |

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
3. A phone using IPv6 queries, encrypted DNS, or an isolated Wi-Fi network must receive actionable diagnosis: Tasks 3 and 7.
4. TLS toggles and restarts must preserve HTTP-only behavior and stable CA identity: Tasks 2 and 7.
5. A hostile website or LAN client must not mutate local settings or reach Caddy administration: Tasks 4 and 7.

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
- [ ] Implement configuration generation and bounded-time diagnostic probes; validate via `dnsmasq --test` against generated fixtures before applying.
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

### Task 6: Add explicit Mac setup and recovery workflows

**Files:** `src/cli/{setup,doctor,uninstall}.ts`; launchd/config templates; `docs/{setup,troubleshooting}.md`; CLI tests.

**Interfaces:** `pnpm run setup -- --dry-run`, `pnpm run doctor`, and `pnpm run uninstall -- --dry-run`; installation requires an explicit apply option and normal privilege approval.

- [ ] Write tests for setup dry runs, idempotent repeat setup, conflicting existing services/resolver files, incorrect ownership, and unavailable ports 53/80/443.
- [ ] Implement reviewed service/resolver installation with least-privilege service identities, explicit launchd configuration, backups, and a manifest of owned changes. Start the user backend at login and infrastructure through launchd; document that the Mac must be awake.
- [ ] Implement removal previews that restore only owned settings and do not remove unrelated configurations or CA data automatically. Document separate trust removal on each device.
- [ ] Document phone DNS and certificate installation, Android browser-versus-native-app trust differences, DHCP reservation, DNS bypass settings, firewall checks, and restoring automatic DNS.
- [ ] Run CLI tests and inspect dry-run output without modifying the host until privileged setup is authorized.

### Task 7: Verify the complete Mac-to-phone workflow

**Files:** `tests/integration/lan.test.ts`; `tests/e2e/domains.spec.ts`; final setup/troubleshooting documentation and command updates in `AGENTS.md`.

- [ ] Run `pnpm test`, `pnpm run lint`, `pnpm run typecheck`, `pnpm run build`, and `pnpm run test:e2e`; record real results.
- [ ] After authorized setup, open `http://shop.own` and `https://api.shop.own` on the Mac and a physical phone on the same LAN; install/trust the public CA and verify HTTPS without bypassing warnings.
- [ ] Verify nested subdomains, hot reload/WebSockets, HTTP-only behavior beside HTTPS routes, and public Internet DNS on the phone.
- [ ] Test application restart, service restart, Mac sleep/wake, changed LAN IP, stopped upstream apps, port conflicts, disabled mappings, and rejected proxy configuration; confirm accurate recovery guidance and persistent CA identity.
- [ ] From the phone, confirm that the management API and Caddy administration are inaccessible. Exercise cross-origin mutation attempts from a separate test origin.
- [ ] Record phone OS/browser and any unresolved environment limitations. Do not claim physical-device verification from mocked/browser-only tests.

## Definition of Done

The React dashboard uses Tailwind CSS and focused feature/component/hook modules, with ESLint based on `eslint-config-pk`. It reliably manages independent `.own` domain/subdomain mappings; a real phone reaches HTTP and trusted HTTPS websites through the Mac; WebSockets work; settings and certificate identity survive restart; failed updates do not silently replace working routes; administration remains local; setup and recovery are documented and reversible. All configured tests, lint, type checks, and builds pass with recorded evidence.
