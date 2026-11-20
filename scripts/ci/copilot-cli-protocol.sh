#!/usr/bin/env bash
# ABOUTME: Finds the newest Copilot CLI whose SDK protocol the locked github-copilot-sdk crate accepts.
# ABOUTME: Asks each candidate's runtime over `ping`, reads the accepted range from the crate's own source.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists (carnet#1637):
#   The copilot_sdk provider talks to the runtime the CLI extracts through the
#   github-copilot-sdk crate, which checks the runtime's protocol version at the
#   handshake and refuses one outside its MIN_PROTOCOL_VERSION..=SDK_PROTOCOL_VERSION.
#   CLI 1.0.113 moved to protocol 4 while every crate release still accepted 3
#   only, so following npm `latest` would have broken the tier at startup with
#   `version mismatch: server=4, supported=3–3`. The drift monitor proposes the
#   newest CLI this script accepts, never npm `latest` blindly.
#
# Usage:
#   copilot-cli-protocol.sh sdk-range <sdk-version>       prints "<min> <max>"
#   copilot-cli-protocol.sh cli <cli-version>             prints the runtime's protocol
#   copilot-cli-protocol.sh target <pinned> <sdk-version> prints key=value lines:
#     sdk_min, sdk_max, pinned_protocol, latest, latest_protocol, target, rejected
#
# `target` walks the stable npm releases newer than <pinned> from the newest
# down and stops at the first one the crate accepts; with none it answers
# <pinned>. It exits 1 when <pinned> itself speaks a protocol the crate rejects,
# since the image then already ships a broken copilot_sdk tier.
set -euo pipefail

# Set once by main, before any command substitution: a subshell's own
# mktemp and EXIT trap would delete the directory as soon as it returned.
WORK=""

platform() {
  local os arch
  case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=darwin ;;
    *) echo "unsupported OS: $(uname -s)" >&2; return 1 ;;
  esac
  case "$(uname -m)" in
    x86_64 | amd64) arch=x64 ;;
    arm64 | aarch64) arch=arm64 ;;
    *) echo "unsupported architecture: $(uname -m)" >&2; return 1 ;;
  esac
  printf '%s-%s\n' "$os" "$arch"
}

# Prints "<min> <max>" read from the crate source of exactly <version>.
sdk_range() {
  local version="$1" dir
  dir="$WORK/sdk-$version"
  mkdir -p "$dir"
  curl -fsSL -A "dravr-platform copilot-cli-protocol (github-actions)" \
    "https://static.crates.io/crates/github-copilot-sdk/github-copilot-sdk-$version.crate" \
    | tar xzf - -C "$dir" \
    || { echo "could not fetch github-copilot-sdk $version from crates.io" >&2; return 1; }
  protocol_range_in "$dir/github-copilot-sdk-$version/src"
}

# Prints "<min> <max>" from the MIN_PROTOCOL_VERSION and SDK_PROTOCOL_VERSION
# constants in a github-copilot-sdk source tree.
protocol_range_in() {
  local src="$1" min max
  [ -d "$src" ] || { echo "no github-copilot-sdk source at $src" >&2; return 1; }
  max="$(grep -rhoE 'SDK_PROTOCOL_VERSION: u32 = [0-9]+' "$src" | grep -oE '[0-9]+$' | head -1 || true)"
  min="$(grep -rhoE 'MIN_PROTOCOL_VERSION: u32 = [0-9]+' "$src" | grep -oE '[0-9]+$' | head -1 || true)"
  if [ -z "$min" ] || [ -z "$max" ]; then
    echo "protocol range not found in $src" >&2
    return 1
  fi
  printf '%s %s\n' "$min" "$max"
}

# Installs <version>, lets it extract its runtime, and asks the runtime its
# protocol over the SDK's own `ping` RPC (Content-Length framed JSON-RPC).
cli_protocol() {
  local version="$1" plat dir runtime answer
  plat="$(platform)"
  dir="$WORK/cli-$version"
  mkdir -p "$dir"
  npm install -g --silent --prefix "$dir/prefix" "@github/copilot-$plat@$version" >/dev/null
  COPILOT_PKG_CACHE_HOME="$dir/pkg-cache" COPILOT_AUTO_UPDATE=false \
    "$dir/prefix/bin/copilot-$plat" --version >/dev/null
  runtime="$dir/pkg-cache/pkg/$plat/$version/prebuilds/$plat/copilot-runtime"
  [ -x "$runtime" ] || { echo "CLI $version extracted no runtime at $runtime" >&2; return 1; }
  if ! answer="$(runtime_protocol "$runtime")"; then
    rm -rf "$dir"
    echo "CLI $version's runtime did not answer ping" >&2
    return 1
  fi
  rm -rf "$dir"
  case "$answer" in
    '' | *[!0-9]*) echo "CLI $version reported no protocol version" >&2; return 1 ;;
  esac
  printf '%s\n' "$answer"
}

