#!/bin/bash
# Non-destructive macOS activation acceptance runner.
# Preflight is the default. Applying host changes requires --accept.
set -euo pipefail

APP="/Applications/Own Domains.app"
CLI=""
HOSTNAME="acceptance.own"
LAN_ADDRESS=""
INTERFACE=""
UPSTREAM_PORT=""
STATE_DIR="$HOME/Library/Application Support/OwnDomains-acceptance-$(date +%s)"
LOG_DIR="$PWD/artifacts/macos-acceptance"
APPLY=0
PURGE=0
FAILED_STEP=""

usage() {
  cat <<'EOF'
Usage: scripts/verify-macos-runtime.sh [options]

Options:
  --app PATH             Signed app bundle (default: /Applications/Own Domains.app)
  --cli PATH             Signed own-domains CLI (default: own-domains on PATH)
  --state-dir PATH       Absolute native state directory (default: disposable under ~/Library/Application Support)
  --hostname NAME        Test hostname under .own (default: acceptance.own)
  --lan-address IPv4     Mac LAN address (required with --accept)
  --interface NAME       LAN interface, for example en0 (required with --accept)
  --upstream-port PORT   Test HTTP server port on 127.0.0.1 (required with --accept)
  --log-dir PATH         JSONL log directory (default: ./artifacts/macos-acceptance)
  --accept               Apply setup/runtime changes and run reversible checks
  --purge                Purge the disposable state during final uninstall
  --help                 Show this help
EOF
}

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e ':a' -e 'N' -e '$!ba' -e 's/\n/\\n/g'
}

record() {
  local event="$1"
  local status="$2"
  shift 2
  printf '{"timestamp":"%s","event":"%s","status":"%s","message":"%s"}\n' \
    "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$event" "$status" "$(json_escape "$*")" \
    | tee -a "$LOG_FILE"
}

die() {
  record "${FAILED_STEP:-error}" "failed" "$*"
  printf 'Recovery: state and logs were retained at %s and %s\n' "$STATE_DIR" "$LOG_DIR" >&2
  exit 1
}

run_step() {
  local event="$1"
  shift
  FAILED_STEP="$event"
  if "$@" >>"$COMMAND_LOG" 2>&1; then
    record "$event" "passed" "$*"
  else
    die "command failed: $*"
  fi
}

run_capture() {
  local event="$1"
  shift
  FAILED_STEP="$event"
  if CAPTURED_OUTPUT=$("$@" 2>&1); then
    printf '%s\n' "$CAPTURED_OUTPUT" >>"$COMMAND_LOG"
    record "$event" "passed" "$*"
  else
    printf '%s\n' "${CAPTURED_OUTPUT:-}" >>"$COMMAND_LOG"
    die "command failed: $*"
  fi
}

expect_failure() {
  local event="$1"
  shift
  FAILED_STEP="$event"
  if "$@" >>"$COMMAND_LOG" 2>&1; then
    die "command unexpectedly succeeded: $*"
  else
    record "$event" "passed" "rejected as expected: $*"
  fi
}

require_absolute_state() {
  case "$STATE_DIR" in
    /*) ;;
    *) die "state directory must be absolute: $STATE_DIR" ;;
  esac
  [ "$STATE_DIR" != "/" ] || die "state directory must not be /"
}

manifest_digest() {
  local component="$1"
  local file="$2"
  sed -n "/\"$component\"[[:space:]]*:/,/^[[:space:]]*}/s/.*\"sha256\"[[:space:]]*:[[:space:]]*\"\([0-9a-f]*\)\".*/\1/p" "$file" | head -n 1
}

check_hash() {
  local label="$1"
  local path="$2"
  local expected="$3"
  [ -f "$path" ] || die "$label is missing: $path"
  [ -n "$expected" ] || die "no expected digest for $label"
  local actual
  actual=$(shasum -a 256 "$path" | awk '{print $1}')
  [ "$actual" = "$expected" ] || die "$label digest mismatch: expected $expected, received $actual"
  local mode
  mode=$(stat -f '%Lp' "$path")
  (( (0$mode & 022) == 0 )) || die "$label is writable by group or other: $path"
  (( (0$mode & 0111) != 0 )) || die "$label is not executable: $path"
  record "hash" "passed" "$label $actual"
}

