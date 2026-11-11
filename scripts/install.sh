#!/bin/sh
# Installer for the native Own Domains app and CLI.
#
# The app bundle and the CLI install independently: --app-only installs just
# the desktop bundle, --cli-only installs just the self-contained CLI prefix
# (binaries, daemon, and sidecars under <prefix>/lib/own-domains, with an
# `own-domains` symlink on PATH). By default both are installed, each with
# its own copy of the resources, so either one works on its own. After
# installing, the script previews privileged host setup, then applies it once
# through sudo. The GUI and daemon always run unprivileged; only setup needs
# administrator approval.
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

# Stage a self-contained CLI prefix: the CLI and daemon in bin/, the macOS
# bundle-mirrored Resources/ tree (sidecar binaries plus the per-target
# manifest), and exe-adjacent plain sidecar names that the Linux backend
# resolves when no bundle directory is present.
stage_cli_prefix() {
  stage_dir=$1
  triple=$2
  profile_bin_dir=$3
  sidecar_dir=$4
  bin_dir="$stage_dir/bin"
  resource_dir="$stage_dir/Resources"
  run_or_fail 'could not create the CLI staging directory' mkdir -p "$bin_dir" "$resource_dir/sidecars"
  run_or_fail 'could not stage the CLI binaries' cp "$profile_bin_dir/own-domains" "$profile_bin_dir/own-domains-daemon" "$bin_dir/"
  run_or_fail 'could not stage the sidecars' cp "$sidecar_dir/coredns-$triple" "$sidecar_dir/caddy-$triple" "$resource_dir/"
  run_or_fail 'could not stage the sidecar manifest' cp "$sidecar_dir/manifest.json" "$resource_dir/sidecars/manifest-$triple.json"
  run_or_fail 'could not link the exe-adjacent sidecars' ln -sf "../Resources/coredns-$triple" "$bin_dir/coredns"
  run_or_fail 'could not link the exe-adjacent sidecars' ln -sf "../Resources/caddy-$triple" "$bin_dir/caddy"
  [ -x "$bin_dir/own-domains" ] || {
    printf 'ERROR: staged CLI is not executable at %s\n' "$bin_dir/own-domains" >&2
    exit 1
  }
}

dry_run=false
lan_address=
interface=
mode=both
prefix=/usr/local

usage() {
  printf 'Usage: %s [--dry-run] [--cli-only] [--app-only] [--prefix=DIR] [--interface=NAME] <LAN_IPv4>\n' "$0" >&2
}

for argument in "$@"; do
  case "$argument" in
    --dry-run) dry_run=true ;;
    --cli-only) mode=cli ;;
    --app-only) mode=app ;;
    --prefix=*) prefix=${argument#--prefix=} ;;
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
      arm64) triple=aarch64-apple-darwin ;;
      x86_64) triple=x86_64-apple-darwin ;;
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
  run_or_fail 'CLI build failed; no system changes were made' cargo build -p own-domains-cli -p own-domains-daemon
  if [ "$mode" != app ]; then
    # Preview through a staged CLI prefix so the dry run exercises the same
    # self-contained layout the real CLI install uses. Without staged
    # sidecars, fall back to the bare debug CLI.
    host_triple=$(rustc -vV | sed -n 's/^host: //p')
    if [ -d "$repository_root/sidecars/build/$host_triple" ]; then
      preview_stage=$(mktemp -d)
      stage_cli_prefix "$preview_stage" "$host_triple" "$repository_root/target/debug" "$repository_root/sidecars/build/$host_triple"
      cli_bin="$preview_stage/bin/own-domains"
    else
      cli_bin="$repository_root/target/debug/own-domains"
    fi
  else
    cli_bin="$repository_root/target/debug/own-domains"
  fi
