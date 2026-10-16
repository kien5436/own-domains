#!/bin/sh

set -eu

had_errors=0
error_messages=

record_error() {
  error_message=$1
  printf 'ERROR: %s\n' "$error_message" >&2
  had_errors=$((had_errors + 1))
  error_messages="$error_messages
- $error_message"
}

finish() {
  if [ 0 -ne "$had_errors" ]; then
    error_label=errors
    if [ 1 -eq "$had_errors" ]; then
      error_label=error
    fi
    printf 'Cleanup finished with %s %s:%s\n' "$had_errors" "$error_label" "$error_messages" >&2
    return 1
  fi

  return 0
}

try_step() {
  failure_message=$1
  shift
  if "$@"; then
    return 0
  else
    record_error "$failure_message"
    return 0
  fi
}

dry_run=false
purge=false

for argument in "$@"; do
  case "$argument" in
    --dry-run) dry_run=true ;;
    --purge) purge=true ;;
    *) printf 'Usage: %s [--dry-run] [--purge]\n' "$0" >&2; exit 2 ;;
  esac
done

application_directory=$HOME/Library/Application\ Support/OwnDomains
manifest_path=$application_directory/install-manifest.json

case "$application_directory" in
  /Users/*/Library/Application\ Support/OwnDomains) ;;
  *) printf 'Refusing unsafe application-data path: %s\n' "$application_directory" >&2; exit 2 ;;
esac

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"
pnpm_bin=$(command -v pnpm) || { printf 'ERROR: pnpm is required; no files were removed\n' >&2; exit 1; }

can_uninstall=true
if [ -f "$manifest_path" ]; then
  if ! "$pnpm_bin" run uninstall -- --dry-run; then
    record_error 'uninstall preview failed; managed setup files will be skipped'
    can_uninstall=false
  fi
else
  record_error 'install manifest is missing; managed setup files will be skipped'
  can_uninstall=false
fi

printf 'UNLOAD dev.owndomains.backend\n'
printf 'UNLOAD dev.owndomains.caddy\n'
printf 'UNLOAD dev.owndomains.dnsmasq\n'
if [ "$can_uninstall" = true ]; then
  printf 'UNINSTALL managed setup files\n'
else
  printf 'SKIP managed setup files\n'
fi
if [ "$purge" = true ]; then
  printf 'PURGE %s\n' "$application_directory"
fi

if [ "$dry_run" = true ]; then
  if ! finish; then
    exit 1
  fi
  exit 0
fi

if [ "$(uname -s)" != Darwin ]; then
  printf 'Own Domains uninstall currently supports macOS only.\n' >&2
  exit 1
fi

user_id=$(id -u)

if /bin/launchctl print "gui/$user_id/dev.owndomains.backend" >/dev/null 2>&1; then
  try_step 'could not unload the backend' /bin/launchctl bootout "gui/$user_id/dev.owndomains.backend"
fi
if /bin/launchctl print system/dev.owndomains.caddy >/dev/null 2>&1; then
  try_step 'could not unload Caddy' /usr/bin/sudo /bin/launchctl bootout system/dev.owndomains.caddy
fi
if /bin/launchctl print system/dev.owndomains.dnsmasq >/dev/null 2>&1; then
  try_step 'could not unload dnsmasq' /usr/bin/sudo /bin/launchctl bootout system/dev.owndomains.dnsmasq
fi
if [ "$can_uninstall" = true ]; then
  try_step 'managed-file uninstall failed' /usr/bin/sudo "$pnpm_bin" run uninstall -- --apply
fi

if [ "$purge" = true ]; then
  try_step 'application-data purge failed' /usr/bin/sudo /bin/rm -rf -- "$application_directory"
fi

if ! finish; then
  exit 1
fi

printf 'Own Domains setup removed.\n'