check_listener_absent() {
  local port="$1"
  local output
  output=$(lsof -nP -iTCP:"$port" -sTCP:LISTEN 2>/dev/null || true)
  [ -z "$output" ] || die "port $port already has a TCP listener"
  output=$(lsof -nP -iUDP:"$port" 2>/dev/null || true)
  [ -z "$output" ] || die "port $port already has a UDP listener"
  record "listener" "passed" "port $port is free"
}

preflight() {
  [ "$(uname -s)" = "Darwin" ] || die "macOS is required"
  local version
  version=$(sw_vers -productVersion)
  local major="${version%%.*}"
  [ "$major" -ge 13 ] || die "macOS 13 or newer is required; found $version"
  record "platform" "passed" "$(uname -m) macOS $version"

  [ -d "$APP" ] || die "signed app is missing: $APP"
  local minimum
  minimum=$(plutil -extract LSMinimumSystemVersion raw "$APP/Contents/Info.plist" 2>/dev/null || true)
  [ "$minimum" = "13.0" ] || die "app minimumSystemVersion is not 13.0: ${minimum:-missing}"
  record "minimum-version" "passed" "$minimum"

  FAILED_STEP="codesign"
  codesign --verify --deep --strict "$APP" >>"$COMMAND_LOG" 2>&1 || die "app code signature verification failed"
  record "codesign" "passed" "app signature verified"
  FAILED_STEP="codesign-cli"
  codesign --verify --strict "$CLI" >>"$COMMAND_LOG" 2>&1 || die "CLI code signature verification failed"
  record "codesign-cli" "passed" "CLI signature verified"

  case "$(uname -m)" in
    arm64) TARGET="aarch64-apple-darwin" ;;
    x86_64) TARGET="x86_64-apple-darwin" ;;
    *) die "unsupported architecture: $(uname -m)" ;;
  esac
  local manifest="$APP/Contents/Resources/sidecars/manifest-$TARGET.json"
  [ -f "$manifest" ] || die "runtime sidecar manifest is missing: $manifest"
  check_hash "CoreDNS" "$APP/Contents/Resources/coredns-$TARGET" "$(manifest_digest coredns "$manifest")"
  check_hash "Caddy" "$APP/Contents/Resources/caddy-$TARGET" "$(manifest_digest caddy "$manifest")"
  [ -x "$APP/Contents/MacOS/own-domains-daemon" ] || die "daemon is missing from the app"
  [ -x "$APP/Contents/MacOS/own-domains-desktop" ] || die "desktop executable is missing from the app"
  local cli_directory
  cli_directory=$(cd "$(dirname "$CLI")" && pwd)
  for binary in "$cli_directory/coredns-$TARGET" "$cli_directory/caddy-$TARGET"; do
    [ -x "$binary" ] || die "standalone CLI sidecar is missing: $binary"
    FAILED_STEP="codesign-cli-sidecar"
    codesign --verify --strict "$binary" >>"$COMMAND_LOG" 2>&1 || die "CLI sidecar signature verification failed: $binary"
  done
  record "sidecars" "passed" "target $TARGET; app and CLI sidecar signatures verified"

  local port
  for port in 5354 8080 8443 4781; do
    check_listener_absent "$port"
  done
  [ ! -e /etc/resolver/own ] || die "existing /etc/resolver/own must be removed by the legacy uninstall first"
  if grep -q '# BEGIN OWN DOMAINS' /etc/pf.conf 2>/dev/null; then
    die "existing owned PF block must be removed by the legacy uninstall first"
  fi
  record "host" "passed" "no conflicting resolver, PF block, or fixed-port listener"

  if [ -n "$LAN_ADDRESS" ] && [ -n "$INTERFACE" ]; then
    FAILED_STEP="setup-preview"
    run_capture "setup-preview" "$CLI" --state-dir "$STATE_DIR" setup \
      --lan-address "$LAN_ADDRESS" --interface "$INTERFACE"
    [ -n "$CAPTURED_OUTPUT" ] || die "setup preview produced no output"
  else
    record "setup-preview" "skipped" "--lan-address and --interface are required for preview"
  fi
}

wait_for_port() {
  local port="$1"
  local attempts=0
  while [ "$attempts" -lt 50 ]; do
    if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
      record "listener" "passed" "TCP port $port is listening"
      return 0
    fi
    attempts=$((attempts + 1))
    sleep 0.2
  done
  die "timed out waiting for TCP port $port"
}

check_port_closed() {
  local port="$1"
  if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
    die "TCP port $port is still listening"
  fi
  record "listener" "passed" "TCP port $port is closed"
}

