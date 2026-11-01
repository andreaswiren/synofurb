#!/usr/bin/env bash
# Shared helpers for scripts/package-*.sh.  Source it:  . "$(dirname "${BASH_SOURCE[0]}")/package-lib.sh"
# shellcheck shell=bash

PKG_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

pkg_die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}
pkg_log() { printf '==> %s\n' "$*" >&2; }

# Workspace version from Cargo.toml ([workspace.package] version = "x.y.z") - the single source of truth.
pkg_version() {
    local v
    v="$(awk '/^\[workspace\.package\]/ { f = 1; next } /^\[/ { f = 0 } f && /^version[[:space:]]*=/ { gsub(/[" \t\r]/, ""); sub(/^version=/, ""); print; exit }' "$PKG_ROOT/Cargo.toml")"
    [ -n "$v" ] || pkg_die "cannot read [workspace.package] version from $PKG_ROOT/Cargo.toml"
    printf '%s\n' "$v" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.+-]+)?$' || pkg_die "workspace version '$v' is not SemVer"
    printf '%s' "$v"
}

# apps/mgmt-plane/VERSION
pkg_mgmt_version() {
    local v
    v="$(tr -d '[:space:]' <"$PKG_ROOT/apps/mgmt-plane/VERSION" 2>/dev/null || true)"
    [ -n "$v" ] || pkg_die "apps/mgmt-plane/VERSION is missing or empty"
    printf '%s' "$v"
}

# Rust target triple -> contract os / arch
pkg_triple_os() {
    case "$1" in
        *-pc-windows-*) echo windows ;;
        *-apple-darwin) echo macos ;;
        *-unknown-linux-gnu) echo linux ;;
        *) pkg_die "unsupported target triple: $1" ;;
    esac
}
pkg_triple_arch() {
    case "$1" in
        x86_64-*) echo x86_64 ;;
        aarch64-*) echo aarch64 ;;
        *) pkg_die "unsupported target triple: $1" ;;
    esac
}

# synofurb-<version>-<os>-<arch>.<ext>
pkg_asset_name() { # version os arch ext
    printf 'synofurb-%s-%s-%s.%s' "$1" "$2" "$3" "$4"
}

# synofurb-cli-<version>-<os>-<arch>.<ext>  (the headless command-line build; no GUI libraries)
pkg_cli_asset_name() { # version os arch ext
    printf 'synofurb-cli-%s-%s-%s.%s' "$1" "$2" "$3" "$4"
}

# a path in the form the host's native tools understand (Git Bash on Windows -> D:/a/..)
pkg_native_path() {
    if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}

pkg_python() {
    if command -v python3 >/dev/null 2>&1; then echo python3; elif command -v python >/dev/null 2>&1; then echo python; else pkg_die "python3 is required"; fi
}

pkg_sha256() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}
