#!/usr/bin/env bash
# ABOUTME: Fixture test for bin/tunnel-env.sh — the one place BASE_URL is armed for a tunnel and reset
# ABOUTME: Pins that a dead quick-tunnel host is reset, a hand-set one is not, and no line beside it is lost
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# An env file a script rewrites and nobody checks is how a `>` redirect once
# truncated the lines beside the one it meant to change, and how a quick-tunnel
# hostname that had already stopped resolving survived in .envrc for days while
# every provider reconnect link built from it failed. .envrc is gitignored with
# no recovery path, so every case here runs on mktemp fixtures with a
# placeholder sentinel: nothing below reads the real .envrc.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/../../bin/tunnel-env.sh}"
REPO_ROOT="${REPO_ROOT:-$(cd "$SCRIPT_DIR/../.." && pwd)}"

failures=0
pass() { echo "  ✅ $1"; }
fail() {
    echo "  ❌ $1"
    failures=$((failures + 1))
}

expect() { # label actual expected
    if [ "$2" = "$3" ]; then pass "$1"; else
        fail "$1"
        echo "      got:      $2"
        echo "      expected: $3"
    fi
}

echo "tunnel-env.sh fixture tests"

# shellcheck source=../../bin/tunnel-env.sh
. "$UNDER_TEST"

TUNNEL_HOST="https://plymouth-animation-tigers-grid.trycloudflare.com"

# A checkout-shaped fixture: an .envrc carrying a sentinel secret beside the
# values under test.
make_fixture() { # base_url http_port -> prints the fixture root
    local base_url="$1" port="$2" d
    d="$(mktemp -d)"
    {
        echo "export HTTP_PORT=\"$port\""
        echo 'export PIERRE_MASTER_ENCRYPTION_KEY="fixture-not-a-real-key"'
        [ -n "$base_url" ] && echo "export BASE_URL=\"$base_url\""
        echo 'export RUST_LOG="info"'
    } >"$d/.envrc"
    echo "$d"
}

# --- 1/2: arming rewrites one line and destroys nothing beside it ---
d="$(make_fixture "http://localhost:8081" 8081)"
before_envrc_lines="$(wc -l <"$d/.envrc")"
tunnel_env_arm "$d" "$TUNNEL_HOST"

expect "arm rewrites the BASE_URL line" \
    "$(grep '^export BASE_URL=' "$d/.envrc")" \
    "export BASE_URL=\"$TUNNEL_HOST\""
expect "arm leaves every other .envrc line in place" \
    "$(wc -l <"$d/.envrc") $(grep -c '^export PIERRE_MASTER_ENCRYPTION_KEY=' "$d/.envrc")" \
    "$before_envrc_lines 1"
rm -rf "$d"

# --- 4: a dead quick tunnel is reset, with no tunnel process left to notice ---
d="$(make_fixture "$TUNNEL_HOST" 8081)"
if tunnel_env_reset "$d"; then pass "reset reports it rewrote a quick-tunnel host"; else
    fail "reset reports it rewrote a quick-tunnel host"
fi
expect "reset points BASE_URL back at the local server" \
    "$(grep '^export BASE_URL=' "$d/.envrc")" \
    'export BASE_URL="http://localhost:8081"'
expect "reset keeps every other .envrc line" \
    "$(grep -c '^export PIERRE_MASTER_ENCRYPTION_KEY=' "$d/.envrc")" "1"
rm -rf "$d"

# --- 5: an isolated worktree resets to its own port, not to 8081 ---
d="$(make_fixture "$TUNNEL_HOST" 8091)"
tunnel_env_reset "$d" || true
expect "reset honours the checkout's HTTP_PORT" \
    "$(grep '^export BASE_URL=' "$d/.envrc")" \
    'export BASE_URL="http://localhost:8091"'
rm -rf "$d"

# --- 6: resetting twice changes nothing the second time ---
d="$(make_fixture "$TUNNEL_HOST" 8081)"
tunnel_env_reset "$d" || true
cp "$d/.envrc" "$d/envrc.first"
if tunnel_env_reset "$d"; then
    fail "a second reset reports it had nothing to do"
