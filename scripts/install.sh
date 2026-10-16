#!/bin/sh

set -eu

run_or_fail() {
  failure_message=$1
  shift
  "$@" || {
    status=$?
    printf 'ERROR: %s\n' "$failure_message" >&2
    exit "$status"
  }
}

dry_run=false
lan_address=

for argument in "$@"; do
  case "$argument" in
    --dry-run) dry_run=true ;;
    --*) printf 'Unknown option: %s\n' "$argument" >&2; exit 2 ;;
    *)
      if [ -n "$lan_address" ]; then
        printf 'Usage: %s [--dry-run] <LAN_IPv4>\n' "$0" >&2
        exit 2
      fi
      lan_address=$argument
      ;;
  esac
done

if [ -z "$lan_address" ]; then
  printf 'Usage: %s [--dry-run] <LAN_IPv4>\n' "$0" >&2
  exit 2
fi

printf 'INSTALL locked dependencies\n'
printf 'BUILD dashboard and server\n'
printf 'SETUP %s\n' "$lan_address"

if [ "$dry_run" = true ]; then
  exit 0
fi

if [ "$(uname -s)" != Darwin ]; then
  printf 'Own Domains setup currently supports macOS only.\n' >&2
  exit 1
fi

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"
pnpm_bin=$(command -v pnpm) || { printf 'pnpm is required. Install it with: brew install pnpm\n' >&2; exit 1; }

run_or_fail 'dependency installation failed; no system changes were made' "$pnpm_bin" install --frozen-lockfile
run_or_fail 'build failed; no system changes were made' "$pnpm_bin" build
run_or_fail 'setup validation failed; no system changes were made' "$pnpm_bin" run setup -- --dry-run --lan-address "$lan_address"
run_or_fail 'setup failed; review the error and rollback report above' /usr/bin/sudo "$pnpm_bin" run setup -- --apply --lan-address "$lan_address"

printf 'Open http://registry.own\n'
