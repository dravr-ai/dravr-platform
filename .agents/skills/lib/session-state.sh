#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: The one account-independent home for per-session state that carnet and bilan share
# ABOUTME: Sourced, never executed; state_adopt folds each Claude account's old copy into it, once
#
# ChefFamille runs Claude Code under several accounts (CLAUDE_CONFIG_DIR = ~/.claude,
# ~/.claude-gatling, ~/.claude-perso) and switches between them, sometimes inside one session: the
# same CLAUDE_CODE_SESSION_ID carries on under another config dir. While the claims ledger and
# bilan's baseline lived under $CLAUDE_CONFIG_DIR, each account held part of such a session — a
# claim under one, the issues it filed under the other — and bilan scored whichever half the
# current account could see (carnet#670).
#
# So that state lives in one place, whatever account is active:
#
#   ${DRAVR_SESSION_STATE:-${XDG_STATE_HOME:-$HOME/.local/state}/dravr/sessions}
#     carnet-claims/<session>.jsonl   the claims ledger, plus pending/, warned/, cache/, gh-login
#     bilan/<session>.*               baseline (+ .commits .head .stack), ack, score, stop-gate state
#
# Other writers still address $CLAUDE_CONFIG_DIR/carnet-claims directly: carnet copies in other
# repos, older platform worktrees, the status line. state_adopt replaces each account's directory
# with a symlink to the shared one, so those keep working untouched and nothing reads two places.
# Only Claude Code's own files — sessions/, projects/, tasks/ — stay per account; bilan reads
# those across every account itself.
#
# Portability: macOS bash 3.2 and BSD tools as well as Linux. mkdir is the lock (atomic on both),
# `[ a -nt b ]` compares mtimes (no stat spelling differs), and nothing here uses readlink -f.

SESSION_STATE_HOME=${DRAVR_SESSION_STATE:-${XDG_STATE_HOME:-$HOME/.local/state}/dravr/sessions}
LEDGER_DIR="$SESSION_STATE_HOME/carnet-claims"
BILAN_DIR="$SESSION_STATE_HOME/bilan"
STATE_SUBDIRS="carnet-claims bilan"

# ------------------------------------------------------------------ accounts
# Every Claude Code config dir on this machine, each physical directory once: the active one, and
# every ~/.claude* directory that is one. A config dir is recognised by what Claude Code puts in
# it, or by holding state this file has not adopted yet; ~/.claude.json and other lookalikes are
# skipped.
state_account_dirs() {
    local d phys seen="|" active=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
    for d in "$active" "$HOME"/.claude*; do
        [ -d "$d" ] || continue
        if [ "$d" != "$active" ] \
           && [ ! -e "$d/sessions" ] && [ ! -e "$d/projects" ] && [ ! -e "$d/settings.json" ] \
           && [ ! -d "$d/carnet-claims" ] && [ ! -d "$d/bilan" ]; then
            continue
        fi
        phys=$(cd "$d" 2>/dev/null && pwd -P) || continue
        case $seen in *"|$phys|"*) continue ;; esac
        seen="$seen$phys|"
        printf '%s\n' "$d"
    done
}

# 0 when some account still has a real state directory, or none at all where the link belongs.
# This is the whole cost of an adopted machine: a glob and a few stats.
# A directory renamed aside by an adoption that was killed mid-merge (a SessionStart hook runs
# under a timeout) is pending too, or its files would be stranded behind a link that already exists.
state_pending_adoption() {
    local d sub left
    while IFS= read -r d; do
        for sub in $STATE_SUBDIRS; do
            [ -L "$d/$sub" ] || return 0
            for left in "$d/$sub".adopting.*; do [ -d "$left" ] && return 0; done
        done
    done < <(state_account_dirs)
    return 1
}

