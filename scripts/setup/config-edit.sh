#!/usr/bin/env bash
# ABOUTME: Reversible edits to a developer's config files: one backup, a ledger of changes, atomic writes
# ABOUTME: An undo restores the backup only when the file is still what the last edit left, never over later hand edits
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

# Sourced by the setup scripts. A setup script that edits `.envrc` used to
# rewrite it in place with no copy and no record, so a bad run could not be
# taken back and a good one could not be told apart from a hand edit.
#
#   cfg_set_export FILE KEY VALUE   set `export KEY="VALUE"` (replace the line, or append it)
#   cfg_record NOTE                 add a line to the ledger (an external change, e.g. an MCP entry)
#   cfg_undo FILE                   restore FILE's backup if FILE is unchanged since the last edit
#
# State lives in CFG_STATE_DIR (default: <dir of FILE>/.setup-state), created
# 0700; the backup is taken once, before the first edit, at 0600.

CFG_STATE_DIR="${CFG_STATE_DIR:-}"

cfg__state_dir() {
    local file="$1"
    local dir="${CFG_STATE_DIR:-$(dirname "$file")/.setup-state}"
    if [ ! -d "$dir" ]; then
        (umask 077 && mkdir -p "$dir")
    fi
    printf '%s' "$dir"
}

cfg__sha() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        sha256sum "$1" | cut -d' ' -f1
    fi
}

cfg__ledger() {
    local dir="$1"
    shift
    printf '%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >>"$dir/ledger.tsv"
}

# Take the one backup of FILE, before anything here ever changed it.
cfg__backup_once() {
    local file="$1" dir
    dir="$(cfg__state_dir "$file")"
    local backup
    backup="$dir/$(basename "$file").orig"
    if [ ! -e "$backup" ]; then
        (umask 077 && cp -p "$file" "$backup")
        chmod 600 "$backup"
        cfg__ledger "$dir" "backup $(basename "$file") sha=$(cfg__sha "$file")"
    fi
}

cfg_set_export() {
    local file="$1" key="$2" value="$3" dir
    [ -f "$file" ] || { echo "cfg_set_export: $file does not exist" >&2; return 1; }
    case "$key" in
        *[!A-Za-z0-9_]* | '') echo "cfg_set_export: bad key '$key'" >&2; return 1 ;;
    esac
    dir="$(cfg__state_dir "$file")"
    cfg__backup_once "$file"

    local staging
    staging="$(mktemp "$dir/.edit.XXXXXX")"
    chmod 600 "$staging"
    # awk, not sed: the value is data, never a pattern or a replacement string.
    KEY="$key" VALUE="$value" awk '
        BEGIN { line = "export " ENVIRON["KEY"] "=\"" ENVIRON["VALUE"] "\""; done = 0 }
        index($0, "export " ENVIRON["KEY"] "=") == 1 { if (!done) { print line; done = 1 }; next }
        { print }
        END { if (!done) { print ""; print line } }
    ' "$file" >"$staging" || { rm -f "$staging"; return 1; }
    chmod "$(stat -c %a "$file" 2>/dev/null || stat -f %Lp "$file")" "$staging"
    mv -f "$staging" "$file"
    cfg__ledger "$dir" "set $key in $(basename "$file") sha=$(cfg__sha "$file")"
}

cfg_record() {
    local file="$1"
    shift
    cfg__ledger "$(cfg__state_dir "$file")" "$*"
}

# Restore FILE's backup, but only if FILE is exactly what the last edit here
# left: a file someone changed by hand since is theirs, and is not overwritten.
cfg_undo() {
    local file="$1" dir backup last
    dir="$(cfg__state_dir "$file")"
    backup="$dir/$(basename "$file").orig"
    if [ ! -f "$backup" ]; then
        echo "cfg_undo: no backup of $(basename "$file"); nothing to undo" >&2
        return 1
    fi
    last="$(awk -F'\t' -v name="$(basename "$file")" \
        'index($2, "set ") == 1 && index($2, " in " name " sha=") > 0 { s = $2 } END { sub(/.*sha=/, "", s); print s }' \
        "$dir/ledger.tsv")"
    if [ -n "$last" ] && [ "$(cfg__sha "$file")" != "$last" ]; then
        echo "cfg_undo: $(basename "$file") changed since the last setup edit; leaving it as it is" >&2
        echo "          (backup: $backup)" >&2
        return 2
    fi
    cp -p "$backup" "$file"
    cfg__ledger "$dir" "restore $(basename "$file") sha=$(cfg__sha "$file")"
}
