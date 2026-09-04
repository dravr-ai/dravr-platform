#!/usr/bin/env bash
# ABOUTME: The one place BASE_URL and EXPO_PUBLIC_API_URL are armed for a tunnel and reset to local
# ABOUTME: Sourced by start-tunnel.sh, stop-tunnel.sh, start-server.sh, stop-all.sh and the setup script
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A cloudflared quick tunnel's hostname lives exactly as long as its process.
# .envrc and frontend-mobile/.env outlive it, so a hostname left behind in
# either resolves NXDOMAIN on the next run and the provider reconnect link an
# athlete taps — built from BASE_URL — reaches a browser that cannot find the
# host. Both files also hold values nothing here owns: .envrc carries every
# secret, and frontend-mobile/.env carries the Firebase and Google client ids.
# Every write below rewrites exactly one anchored line and passes every other
# line through untouched.

# True when a URL names a cloudflared quick tunnel — the only kind these
# scripts create, and the only kind whose host stops resolving on its own.
# A BASE_URL an operator set by hand (a named tunnel, an ngrok host, a LAN
# address for on-device testing) is a different thing and is never reset.
tunnel_env_is_ephemeral_url() { # url
    local host="${1#*://}"
    host="${host%%/*}"
    host="${host%%:*}"
    case "$host" in
        *.trycloudflare.com) return 0 ;;
        *) return 1 ;;
    esac
}

# The current value of an `export KEY="value"` line, with one layer of quotes
# removed. Reads that one line only; nothing here ever reads the file whole.
tunnel_env_get_export() { # file key
    local file="$1" key="$2" line value
    [ -f "$file" ] || return 0
    line="$(grep "^export ${key}=" "$file" 2>/dev/null | tail -1)" || return 0
    value="${line#export "${key}"=}"
    case "$value" in
        \"*\") value="${value#\"}" && value="${value%\"}" ;;
        \'*\') value="${value#\'}" && value="${value%\'}" ;;
    esac
    echo "$value"
}

# Same, for the KEY=value form Expo reads.
tunnel_env_get_plain() { # file key
    local file="$1" key="$2" line value
    [ -f "$file" ] || return 0
    line="$(grep "^${key}=" "$file" 2>/dev/null | tail -1)" || return 0
    value="${line#"${key}"=}"
    case "$value" in
        \"*\") value="${value#\"}" && value="${value%\"}" ;;
        \'*\') value="${value#\'}" && value="${value%\'}" ;;
    esac
    echo "$value"
}

# The numeric value of an `export KEY=<n>` line in .envrc, or the fallback when
# the file declares none. This is the one .envrc port read in the tree: the
# tunnel reset needs it to point BASE_URL back at the port this checkout serves
# on, and bin/stop-all.sh and bin/stop-server.sh need it to reclaim the port
# their own start scripts were pinned to. Invoked without direnv, an ambient
# HTTP_PORT is absent and 8081 is a different checkout's port.
tunnel_env_declared_port() { # envrc key fallback
    local port
    port="$(sed -n "s/^export ${2}=\"\{0,1\}\([0-9]\{1,\}\)\"\{0,1\}.*/\1/p" "$1" 2>/dev/null | tail -1)"
    echo "${port:-$3}"
}

# The URL the server answers on with no tunnel up. The port comes from the
# HTTP_PORT already declared in .envrc, so a worktree on 8091 resets to 8091.
tunnel_env_local_base_url() { # envrc
    echo "http://localhost:$(tunnel_env_declared_port "$1" HTTP_PORT 8081)"
}

# Rewrite one anchored line in place. `sed -i` needs a backup suffix on BSD sed,
# and that backup is a complete second copy of the file — for .envrc, of every
# secret this project has, in a public repository. `.bak` is the suffix
# .gitignore already matches, so the copy is unstageable while it exists, and
# the subshell trap removes it on every way out of the edit, an interrupt
# included. The trap lives in a subshell so it leaves the calling script's own
# traps alone; BSD and GNU sed both rename the finished temp file over the
# original, so the file itself is never half-written for the trap to salvage.
tunnel_env_edit_line() { # file sed_expression
    local file="$1" expression="$2"
    (
        trap 'rm -f "$file.bak"' EXIT HUP INT TERM
        sed -i.bak "$expression" "$file"
    )
}

# Rewrite one `export KEY="value"` line, appending it when the key is absent.
tunnel_env_set_export() { # file key value
    local file="$1" key="$2" value="$3"
    if [ -f "$file" ] && grep -q "^export ${key}=" "$file"; then
        tunnel_env_edit_line "$file" "s|^export ${key}=.*|export ${key}=\"${value}\"|"
    else
        echo "export ${key}=\"${value}\"" >>"$file"
    fi
}

# Same, for the KEY=value form Expo reads.
tunnel_env_set_plain() { # file key value
    local file="$1" key="$2" value="$3"
    if [ -f "$file" ] && grep -q "^${key}=" "$file"; then
        tunnel_env_edit_line "$file" "s|^${key}=.*|${key}=\"${value}\"|"
    else
        echo "${key}=\"${value}\"" >>"$file"
    fi
}

# Point both files at a live tunnel.
tunnel_env_arm() { # root url
    local root="$1" url="$2"
    tunnel_env_set_export "$root/.envrc" BASE_URL "$url"
    tunnel_env_set_plain "$root/frontend-mobile/.env" EXPO_PUBLIC_API_URL "$url"
}

# Point back at the local server every value that names a quick tunnel, and
# leave every other value alone. Each key is judged on its own: start-server.sh
# --tunnel writes the mobile file while exporting BASE_URL only in-process, so
# one can name a dead tunnel while the other is already local. Returns 0 when
# something was rewritten, 1 when both values were left as the operator set
# them — which is also the answer when a tunnel died on its own and the files
# were already clean.
tunnel_env_reset() { # root
    local root="$1" local_url current reset=1
    local_url="$(tunnel_env_local_base_url "$root/.envrc")"
    current="$(tunnel_env_get_export "$root/.envrc" BASE_URL)"
    if tunnel_env_is_ephemeral_url "$current"; then
        tunnel_env_set_export "$root/.envrc" BASE_URL "$local_url"
        reset=0
    fi
    current="$(tunnel_env_get_plain "$root/frontend-mobile/.env" EXPO_PUBLIC_API_URL)"
    if tunnel_env_is_ephemeral_url "$current"; then
        tunnel_env_set_plain "$root/frontend-mobile/.env" EXPO_PUBLIC_API_URL "$local_url"
        reset=0
    fi
    return "$reset"
}
