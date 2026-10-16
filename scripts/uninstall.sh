#!/bin/sh

set -eu

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

case "$application_directory" in
  /Users/*/Library/Application\ Support/OwnDomains) ;;
  *) printf 'Refusing unsafe application-data path: %s\n' "$application_directory" >&2; exit 2 ;;
esac

printf 'UNLOAD dev.owndomains.backend\n'
printf 'UNLOAD dev.owndomains.caddy\n'
printf 'UNLOAD dev.owndomains.dnsmasq\n'
printf 'UNINSTALL managed setup files\n'
if [ "$purge" = true ]; then
  printf 'PURGE %s\n' "$application_directory"
fi

if [ "$dry_run" = true ]; then
  exit 0
fi

if [ "$(uname -s)" != Darwin ]; then
  printf 'Own Domains uninstall currently supports macOS only.\n' >&2
  exit 1
fi

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"
pnpm_bin=$(command -v pnpm) || { printf 'pnpm is required. Install it with: brew install pnpm\n' >&2; exit 1; }
user_id=$(id -u)

/bin/launchctl bootout "gui/$user_id/dev.owndomains.backend" 2>/dev/null || true
/usr/bin/sudo /bin/launchctl bootout system/dev.owndomains.caddy 2>/dev/null || true
/usr/bin/sudo /bin/launchctl bootout system/dev.owndomains.dnsmasq 2>/dev/null || true
/usr/bin/sudo "$pnpm_bin" run uninstall -- --apply

if [ "$purge" = true ]; then
  /usr/bin/sudo /bin/rm -rf -- "$application_directory"
fi

printf 'Own Domains setup removed.\n'
