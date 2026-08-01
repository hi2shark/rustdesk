#!/usr/bin/env bash
# Derive app/package VERSION from CI TAG_NAME (git tag / workflow upload-tag)
# and patch Cargo.toml / pubspec so gen_version() and packaging match the release tag.
#
# TAG_NAME examples: v1.5.0-1, 1.5.0-1, nightly
# - Semver tags (with optional -N patch) become VERSION (leading "v" stripped).
# - Non-semver tags (e.g. nightly) fall back to Cargo.toml package version.
#
# Run from the repository root. Safe on Linux / macOS / Windows (Git Bash).
set -euo pipefail

TAG_NAME="${TAG_NAME:-}"
VER="${TAG_NAME#v}"

is_release_ver() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9]+)?$ ]]
}

cargo_pkg_version() {
  local file="${1:-Cargo.toml}"
  grep -m1 '^version' "$file" | sed -E 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/'
}

sed_inplace() {
  local expr="$1"
  local file="$2"
  if sed --version >/dev/null 2>&1; then
    sed -i -E "$expr" "$file"
  else
    sed -i '' -E "$expr" "$file"
  fi
}

if ! is_release_ver "$VER"; then
  if [[ -f Cargo.toml ]]; then
    VER="$(cargo_pkg_version Cargo.toml)"
  elif [[ -n "${VERSION:-}" ]]; then
    VER="$VERSION"
  else
    echo "Unable to determine version from TAG_NAME='$TAG_NAME'" >&2
    exit 1
  fi
  echo "TAG_NAME='$TAG_NAME' is not a release version; falling back to VERSION=$VER"
fi

echo "Using VERSION=$VER (TAG_NAME='$TAG_NAME')"

if [[ -n "${GITHUB_ENV:-}" ]]; then
  echo "VERSION=$VER" >> "$GITHUB_ENV"
fi

CARGO_VERSION_CHANGED=false

if [[ -f Cargo.toml ]]; then
  if [[ "$(cargo_pkg_version Cargo.toml)" != "$VER" ]]; then
    sed_inplace "0,/^version = \".*\"/s//version = \"$VER\"/" Cargo.toml
    CARGO_VERSION_CHANGED=true
  fi
fi

if [[ -f libs/portable/Cargo.toml ]]; then
  if [[ "$(cargo_pkg_version libs/portable/Cargo.toml)" != "$VER" ]]; then
    sed_inplace "0,/^version = \".*\"/s//version = \"$VER\"/" libs/portable/Cargo.toml
    CARGO_VERSION_CHANGED=true
  fi
fi

# CI builds with --locked, so keep workspace package versions aligned in Cargo.lock.
if [[ "$CARGO_VERSION_CHANGED" == true && -f Cargo.lock ]]; then
  cargo update --workspace
fi

if [[ -f flutter/pubspec.yaml ]]; then
  line="$(grep -E '^version:' flutter/pubspec.yaml | head -1)"
  BUILD_NUM="$(echo "$line" | sed -E 's/.*\+([0-9]+).*/\1/')"
  if [[ -z "$BUILD_NUM" || "$BUILD_NUM" == "$line" ]]; then
    BUILD_NUM=1
  fi
  sed_inplace "s/^version: .*/version: ${VER}+${BUILD_NUM}/" flutter/pubspec.yaml
fi