check_no_owned_sidecars() {
  local binary
  for binary in "$APP/Contents/Resources/coredns-$TARGET" "$APP/Contents/Resources/caddy-$TARGET"; do
    if pgrep -f "$binary" >/dev/null 2>&1; then
      die "owned sidecar is still running: $binary"
    fi
  done
  record "process" "passed" "no owned CoreDNS or Caddy process remains"
}

read_revision() {
  local status_json
  run_capture "status" "$CLI" --state-dir "$STATE_DIR" --json status
  status_json="$CAPTURED_OUTPUT"
  printf '%s' "$status_json" | sed -n 's/.*"revision": *\([0-9][0-9]*\).*/\1/p' | head -n 1
}

check_dns() {
  local name="$1"
  local expected="$2"
  local result
  result=$(dig @127.0.0.1 -p 5354 "$name" A +short +time=2 +tries=1)
  [ "$result" = "$expected" ] || die "loopback UDP DNS for $name returned '$result', expected '$expected'"
  result=$(dig +tcp @127.0.0.1 -p 5354 "$name" A +short +time=2 +tries=1)
  [ "$result" = "$expected" ] || die "loopback TCP DNS for $name returned '$result', expected '$expected'"
  result=$(dig @"$LAN_ADDRESS" -p 53 "$name" A +short +time=2 +tries=1)
  [ "$result" = "$expected" ] || die "LAN UDP DNS for $name returned '$result', expected '$expected'"
  result=$(dig +tcp @"$LAN_ADDRESS" -p 53 "$name" A +short +time=2 +tries=1)
  [ "$result" = "$expected" ] || die "LAN TCP DNS for $name returned '$result', expected '$expected'"
  record "dns" "passed" "$name resolves to $expected over UDP and TCP on loopback and LAN"
}

check_setup_page() {
  local code
  code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 5 -H 'Host: setup.own' "http://$LAN_ADDRESS/")
  [ "$code" = "200" ] || die "setup.own returned HTTP $code"
  code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 5 -H 'Host: setup.own' "http://$LAN_ADDRESS/settings")
  [ "$code" = "404" ] || die "setup.own private route returned HTTP $code instead of 404"
  local fingerprint
  fingerprint=$(curl -fsS --max-time 5 -H 'Host: setup.own' "http://$LAN_ADDRESS/fingerprint")
  case "$fingerprint" in SHA256:*) ;; *) die "setup.own fingerprint is invalid" ;; esac
  record "setup-page" "passed" "public routes allowed; private route denied; fingerprint present"
}

check_http_route() {
  local body
  body=$(curl -fsS --max-time 5 -H "Host: $HOSTNAME" "http://$LAN_ADDRESS/")
  [ -n "$body" ] || die "HTTP route returned an empty body"
  local code
  code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 5 -H 'Host: registry.own' "http://$LAN_ADDRESS/")
  [ "$code" = "404" ] || die "registry.own returned HTTP $code instead of 404"
  record "http-route" "passed" "$HOSTNAME served over HTTP; registry.own returns 404"
}

check_https_route() {
  local body
  body=$(curl -fsS --max-time 5 --resolve "$HOSTNAME:443:$LAN_ADDRESS" \
    --cacert "$STATE_DIR/caddy-data/pki/authorities/local/root.crt" "https://$HOSTNAME/")
  [ -n "$body" ] || die "HTTPS route returned an empty body"
  record "https-route" "passed" "$HOSTNAME served over trusted HTTPS"
}