else
    pass "a second reset reports it had nothing to do"
fi
if cmp -s "$d/.envrc" "$d/envrc.first"; then
    pass "a second reset is byte-for-byte idempotent"
else
    fail "a second reset is byte-for-byte idempotent"
fi
rm -rf "$d"

# --- 7: arming a file with no BASE_URL appends exactly one line ---
d="$(make_fixture "" 8081)"
before_envrc_lines="$(wc -l <"$d/.envrc")"
tunnel_env_arm "$d" "$TUNNEL_HOST"
expect "arm appends exactly one line when BASE_URL is absent" \
    "$(($(wc -l <"$d/.envrc") - before_envrc_lines))" "1"
expect "the appended line is the export form" \
    "$(tail -1 "$d/.envrc")" "export BASE_URL=\"$TUNNEL_HOST\""
rm -rf "$d"

# --- 8/9: a BASE_URL an operator set by hand is never overwritten ---
for hand_set in "http://192.168.1.42:8081" "https://pierre-dev.ngrok.io"; do
    d="$(make_fixture "$hand_set" 8081)"
    if tunnel_env_reset "$d"; then
        fail "reset leaves a hand-set BASE_URL alone ($hand_set)"
    else
        pass "reset leaves a hand-set BASE_URL alone ($hand_set)"
    fi
    expect "  BASE_URL is untouched ($hand_set)" \
        "$(grep '^export BASE_URL=' "$d/.envrc")" \
        "export BASE_URL=\"$hand_set\""
    rm -rf "$d"
done

# --- 11: the predicate itself, on the shapes that reach it ---
for ephemeral in \
    "https://plymouth-animation-tigers-grid.trycloudflare.com" \
    "https://demo-quick.trycloudflare.com/"; do
    if tunnel_env_is_ephemeral_url "$ephemeral"; then pass "ephemeral: $ephemeral"; else
        fail "ephemeral: $ephemeral"
    fi
done
for durable in \
    "" \
    "http://localhost:8081" \
    "http://192.168.1.42:8081" \
    "https://pierre-dev.ngrok.io" \
    "https://api.dravr.ai" \
    "https://trycloudflare.com.evil.example"; do
    if tunnel_env_is_ephemeral_url "$durable"; then fail "durable: ${durable:-(empty)}"; else
        pass "durable: ${durable:-(empty)}"
    fi
done

# --- 12: a rewrite leaves no second copy of the file behind ---
# `sed -i` needs a backup suffix on BSD sed, and that backup is a COMPLETE copy
# of the file — for .envrc, of every secret this project has, in a public
# repository. Two properties are pinned: the edit leaves nothing behind, and the
# suffix it would leave behind is one .gitignore already matches, so a copy that
# outlives an interrupt still cannot be staged.
d="$(make_fixture "http://localhost:8081" 8081)"
tunnel_env_arm "$d" "$TUNNEL_HOST"
tunnel_env_reset "$d" || true
strays="$(find "$d" -name '*bak*' -o -name '*.orig' -o -name '*.tmp' | sort | tr '\n' ' ')"
expect "arming and resetting leave no backup file behind" "$strays" ""
rm -rf "$d"

# --- 13: an interrupted rewrite leaves no copy, and could not be staged if it did ---
# The stand-in sed records the backup path the real call asked for, writes it,
# and then reproduces a Ctrl-C: the terminal signals every process in the
# foreground group, so the shim signals both the shell that invoked it and
# itself, and dies of the signal rather than returning a status. That lands the
# interrupt exactly where a Ctrl-C would — after the copy exists, before the
# edit returns. Recording the path is what ties the gitignore assertion below to
# the suffix the library actually passes, rather than to one spelled out here.
d="$(make_fixture "http://localhost:8081" 8081)"
shim="$(mktemp -d)"
cat >"$shim/sed" <<'SHIM'
#!/bin/sh
suffix=""
for arg in "$@"; do
    case "$arg" in -i*) suffix="${arg#-i}" ;; esac
    file="$arg"
