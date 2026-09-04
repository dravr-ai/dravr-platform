#!/usr/bin/env bash
# ABOUTME: Process identity for the dev scripts — which pid this checkout started, and whose a port is
# ABOUTME: Sourced by start-server.sh, stop-server.sh, stop-all.sh, stop-tunnel.sh and the setup script
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A name — "pierre-mcp-server", "expo start", "whatever holds 8081" — names
# every checkout's stack at once, and several worktrees of this repo run side
# by side. Identity here is the pair (pid, kernel start stamp) this checkout
# records when it spawns a process, plus the checkout a running process
# resolves to through its executable or its working directory. Stopping is
# scoped to what this checkout recorded; only start-server.sh takes a port off
# a stranger, and it names the pid, its directory and its command first.

# This file lives in bin/, so one `..` is the checkout root — the directory
# whose logs/ holds the pid files and against which lsof paths are matched. The
# `${VAR:-...}` seam is what scripts/ci/dev-process-scope.test.sh overrides to
# point the library at a fixture checkout; one case there sources this file with
# no override so the default itself is exercised, because a root resolving one
# level too high makes every checkout under it share one set of pid files and
# read every neighbour's process as its own. `pwd -P` matters: /tmp and /var are
# symlinks on macOS and lsof reports the resolved side.
DEV_PROJECT_ROOT="${DEV_PROJECT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)}"
DEV_RUN_DIR="${DEV_RUN_DIR:-$DEV_PROJECT_ROOT/logs}"

dev_pid_file() { echo "$DEV_RUN_DIR/$1.pid"; }

# The instant the kernel started a process, whitespace-normalised. macOS pads
# `lstart` to a fixed width and `read` trims what it pads, so both sides of the
# comparison in dev_owned go through this one spelling.
dev_process_stamp() { # pid
    ps -p "$1" -o lstart= 2>/dev/null | awk '{ $1 = $1; print }'
}

# Record pid, process-group id and kernel start stamp.
dev_record() { # name pid
    local name="$1" pid="$2"
    mkdir -p "$DEV_RUN_DIR"
    printf '%s\n%s\n%s\n' \
        "$pid" \
        "$(ps -p "$pid" -o pgid= | tr -d ' ')" \
        "$(dev_process_stamp "$pid")" \
        >"$(dev_pid_file "$name")"
}

# Echo "<pid> <pgid>" when the recorded process is still the one this checkout
# started. The kernel reuses pid numbers, never a number together with the
# instant it was created, so the start stamp is what tells a live service apart
# from a stranger who inherited its number. A record that no longer describes a
# live process of ours is removed and the call returns 1.
dev_owned() { # name
    local name="$1" file pid pgid stamp
    file="$(dev_pid_file "$name")"
    [ -r "$file" ] || return 1
    { read -r pid; read -r pgid; read -r stamp; } <"$file" || { rm -f "$file"; return 1; }
    case "$pid" in '' | *[!0-9]*)
        rm -f "$file"
        return 1
        ;;
    esac
    kill -0 "$pid" 2>/dev/null || { rm -f "$file"; return 1; }
    if [ "$(dev_process_stamp "$pid")" != "$stamp" ]; then
        echo "  pid $pid was recycled by an unrelated process; leaving it alone" >&2
        rm -f "$file"
        return 1
    fi
    echo "$pid $pgid"
}

# TERM the process group, wait, then KILL it. The group is what makes this
# correct for bun and expo: the recorded pid is a supervisor, and the vite,
# esbuild, Metro and NativeWind workers it forks are its group members.
dev_stop() { # name label
    local owned pid pgid
    if ! owned="$(dev_owned "$1")"; then
        echo "  No recorded $2 for this checkout"
        return 0
    fi
    read -r pid pgid <<<"$owned"
    kill -TERM -- "-$pgid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.5
    done
    if kill -0 "$pid" 2>/dev/null; then
        kill -KILL -- "-$pgid" 2>/dev/null || kill -KILL "$pid" 2>/dev/null || true
    fi
    rm -f "$(dev_pid_file "$1")"
    echo "  Stopped: $2 (pid $pid)"
}

# Start a service in a process group of its own and record it. `set -m` is the
# mechanism: a backgrounded job under job control becomes its own group leader,
# so `kill -- -$pgid` later reaps the children it forks. The pid is published in
# DEV_SPAWNED_PID rather than echoed so the job stays a real child of the caller.
#
# stdin is /dev/null because that same group is a BACKGROUND group of the
# controlling terminal: a process in one that calls tcsetattr on the terminal
# receives SIGTTOU and is stopped by the kernel rather than run. `npx expo
# start` and `bun run dev` both install raw-mode keypress handlers, and both
# reach for tcsetattr only when stdin is a tty — a service running in the
# background has no keystrokes to read, so it is handed a stdin that is not one.
dev_spawn() { # name log_file cmd...
    local name="$1" log="$2"
    shift 2
    set -m
    "$@" </dev/null >"$log" 2>&1 &
    DEV_SPAWNED_PID=$!
    set +m
    dev_record "$name" "$DEV_SPAWNED_PID"
}

# Every pid listening on a port, one per line, deduplicated. A port normally has
# two listening sockets — the IPv4 one and the IPv6 one — and when it is
# contested they belong to different processes. Every decision below therefore
# runs over the whole set: reading one arbitrary listener reports a live server
# as absent (its neighbour answered lsof first) and kills one holder while the
# other keeps the port.
dev_port_listeners() { lsof -nP -iTCP:"$1" -sTCP:LISTEN -t 2>/dev/null | awk '!seen[$0]++'; }

# One listener, for a human-readable line. Decisions use the full set.
dev_port_listener() { dev_port_listeners "$1" | head -1; }