# ------------------------------------------------------------------ locking
# mkdir is the lock: atomic on every platform this runs on, bash 3.2 included. A holder that was
# killed leaves the lock behind; its pid says so at once, and a lock with no pid yet is either
# being taken this instant or older than <stale-minutes>. 1 when a live holder keeps it past
# about five seconds, and the caller decides what that means for its write.
state_lock() { # <lock-dir> <stale-minutes>
    local waited=0 holder
    until mkdir "$1" 2>/dev/null; do
        holder=$(cat "$1/pid" 2>/dev/null || true)
        if { [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; } \
           || [ -n "$(find "$1" -maxdepth 0 -mmin +"$2" 2>/dev/null)" ]; then
            rm -rf "$1"
            continue
        fi
        waited=$((waited + 1))
        [ "$waited" -le 25 ] || return 1
        sleep 0.2
    done
    printf '%s\n' "$$" > "$1/pid"
}

state_unlock() { rm -rf "$1"; }

# The lock every writer of one session's ledger holds: carnet's writes, and bilan dropping a claim
# the tracker has closed. Parallel tool calls fire one auto-claim hook each for the same session,
# and a write that rewrites the file loses a line another process appended between its read and
# its mv. A holder still busy past the wait is written through rather than waited on: a claim
# missing from the ledger costs more than a race that rarely lands.
state_ledger_lock() { # <ledger-file> -> 0 when this process now holds the lock
    mkdir -p "${1%/*}" 2>/dev/null || return 1
    state_lock "$1.lock" 1
}

state_ledger_unlock() { # <ledger-file> <state_ledger_lock status>
    [ "$2" != 0 ] || state_unlock "$1.lock"
    return 0
}

# ------------------------------------------------------------------ liveness
# 0 when a Claude Code process is running this session on this machine. Claude Code writes
# sessions/<pid>.json while a session runs and removes it on exit. The pid recorded when a claim
# was made is NOT enough: a session resumed under another account runs under a new pid with the
# same id, and reading the old pid alone called that live session dead.
state_session_alive() { # <session-id>
    local sid=$1 d f p
    [ -n "$sid" ] || return 1
    for d in "${CLAUDE_CONFIG_DIR:-$HOME/.claude}" "$HOME"/.claude*; do
        [ -d "$d/sessions" ] || continue
        for f in $(grep -l -F "\"$sid\"" "$d"/sessions/*.json 2>/dev/null); do
            [ "$(jq -r '.sessionId // empty' "$f" 2>/dev/null)" = "$sid" ] || continue
            p=$(jq -r '.pid // empty' "$f" 2>/dev/null)
            [ -n "$p" ] || p=$(basename "$f" .json)
            case $p in '' | *[!0-9]*) continue ;; esac
            kill -0 "$p" 2>/dev/null && return 0
        done
    done
    return 1
}

# ------------------------------------------------------------------ merging
# Two ledgers of the same session, one per account. One identity line: the EARLIEST .at, because
# bilan dates the session's start from it, with pid, name and branch from the newer file. Every
# other line once, keyed on what it says (kind, tracker, issue, commit), at its earliest stamp.
state_merge_ledger() { # <src> <dst>
    local older newer tmp line t n state
    if [ "$1" -nt "$2" ]; then older=$2; newer=$1; else older=$1; newer=$2; fi
    tmp=$(mktemp "${TMPDIR:-/tmp}/state-ledger.XXXXXX") || return 1
    jq -cs '
        (map(select(.kind == "identity"))) as $ids
        | (map(select(.kind != "identity"))) as $rest
        | (if ($ids | length) == 0 then []
           else [($ids | last) + {at: (($ids | map(.at // empty) | min) // ($ids | last | .at))}] end)
          + ($rest
             | group_by([.kind, (.tracker // ""), (.issue // 0), (.commit // "")])
             | map(min_by(.at // ""))
             | sort_by(.at // ""))
        | .[]' "$older" "$newer" > "$tmp" 2>/dev/null || { rm -f "$tmp"; return 1; }
    # A claim or a filed line that one account kept after the other account's close removed it
    # would come back from the dead and cap bilan at 6 over an issue nobody holds. Only a
    # collision can do that, so only a collision asks the tracker; an unreachable tracker keeps
    # the line, which is the safe direction.
    if command -v gh >/dev/null 2>&1; then
        while IFS= read -r line; do
            t=$(printf '%s' "$line" | jq -r '.tracker // empty')
            n=$(printf '%s' "$line" | jq -r '.issue // empty')
            [ -n "$t" ] && [ -n "$n" ] || continue
            state=$(gh api "repos/$t/issues/$n" 2>/dev/null | jq -r '.state // empty' 2>/dev/null)
            [ "$state" = closed ] || continue
            jq -c --arg t "$t" --argjson n "$n" \
                'select((.kind == "claim" or .kind == "filed") and .tracker == $t and .issue == $n | not)' \
                "$tmp" > "$tmp.next" && mv "$tmp.next" "$tmp"
        done < <(jq -c 'select(.kind == "claim" or .kind == "filed")' "$tmp" 2>/dev/null)
    fi
    mv "$tmp" "$2"
}

# A baseline records what the session found when it opened, so of two the OLDER is the truth, and
# it travels with its companions (.commits .head .stack) as one set.
state_merge_baseline() { # <src-baseline> <dst-baseline>
    local ext
    if [ -e "$2" ] && [ ! "$1" -ot "$2" ]; then
        for ext in "" .commits .head .stack; do [ ! -e "$1$ext" ] || rm -f "$1$ext"; done
        return 0
    fi
    for ext in "" .commits .head .stack; do
        if [ -e "$1$ext" ]; then mv "$1$ext" "$2$ext" || return 1
        elif [ -e "$2$ext" ]; then rm -f "$2$ext"; fi
    done
}

# One call per file, so a first adoption of a few hundred sessions forks as little as it can:
# no dirname, and a mkdir only for the nested directories (pending/, warned/, cache/).
state_merge_file() { # <src> <dst> <sub/relative-path>
    case $3 in */*/*) mkdir -p "${2%/*}" || return 1 ;; esac
    case $3 in
        carnet-claims/cache/*) rm -f "$1"; return 0 ;;        # a one-minute cache: rebuilt, never merged
    esac
    [ -e "$2" ] || { mv "$1" "$2"; return; }
    case $3 in
        carnet-claims/*/*) ;;
        carnet-claims/*.jsonl) state_merge_ledger "$1" "$2"; return ;;
    esac
    case $3 in
        carnet-claims/warned/*)
            sort -u "$1" "$2" > "$2.merge" && mv "$2.merge" "$2" && rm -f "$1"; return ;;
        bilan/*.baseline.*) rm -f "$1"; return 0 ;;          # its set was settled with the baseline
    esac
    # Everything else is a latest-value file — pending list, gh-login cache, ack, score, gate state.
    if [ "$1" -nt "$2" ]; then mv "$1" "$2"; else rm -f "$1"; fi
}

state_merge_tree() { # <from-dir> <sub>
    local rel rc=0
    [ "$2" = bilan ] && while IFS= read -r rel; do
        rel=${rel#./}
        state_merge_baseline "$1/$rel" "$BILAN_DIR/$rel" || rc=1
    done < <(cd "$1" && find . -type f -name '*.baseline')
    while IFS= read -r rel; do
        rel=${rel#./}
        state_merge_file "$1/$rel" "$SESSION_STATE_HOME/$2/$rel" "$2/$rel" || rc=1
    done < <(cd "$1" && find . -type f)
    return $rc
}

# ------------------------------------------------------------------ adoption
# Turns one account's <sub> into a link to the shared one. The directory is renamed aside first,
# so a writer that lands mid-adoption either finds the link or recreates a real directory, which
# the next pass adopts in turn. A merge that fails leaves the renamed directory in place: kept
# data beats a clean tree.
state_adopt_one() { # <account-dir> <sub>
    local src="$1/$2" moved tries=0 left
    # Finish what a killed adoption started.
    for left in "$src".adopting.*; do
        [ -d "$left" ] || continue
        state_merge_tree "$left" "$2" && rm -rf "$left"
    done
    while [ "$tries" -lt 3 ]; do
        tries=$((tries + 1))
        [ -L "$src" ] && return 0
        moved=""
        if [ -d "$src" ]; then
            moved="$src.adopting.$$"
            mv "$src" "$moved" 2>/dev/null || return 1
        elif [ -e "$src" ]; then
            return 1                                     # a plain file where the dir belongs: not ours
        fi
        ln -s "$SESSION_STATE_HOME/$2" "$src" 2>/dev/null || true
        if [ -n "$moved" ]; then
            state_merge_tree "$moved" "$2" && rm -rf "$moved"
        fi
    done
    [ -L "$src" ]
}

# Called at the top of carnet.sh and bilan.sh. Returns at once on an adopted machine; otherwise
# takes the lock, merges every account's state into the shared home and leaves links behind.
state_adopt() {
    local lock="$SESSION_STATE_HOME/.adopt.lock" d sub
    command -v jq >/dev/null 2>&1 || return 0
    mkdir -p "$LEDGER_DIR" "$BILAN_DIR" 2>/dev/null || return 0
    state_pending_adoption || return 0
    # Five minutes is longer than any adoption takes. A holder still busy past the wait is another
    # process adopting; it will finish.
    state_lock "$lock" 5 || return 0
    while IFS= read -r d; do
        for sub in $STATE_SUBDIRS; do
            state_adopt_one "$d" "$sub" || true
        done
    done < <(state_account_dirs)
    state_unlock "$lock"
    return 0
}
