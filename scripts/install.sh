#!/bin/sh
# Installer for the native Own Domains app.
#
# Builds the desktop app and its CLI from this checkout, installs the app,
# previews privileged host setup, then applies it once through sudo. The GUI
# and daemon always run unprivileged; only setup needs administrator approval.
#
# Build details live in docs/desktop.md and docs/linux-install.md.

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
interface=

usage() {
  printf 'Usage: %s [--dry-run] [--interface=NAME] <LAN_IPv4>\n' "$0" >&2
}

for argument in "$@"; do
  case "$argument" in
    --dry-run) dry_run=true ;;
    --interface=*) interface=${argument#--interface=} ;;
    --*) printf 'Unknown option: %s\n' "$argument" >&2; exit 2 ;;
    *)
      if [ -n "$lan_address" ]; then
        usage
        exit 2
      fi
      lan_address=$argument
      ;;
  esac
done

if [ -z "$lan_address" ]; then
  usage
  exit 2
fi

# macOS requires an explicit interface (for example en0 for Wi-Fi);
# Linux discovers the covered interfaces when it is omitted.
if [ "$(uname -s)" = Darwin ] && [ -z "$interface" ]; then
  usage
  exit 2
fi

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"
export PATH="$HOME/.cargo/bin:$PATH"

os=$(uname -s)
arch=$(uname -m)
build_script=
cli_bin=
app_bundle=
case "$os" in
  Darwin)
    case "$arch" in
      arm64) build_script=desktop:build:arm64; triple=aarch64-apple-darwin ;;
      x86_64) build_script=desktop:build:x86_64; triple=x86_64-apple-darwin ;;
      *) printf 'Unsupported macOS architecture: %s\n' "$arch" >&2; exit 1 ;;
    esac
    cli_bin="$repository_root/target/$triple/release/own-domains"
    app_bundle="$repository_root/target/$triple/release/bundle/macos/Own Domains.app"
    ;;
  Linux)
    build_script=desktop:build:linux
    triple=x86_64-unknown-linux-gnu
    cli_bin="$repository_root/target/$triple/release/own-domains"
    ;;
  *)
    printf 'This installer supports macOS and Linux. On Windows, build the NSIS installer as described in docs/desktop.md.\n' >&2
    exit 1
    ;;
esac

if [ "$dry_run" = true ]; then
  printf 'BUILD native CLI for the setup preview\n'
  run_or_fail 'CLI build failed; no system changes were made' cargo build -p own-domains-cli
  cli_bin="$repository_root/target/debug/own-domains"
else
  printf 'INSTALL locked dependencies\n'
  run_or_fail 'dependency installation failed; no system changes were made' pnpm install --frozen-lockfile

  printf 'FETCH pinned sidecars\n'
  run_or_fail 'sidecar download failed; no system changes were made' pnpm run sidecars:fetch
  if [ "$os" = Linux ]; then
    run_or_fail 'sidecar download failed; no system changes were made' pnpm run sidecars:fetch -- --platform windows
    run_or_fail 'sidecar download failed; no system changes were made' pnpm run sidecars:fetch -- --platform linux
  fi

  printf 'BUILD desktop app (%s)\n' "$build_script"
  run_or_fail 'desktop build failed; no system changes were made' pnpm run "$build_script"
fi

if [ -n "$interface" ]; then
  set -- --lan-address "$lan_address" --interface "$interface"
else
  set -- --lan-address "$lan_address"
fi

printf 'PREVIEW privileged setup\n'
run_or_fail 'setup validation failed; no system changes were made' "$cli_bin" setup --dry-run "$@"

if [ "$dry_run" = true ]; then
  exit 0
fi

case "$os" in
  Darwin)
    [ -d "$app_bundle" ] || {
      printf 'ERROR: built app bundle not found at %s\n' "$app_bundle" >&2
      exit 1
    }
    target_app="/Applications/Own Domains.app"
    [ "$target_app" != "/" ] || exit 1
    printf 'INSTALL app to %s (administrator approval required)\n' "$target_app"
    run_or_fail 'could not replace the installed app' sudo rm -rf "$target_app"
    run_or_fail 'could not copy the app bundle' sudo ditto "$app_bundle" "$target_app"
    [ -f "$cli_bin" ] || {
      printf 'ERROR: built CLI not found at %s\n' "$cli_bin" >&2
      exit 1
    }
    printf 'INSTALL %s on PATH\n' "own-domains"
    run_or_fail 'could not install the CLI' sudo mkdir -p /usr/local/bin
    run_or_fail 'could not install the CLI' sudo install -m 755 "$cli_bin" /usr/local/bin/own-domains
    ;;
  Linux)
    deb_file=
    for candidate in \
      "$repository_root"/target/*/release/bundle/deb/*.deb \
      "$repository_root"/crates/own-domains-desktop/target/*/release/bundle/deb/*.deb \
      "$repository_root"/crates/own-domains-desktop/target/release/bundle/deb/*.deb; do
      [ -f "$candidate" ] || continue
      deb_file=$candidate
    done
    [ -n "$deb_file" ] || {
      printf 'ERROR: no .deb package found under crates/own-domains-desktop/target\n' >&2
      exit 1
    }
    printf 'INSTALL %s (administrator approval required)\n' "$deb_file"
    run_or_fail 'package install failed; run sudo apt-get install -f and retry' sudo dpkg -i "$deb_file"
    ;;
esac

printf 'APPLY privileged setup (administrator approval required)\n'
run_or_fail 'setup failed; review the error and rollback report above' sudo -E "$cli_bin" setup "$@"

printf 'Install complete. Open Own Domains and press Start.\n'