apply() {
  [ "$(id -u)" -eq 0 ] && die "run the acceptance script as the logged-in user, not root; setup uses sudo"
  [ -n "$LAN_ADDRESS" ] || die "--lan-address is required with --accept"
  [ -n "$INTERFACE" ] || die "--interface is required with --accept"
  [ -n "$UPSTREAM_PORT" ] || die "--upstream-port is required with --accept"
  case "$UPSTREAM_PORT" in
    ''|*[!0-9]*) die "--upstream-port must be numeric: $UPSTREAM_PORT" ;;
  esac
  [ "$UPSTREAM_PORT" -ge 1 ] && [ "$UPSTREAM_PORT" -le 65535 ] || die "--upstream-port must be between 1 and 65535"
  record "acceptance" "started" "explicit --accept requested; state $STATE_DIR"
  export OWN_DOMAINS_DAEMON="$APP/Contents/MacOS/own-domains-daemon"

  preflight
  run_step "setup-apply" sudo "$CLI" --state-dir "$STATE_DIR" setup --apply \
    --lan-address "$LAN_ADDRESS" --interface "$INTERFACE"
  run_step "start" "$CLI" --state-dir "$STATE_DIR" start
  wait_for_port 5354
  wait_for_port 8080
  wait_for_port 8443
  wait_for_port 4781
  check_dns "$HOSTNAME" "$LAN_ADDRESS"
  check_setup_page

  local revision
  revision=$(read_revision)
  [ -n "$revision" ] || die "could not read the settings revision"
  run_step "add" "$CLI" --state-dir "$STATE_DIR" add "$HOSTNAME" \
    --port "$UPSTREAM_PORT" --revision "$revision"
  check_http_route
  revision=$(read_revision)
  if [ "$revision" -gt 0 ]; then
    expect_failure "stale-revision" "$CLI" --state-dir "$STATE_DIR" add "stale.own" \
      --port "$UPSTREAM_PORT" --revision "$((revision - 1))"
  fi
  run_step "enable-https" "$CLI" --state-dir "$STATE_DIR" edit "$HOSTNAME" \
    --https true --revision "$revision"
  check_https_route
  record "gui-parity" "manual" "compare the open GUI list/status with the CLI output"

  revision=$(read_revision)
  run_step "remove" "$CLI" --state-dir "$STATE_DIR" remove "$HOSTNAME" --revision "$revision"
  run_step "stop" "$CLI" --state-dir "$STATE_DIR" stop
  for port in 5354 8080 8443 4781; do check_port_closed "$port"; done
  check_no_owned_sidecars
  run_step "restart" "$CLI" --state-dir "$STATE_DIR" start
  wait_for_port 5354
  run_step "stop-again" "$CLI" --state-dir "$STATE_DIR" stop
  for port in 5354 8080 8443 4781; do check_port_closed "$port"; done
  check_no_owned_sidecars

  run_step "uninstall" sudo "$CLI" --state-dir "$STATE_DIR" uninstall --apply
  [ ! -e /etc/resolver/own ] || die "uninstall left /etc/resolver/own in place"
  if grep -q '# BEGIN OWN DOMAINS' /etc/pf.conf 2>/dev/null; then
    die "uninstall left the owned PF block in place"
  fi
  record "restoration" "passed" "resolver and PF ownership removed"
  if [ "$PURGE" -eq 1 ]; then
    run_step "purge" sudo "$CLI" --state-dir "$STATE_DIR" uninstall --apply --purge
    [ ! -e "$STATE_DIR" ] || die "purge left the acceptance state directory in place"
    record "purge" "passed" "acceptance state removed"
  fi
  record "acceptance" "passed" "automated checks completed; GUI and physical-phone checks remain manual"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --app) APP="$2"; shift 2 ;;
    --cli) CLI="$2"; shift 2 ;;
    --state-dir) STATE_DIR="$2"; shift 2 ;;
    --hostname) HOSTNAME="$2"; shift 2 ;;
    --lan-address) LAN_ADDRESS="$2"; shift 2 ;;
    --interface) INTERFACE="$2"; shift 2 ;;
    --upstream-port) UPSTREAM_PORT="$2"; shift 2 ;;
    --log-dir) LOG_DIR="$2"; shift 2 ;;
    --accept) APPLY=1; shift ;;
    --purge) PURGE=1; shift ;;
    --help|-h) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

mkdir -p "$LOG_DIR"
LOG_FILE="$LOG_DIR/$(date -u '+%Y%m%dT%H%M%SZ').jsonl"
COMMAND_LOG="$LOG_DIR/$(date -u '+%Y%m%dT%H%M%SZ').commands.log"
: >"$LOG_FILE"
: >"$COMMAND_LOG"
record "log" "started" "JSONL log: $LOG_FILE"
[ -n "$CLI" ] || CLI=$(command -v own-domains || true)
[ -n "$CLI" ] || die "own-domains CLI not found; pass --cli"
require_absolute_state
case "$HOSTNAME" in
  *[!a-z0-9.-]*|.*|*.) die "invalid test hostname: $HOSTNAME" ;;
  *.own) ;;
  *) die "test hostname must end in .own: $HOSTNAME" ;;
esac

if [ "$APPLY" -eq 1 ]; then
  apply
else
  preflight
  record "preflight" "passed" "no changes applied; rerun with --accept to apply"
fi
