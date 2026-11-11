#!/bin/sh
# Uninstaller for the native Own Domains app.
#
# Removes the managed host integration, then the installed app package.
# Independent steps continue after an error and every failure is reported
# with a nonzero status at the end.

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

if [ "$purge" = true ] && [ "$dry_run" = true ]; then
  printf 'ERROR: --purge cannot be combined with --dry-run\n' >&2
  exit 2
fi

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"
export PATH="$HOME/.cargo/bin:$PATH"

os=$(uname -s)

# Prefer the installed CLI (the Linux package puts it on PATH); fall back to
# a debug build from this checkout for macOS and uninstalled checkouts.
cli_bin=$(command -v own-domains 2>/dev/null || true)
if [ -z "$cli_bin" ]; then
  printf 'BUILD native CLI\n'
  if ! cargo build -p own-domains-cli; then
    record_error 'CLI build failed; nothing was removed'
    finish
    exit 1
  fi
  cli_bin="$repository_root/target/debug/own-domains"
fi

if [ "$dry_run" = true ]; then
  printf 'PREVIEW host integration removal\n'
  try_step 'uninstall preview failed' "$cli_bin" uninstall --dry-run
  finish
  exit 0
fi

printf 'UNINSTALL managed host integration (administrator approval required)\n'
if [ "$purge" = true ]; then
  try_step 'managed-file uninstall failed' sudo -E "$cli_bin" uninstall --purge
else
  try_step 'managed-file uninstall failed' sudo -E "$cli_bin" uninstall
fi

case "$os" in
  Darwin)
    target_app="/Applications/Own Domains.app"
    if [ -d "$target_app" ]; then
      printf 'UNINSTALL app from %s (administrator approval required)\n' "$target_app"
      try_step 'could not remove the installed app' sudo rm -rf "$target_app"
    fi
    if [ -f /usr/local/bin/own-domains ]; then
      try_step 'could not remove the own-domains CLI' sudo rm -f /usr/local/bin/own-domains
    fi
    ;;
  Linux)
    if dpkg -s own-domains >/dev/null 2>&1; then
      printf 'UNINSTALL own-domains package (administrator approval required)\n'
      try_step 'could not remove the own-domains package' sudo dpkg -r own-domains
    fi
    ;;
esac

if ! finish; then
  exit 1
fi

printf 'Own Domains removed.\n'