# True when the process resolves to THIS checkout: its executable lives under
# the worktree (the Rust binaries) or its working directory is the worktree or
# something inside it (bun, npx, node and cloudflared all run from a system-wide
# executable, and the start scripts cd to the project root before spawning).
dev_pid_is_ours() { # pid
    local path
    while IFS= read -r path; do
        case "$path" in "$DEV_PROJECT_ROOT" | "$DEV_PROJECT_ROOT"/*) return 0 ;; esac
    done < <(lsof -a -p "$1" -d txt,cwd -Fn 2>/dev/null | sed -n 's/^n//p')
    return 1
}

dev_describe_pid() { # pid
    echo "pid $1  dir=$(lsof -a -p "$1" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -1)  cmd=$(ps -p "$1" -o command= 2>/dev/null)"
}

# True when `pid` is behind the port, or leads the group of something that is.
# bun and expo fork the listener, so the pid a script records is a supervisor
# and the group is what ties the two together. Any one of the port's listeners
# answering to `pid` is enough: a server holding IPv4 while a stranger holds
# IPv6 is still the server this checkout started.
dev_pid_owns_port() { # pid port
    local pid="$1" listener want got
    want="$(ps -p "$pid" -o pgid= 2>/dev/null | tr -d ' ')"
    while IFS= read -r listener; do
        [ -n "$listener" ] || continue
        [ "$listener" = "$pid" ] && return 0
        got="$(ps -p "$listener" -o pgid= 2>/dev/null | tr -d ' ')"
        if [ -n "$want" ] && [ "$want" = "$got" ]; then return 0; fi
    done < <(dev_port_listeners "$2")
    return 1
}

# A 200 proves the PORT answered; it does not prove the answer came from the
# process this script started. Both halves are checked here, and the spawned
# process dying (an "Address already in use" exit, say) ends the wait at once
# instead of burning the full timeout against someone else's healthy server.
dev_wait_healthy() { # port pid [path] [tries]
    local port="$1" pid="$2" path="${3:-/health}" tries="${4:-30}"
    local _try listener
    for _try in $(seq 1 "$tries"); do
        if ! kill -0 "$pid" 2>/dev/null; then
            echo "  process $pid exited before it became healthy" >&2
            return 1
        fi
        if curl -s -f "http://127.0.0.1:$port$path" >/dev/null 2>&1; then
            dev_pid_owns_port "$pid" "$port" && return 0
            while IFS= read -r listener; do
                [ -n "$listener" ] || continue
                echo "  port $port answered 200 from $(dev_describe_pid "$listener")" >&2
            done < <(dev_port_listeners "$port")
            echo "  this checkout started pid $pid — it did not get the port" >&2
            return 1
        fi
        sleep 1
    done
    echo "  port $port did not become healthy within ${tries}s" >&2
    return 1
}

# Stop side. A stack started before this checkout kept pid files leaves a
# listener with no record; reclaim it when it resolves to this worktree, and
# name it and leave it running when it belongs to another. Each of the port's
# listeners is judged on its own, so our IPv4 socket is reclaimed while a
# peer's IPv6 socket on the same port stays up. Returns 1 when any listener
# belonged to another checkout, so the caller can say so once at the end.
dev_reclaim_port() { # port label
    local listener foreign=0
    local -a ours=()
    while IFS= read -r listener; do
        [ -n "$listener" ] || continue
        if dev_pid_is_ours "$listener"; then
            ours+=("$listener")
        else
            echo "  Port $1 is held by another checkout: $(dev_describe_pid "$listener")"
            foreign=1
        fi
    done < <(dev_port_listeners "$1")
    for listener in ${ours[@]+"${ours[@]}"}; do
        kill -TERM "$listener" 2>/dev/null || true
    done
    for listener in ${ours[@]+"${ours[@]}"}; do
        for _ in 1 2 3 4; do
            kill -0 "$listener" 2>/dev/null || break
            sleep 0.5
        done
        if kill -0 "$listener" 2>/dev/null; then
            kill -KILL "$listener" 2>/dev/null || true
        fi
        echo "  Stopped: $2 (port $1, pid $listener — this checkout, no pid file)"
    done
    return "$foreign"
}

# Start side. The port a start script was asked for belongs to that start
# script: every listener on it is named — pid, directory, command, so the
# operator can see whose stack was taken — and then each of those pids is
# signalled. Listeners, never a name pattern. Returns 1 only when the port is
# still held afterwards.
dev_take_port() { # port label
    local listener remaining
    remaining="$(dev_port_listeners "$1")"
    [ -n "$remaining" ] || return 0
    while IFS= read -r listener; do
        [ -n "$listener" ] || continue
        if dev_pid_is_ours "$listener"; then
            echo "  Port $1 held by this checkout's $2 with no pid file: $(dev_describe_pid "$listener")"
        else
            echo "  Port $1 is held by another stack; taking it: $(dev_describe_pid "$listener")"
        fi
        kill -TERM "$listener" 2>/dev/null || true
    done <<<"$remaining"
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        remaining="$(dev_port_listeners "$1")"
        [ -n "$remaining" ] || return 0
        sleep 0.5
    done
    while IFS= read -r listener; do
        [ -n "$listener" ] || continue
        kill -KILL "$listener" 2>/dev/null || true
    done <<<"$remaining"
    sleep 1
    remaining="$(dev_port_listeners "$1")"
    [ -n "$remaining" ] || return 0
    while IFS= read -r listener; do
        [ -n "$listener" ] || continue
        echo "  Port $1 is still held by $(dev_describe_pid "$listener")"
    done <<<"$remaining"
    return 1
}