# Sends one `ping` to <runtime> in server mode and prints the protocolVersion
# of the reply with id 1, skipping any other frame the runtime writes first.
runtime_protocol() {
  python3 -I - "$1" <<'EOF'
import json, subprocess, sys
body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": {}}).encode()
frame = b"Content-Length: %d\r\n\r\n" % len(body) + body
out = subprocess.run([sys.argv[1], "--server", "--stdio", "--no-auto-update"],
                     input=frame, capture_output=True, timeout=120).stdout
while out:
    head, sep, rest = out.partition(b"\r\n\r\n")
    if not sep:
        break
    length = next(int(line.split(b":", 1)[1]) for line in head.split(b"\r\n")
                  if line.lower().startswith(b"content-length:"))
    message, out = json.loads(rest[:length]), rest[length:]
    if message.get("id") == 1:
        print(message["result"]["protocolVersion"])
        sys.exit(0)
sys.exit("the runtime sent no reply to ping")
EOF
}

# Stable releases of the CLI, oldest first.
npm_versions() {
  npm view "@github/copilot-$(platform)" versions --json \
    | python3 -I -c 'import json,sys; print("\n".join(v for v in json.load(sys.stdin) if all(p.isdigit() for p in v.split("."))))' \
    | sort -V
}

target() {
  local pinned="$1" sdk="$2" range min max pinned_protocol versions latest
  local latest_protocol="" chosen="" rejected="" v p
  range="$(sdk_range "$sdk")"
  min="${range% *}"
  max="${range#* }"

  pinned_protocol="$(cli_protocol "$pinned")"
  case "$pinned_protocol" in
    '' | *[!0-9]*) echo "CLI $pinned answered protocol '$pinned_protocol', not a number" >&2; return 1 ;;
  esac
  if [ "$pinned_protocol" -lt "$min" ] || [ "$pinned_protocol" -gt "$max" ]; then
    echo "::error::the pinned Copilot CLI $pinned speaks protocol $pinned_protocol, outside the $min-$max github-copilot-sdk $sdk accepts (carnet#1637): the copilot_sdk tier cannot start" >&2
    return 1
  fi

  versions="$(npm_versions)"
  [ -n "$versions" ] || { echo "npm listed no stable Copilot CLI release" >&2; return 1; }
  latest="$(printf '%s\n' "$versions" | tail -1)"

  # Newest first, newer than the pin only.
  for v in $(printf '%s\n' "$versions" | sort -rV); do
    [ "$(printf '%s\n%s\n' "$v" "$pinned" | sort -V | tail -1)" = "$pinned" ] && break
    p="$(cli_protocol "$v")"
    case "$p" in
      '' | *[!0-9]*) echo "CLI $v answered protocol '$p', not a number" >&2; return 1 ;;
    esac
    [ "$v" = "$latest" ] && latest_protocol="$p"
    if [ "$p" -ge "$min" ] && [ "$p" -le "$max" ]; then
      chosen="$v"
      break
    fi
    rejected="${rejected:+$rejected }$v=$p"
  done
  [ "$latest" = "$pinned" ] && latest_protocol="$pinned_protocol"

  printf 'sdk_min=%s\nsdk_max=%s\npinned_protocol=%s\nlatest=%s\nlatest_protocol=%s\ntarget=%s\nrejected=%s\n' \
    "$min" "$max" "$pinned_protocol" "$latest" "$latest_protocol" "${chosen:-$pinned}" "$rejected"
}

main() {
  WORK="$(mktemp -d)"
  trap 'rm -rf "$WORK"' EXIT
  case "${1:-}" in
    sdk-range) [ $# -eq 2 ] || { echo "usage: $0 sdk-range <sdk-version>" >&2; exit 2; }; sdk_range "$2" ;;
    cli) [ $# -eq 2 ] || { echo "usage: $0 cli <cli-version>" >&2; exit 2; }; cli_protocol "$2" ;;
    target) [ $# -eq 3 ] || { echo "usage: $0 target <pinned> <sdk-version>" >&2; exit 2; }; target "$2" "$3" ;;
    *) echo "usage: $0 sdk-range <sdk> | cli <version> | target <pinned> <sdk>" >&2; exit 2 ;;
  esac
}

# Sourced by copilot-cli-protocol.test.sh, which replaces the network probes.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  main "$@"
fi