else
  printf 'INSTALL locked dependencies\n'
  run_or_fail 'dependency installation failed; no system changes were made' pnpm install --frozen-lockfile

  printf 'FETCH pinned sidecars\n'
  run_or_fail 'sidecar download failed; no system changes were made' pnpm run sidecars:fetch
  if [ "$os" = Linux ]; then
    run_or_fail 'sidecar download failed; no system changes were made' pnpm run sidecars:fetch -- --platform windows
    run_or_fail 'sidecar download failed; no system changes were made' pnpm run sidecars:fetch -- --platform linux
  fi

  if [ "$mode" != app ]; then
    printf 'BUILD release CLI and daemon for %s\n' "$triple"
    run_or_fail 'Rust binaries failed to build; no system changes were made' \
      cargo build --release --target "$triple" -p own-domains-daemon -p own-domains-cli
    stage_dir="$repository_root/target/$triple/cli-prefix"
    run_or_fail 'could not clear the CLI staging directory' rm -rf "$stage_dir"
    stage_cli_prefix "$stage_dir" "$triple" "$repository_root/target/$triple/release" "$repository_root/sidecars/build/$triple"
    cli_bin="$stage_dir/bin/own-domains"
  fi

  if [ "$mode" != cli ] && [ "$os" = Darwin ]; then
    printf 'BUILD desktop app bundle for %s\n' "$triple"
    # The macOS package scripts skip bundling (`--no-bundle`), so build the
    # .app here: it carries its own CLI, daemon, and sidecars.
    run_or_fail 'Rust binaries failed to build; no system changes were made' \
      cargo build --release --target "$triple" -p own-domains-daemon -p own-domains-cli
    run_or_fail 'binary staging failed; no system changes were made' \
      pnpm exec tsx scripts/stage-desktop.ts --release --target "$triple"
    run_or_fail 'desktop build failed; no system changes were made' \
      sh -c "cd crates/own-domains-desktop && pnpm exec tauri build --target $triple --bundles app"
  fi

  if [ "$mode" != cli ]; then
    if [ "$os" = Darwin ]; then
      bundled_cli="$app_bundle/Contents/MacOS/own-domains"
      [ -x "$bundled_cli" ] || {
        printf 'ERROR: built CLI missing from the app bundle at %s\n' "$bundled_cli" >&2
        exit 1
      }
      [ "$mode" = app ] && cli_bin=$bundled_cli
    else
      run_or_fail 'desktop build failed; no system changes were made' pnpm run "$build_script"
    fi
  fi
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

# A custom prefix is a user-writable test/dev install; the default prefix
# needs administrator approval for the file copies below.
if [ "$prefix" = /usr/local ]; then
  file_sudo=sudo
else
  file_sudo=
fi

if [ "$mode" != app ]; then
  lib_dir="$prefix/lib/own-domains"
  printf 'INSTALL CLI prefix to %s\n' "$lib_dir"
  run_or_fail 'could not create the CLI prefix' $file_sudo mkdir -p "$lib_dir" "$prefix/bin"
  run_or_fail 'could not copy the CLI prefix' $file_sudo cp -R "$stage_dir/bin" "$stage_dir/Resources" "$lib_dir/"
  run_or_fail 'could not fix CLI prefix permissions' $file_sudo chmod 755 "$lib_dir/bin/own-domains" "$lib_dir/bin/own-domains-daemon"
  run_or_fail 'could not link the CLI on PATH' $file_sudo ln -sf ../lib/own-domains/bin/own-domains "$prefix/bin/own-domains"
  installed_cli="$prefix/bin/own-domains"
fi

if [ "$mode" != cli ]; then
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
    [ "$mode" = app ] && installed_cli="$target_app/Contents/MacOS/own-domains"
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
    [ "$mode" = app ] && installed_cli=$(command -v own-domains 2>/dev/null || true)
    ;;
esac
fi

printf 'APPLY privileged setup (administrator approval required)\n'
run_or_fail 'setup failed; review the error and rollback report above' sudo -E "${installed_cli:-$cli_bin}" setup "$@"

if [ "$mode" != cli ]; then
  printf 'Install complete. Open Own Domains and press Start.\n'
else
  printf 'Install complete. Run `own-domains start` to launch the daemon.\n'
fi