done
printf '%s\n' "${file}${suffix}" >"$BACKUP_PATH_RECORD"
cp "$file" "${file}${suffix}"
kill -INT "$PPID"
kill -INT $$
exit 130
SHIM
chmod +x "$shim/sed"
export BACKUP_PATH_RECORD="$d/backup-path"
(
    PATH="$shim:$PATH"
    tunnel_env_edit_line "$d/.envrc" 's|^export BASE_URL=.*|export BASE_URL="x"|'
) >/dev/null 2>&1 || true

backup_path="$(cat "$BACKUP_PATH_RECORD" 2>/dev/null || true)"
if [ -n "$backup_path" ]; then
    pass "the rewrite goes through a backup the test can name"
else
    fail "the rewrite goes through a backup the test can name"
fi
if [ -n "$backup_path" ] && [ -e "$backup_path" ]; then
    fail "an interrupted rewrite leaves no copy of the file behind"
else
    pass "an interrupted rewrite leaves no copy of the file behind"
fi
expect "the file the interrupted rewrite was editing still carries its secret" \
    "$(grep -c '^export PIERRE_MASTER_ENCRYPTION_KEY=' "$d/.envrc")" "1"

# The suffix the library asked for, checked against the rule that has to cover
# it. The writer edits a path .gitignore never names, so the suffix is the only
# thing that can make a surviving copy unstageable — and .envrc holds every
# secret this project has, in a public repository.
backup_suffix="${backup_path#"$d/.envrc"}"
if [ -n "$backup_suffix" ] && git -C "$REPO_ROOT" check-ignore -q ".envrc${backup_suffix}"; then
    pass "a surviving .envrc${backup_suffix} is gitignored"
else
    fail "a surviving .envrc${backup_suffix:-<no suffix recorded>} is gitignored"
fi
unset BACKUP_PATH_RECORD
rm -rf "$shim" "$d"

# --- 14: the .envrc port read the stop scripts share with the tunnel reset ---
# bin/stop-all.sh and bin/stop-server.sh resolve the port they reclaim through
# this one function. Invoked without direnv there is no ambient HTTP_PORT, and
# the bare default is a DIFFERENT checkout's port.
d="$(make_fixture "http://localhost:8091" 8091)"
expect "the declared HTTP_PORT is read from .envrc" \
    "$(tunnel_env_declared_port "$d/.envrc" HTTP_PORT 8081)" "8091"
expect "a key .envrc does not declare falls back" \
    "$(tunnel_env_declared_port "$d/.envrc" FIXTURE_PORT 9555)" "9555"
echo 'export FIXTURE_PORT=9565' >>"$d/.envrc"
expect "a declared FIXTURE_PORT wins over the fallback" \
    "$(tunnel_env_declared_port "$d/.envrc" FIXTURE_PORT 9555)" "9565"
expect "a missing .envrc falls back rather than failing" \
    "$(tunnel_env_declared_port "$d/nope/.envrc" HTTP_PORT 8081)" "8081"
rm -rf "$d"

# --- 15: no dev script dials the tunnel origin by hostname ---
# The server binds IPv4 only (HOST="localhost" is not a SocketAddr, so
# multitenant.rs falls back to 127.0.0.1) while localhost resolves ::1 first on
# macOS, so a hostname origin gives cloudflared an address nothing listens on.
if grep -rn 'cloudflared tunnel --url http://localhost' "$REPO_ROOT/bin" >/dev/null 2>&1; then
    fail "no bin/ script points cloudflared at the localhost hostname"
    grep -rn 'cloudflared tunnel --url http://localhost' "$REPO_ROOT/bin"
else
    pass "no bin/ script points cloudflared at the localhost hostname"
fi

echo ""
if [ "$failures" -ne 0 ]; then
    echo "❌ $failures tunnel-env case(s) failed"
    exit 1
fi
echo "✅ all tunnel-env cases passed"
