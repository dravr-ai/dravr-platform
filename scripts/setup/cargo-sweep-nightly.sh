#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Reclaims Cargo build output across every repository under a scan root, on a schedule.
# ABOUTME: Runs cargo-sweep per repo, then enforces a fleet-wide size ceiling by escalating on idle repos.
#
# macOS only: it uses BSD `stat -f`, BSD `du`, and `launchctl` for the schedule.
# Every repository keeps its own plain target/ directory and disk is reclaimed on
# a schedule. A repository can also hold alternate build trees, from a side build
# pointed at another CARGO_TARGET_DIR; those are reclaimed too, and carry a
# repo[dir] label so two trees in one repository stay separate entries.
# `sweep` is the routine age-based pass, `purge` is the aggressive escape hatch,
# and both end by enforcing the fleet cap.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT_PATH="$SCRIPT_DIR/$(basename "${BASH_SOURCE[0]}")"

SCAN_ROOT="${CARGO_SWEEP_SCAN_ROOT:-$HOME/workspace}"
CAP_SPEC="${CARGO_SWEEP_CAP:-400GiB}"
DAYS="${CARGO_SWEEP_DAYS:-30}"
IDLE_DAYS="${CARGO_SWEEP_IDLE_DAYS:-7}"
KEEP="${CARGO_SWEEP_KEEP:-1}"
MAX_DEPTH="${CARGO_SWEEP_MAX_DEPTH:-3}"

NO_CAP=0
FORCE=0
DRY_RUN=0
COMMAND=""

AGENT_LABEL="ai.dravr.cargo-sweep"
AGENT_DEST="$HOME/Library/LaunchAgents/$AGENT_LABEL.plist"
PLIST_TEMPLATE="$SCRIPT_DIR/$AGENT_LABEL.plist.template"
LOG_FILE="$HOME/Library/Logs/cargo-sweep.log"
LOG_MAX_BYTES=5242880
LOG_KEEP_LINES=500

if [[ -t 1 ]]; then
    RED=$'\033[0;31m'
    GREEN=$'\033[0;32m'
    YELLOW=$'\033[0;33m'
    BLUE=$'\033[0;34m'
    DIM=$'\033[2m'
    NC=$'\033[0m'
else
    RED="" GREEN="" YELLOW="" BLUE="" DIM="" NC=""
fi

usage() {
    cat <<'EOF'
Usage: cargo-sweep-nightly.sh <command> [options]

Reclaims Cargo build output across every repository under a scan root and keeps
the combined size of every build tree under a hard ceiling. A tree is any
target/ or target-*/ directory that carries cargo's CACHEDIR.TAG and sits beside
a Cargo.toml, so a side build's CARGO_TARGET_DIR is swept and billed like the rest.

Commands:
  sweep       Age-based cargo-sweep pass across the fleet, then enforce the cap (default)
  purge       Aggressive immediate reclaim (incremental caches, idle repos), then enforce the cap
  status      Per-repo sizes, fleet total, headroom against the cap; read-only
  install     Render and bootstrap the ai.dravr.cargo-sweep LaunchAgent
  uninstall   Boot out and remove the LaunchAgent

Options:
  --root PATH      Scan root                        (env CARGO_SWEEP_SCAN_ROOT, default ~/workspace)
  --cap SIZE       Fleet ceiling, e.g. 400GiB       (env CARGO_SWEEP_CAP,       default 400GiB)
  --days N         Sweep age threshold in days      (env CARGO_SWEEP_DAYS,      default 30)
  --idle-days N    Purge: a repo counts as idle     (env CARGO_SWEEP_IDLE_DAYS, default 7)
  --keep N         Never wholesale-drop the N most-recently-built repos
                                                    (env CARGO_SWEEP_KEEP,      default 1)
  --max-depth N    Discovery depth under the root   (env CARGO_SWEEP_MAX_DEPTH, default 3)
  --no-cap         Run the sweep or purge, skip cap enforcement
  --force          Proceed on repos whose cargo build lock is held
  --dry-run        Print every action, change nothing
  -h, --help       Show this help

Sizes accept KiB/MiB/GiB/TiB and KB/MB/GB/TB; a bare number means MiB.

Exit codes:
  0    ran; fleet is under the cap (or --no-cap)
  1    cap could not be reached; the message names the repos that blocked it
  2    usage error
  127  cargo-sweep is not installed

The cap is enforced, not warned about: when the routine pass leaves the fleet
over the ceiling, reclaim escalates from the least-recently-built repositories
first, so the repo being worked on keeps its warm cache longest.
EOF
}

usage_error() {
    echo "${RED}$1${NC}" >&2
    usage >&2
    exit 2
}

need_value() {
    [[ $2 -ge 2 ]] || usage_error "$1 requires a value"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        sweep|purge|status|install|uninstall)
            [[ -z "$COMMAND" ]] || usage_error "unexpected argument: $1"
            COMMAND="$1"; shift ;;
        --root) need_value "$1" $#; SCAN_ROOT="$2"; shift 2 ;;
        --cap) need_value "$1" $#; CAP_SPEC="$2"; shift 2 ;;
        --days) need_value "$1" $#; DAYS="$2"; shift 2 ;;
        --idle-days) need_value "$1" $#; IDLE_DAYS="$2"; shift 2 ;;
        --keep) need_value "$1" $#; KEEP="$2"; shift 2 ;;
        --max-depth) need_value "$1" $#; MAX_DEPTH="$2"; shift 2 ;;
        --no-cap) NO_CAP=1; shift ;;
        --force) FORCE=1; shift ;;
        --dry-run) DRY_RUN=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage_error "unknown argument: $1" ;;
    esac
done

COMMAND="${COMMAND:-sweep}"

for pair in "days:$DAYS" "idle-days:$IDLE_DAYS" "keep:$KEEP" "max-depth:$MAX_DEPTH"; do
    name="${pair%%:*}"
    value="${pair#*:}"
    [[ "$value" =~ ^[0-9]+$ ]] || usage_error "--$name expects a non-negative integer, got: $value"
done
[[ "$MAX_DEPTH" -ge 1 ]] || usage_error "--max-depth must be at least 1"

# Size string -> bytes. A bare number means MiB, matching what cargo-sweep's
# parser actually does with --maxsize even though its help text says MB.
parse_size() {
    local raw="$1" num unit mult
    num=$(printf '%s' "$raw" | sed -n 's/^\([0-9][0-9]*\)[A-Za-z]*$/\1/p')
    unit=$(printf '%s' "$raw" | sed -n 's/^[0-9][0-9]*\([A-Za-z]*\)$/\1/p' | tr '[:lower:]' '[:upper:]')
    [[ -n "$num" ]] || return 1
    case "$unit" in
        "") mult=1048576 ;;
        KIB) mult=1024 ;;
        MIB) mult=1048576 ;;
        GIB) mult=1073741824 ;;
        TIB) mult=1099511627776 ;;
        KB) mult=1000 ;;
        MB) mult=1000000 ;;
        GB) mult=1000000000 ;;
        TB) mult=1000000000000 ;;
        *) return 1 ;;
    esac
    echo $((num * mult))
}

CAP_BYTES=$(parse_size "$CAP_SPEC") || usage_error \
    "unparseable size: $CAP_SPEC — accepted forms: 400GiB, 400GB, 512000MiB, 400 (bare = MiB)"
CAP_KIB=$((CAP_BYTES / 1024))
[[ "$CAP_KIB" -gt 0 ]] || usage_error "--cap must be greater than zero, got: $CAP_SPEC"

# A scan root that is missing, unreadable, or not a directory finds no target
# dirs and would otherwise report a healthy empty fleet and exit 0 — a nightly
# job announcing success while sweeping nothing. Refuse instead.
[[ -e "$SCAN_ROOT" ]] || usage_error "scan root does not exist: $SCAN_ROOT"
[[ -d "$SCAN_ROOT" ]] || usage_error "scan root is not a directory: $SCAN_ROOT"
[[ -r "$SCAN_ROOT" && -x "$SCAN_ROOT" ]] || usage_error "scan root is not readable: $SCAN_ROOT"

# Every line a dry run prints is prefixed, so no output can be mistaken for
# something that happened.
PREFIX=""
[[ $DRY_RUN -eq 1 ]] && PREFIX="would "

STAGE="$SCAN_ROOT/.cargo-reclaim"
RUN_LOCK="${TMPDIR:-/tmp}/cargo-sweep-nightly.lock"
LOCK_HELD=0

WORK="$(mktemp -d "${TMPDIR:-/tmp}/cargo-sweep-nightly.XXXXXX")"
TARGETS="$WORK/targets.tsv"
ORDER="$WORK/order.tsv"
SIZES="$WORK/sizes.tsv"
ROWS="$WORK/rows.tsv"
CAP_ROWS="$WORK/cap-rows.tsv"
BLOCKED="$WORK/blocked.txt"
SKIPPED_LOCKED="$WORK/skipped-locked.txt"
: > "$SIZES"
: > "$ROWS"
: > "$CAP_ROWS"
: > "$BLOCKED"
: > "$SKIPPED_LOCKED"

cleanup() {
    rm -rf "$WORK" 2>/dev/null || true
    if [[ $LOCK_HELD -eq 1 ]]; then
        rmdir "$RUN_LOCK" 2>/dev/null || true
    fi
    return 0
}
trap cleanup EXIT

PYTHON_BIN="$(command -v python3 2>/dev/null || true)"
[[ -x "$PYTHON_BIN" ]] || PYTHON_BIN=""

NOW=$(date +%s)

# --- size and time helpers -------------------------------------------------

# Size of a path in KiB, tolerant of a concurrent writer. du exits non-zero when
# a file vanishes mid-scan, which happens constantly against a target dir a build
# or a background rm is touching; under pipefail that would abort the script. The
# `|| true` neutralizes du's exit code inside the braces while its stdout still
# flows to awk, so the partial sum survives instead of being discarded.
size_kib() {
    local path="$1" out
    [[ -e "$path" ]] || { echo 0; return 0; }
    out=$( { du -sk "$path" 2>/dev/null || true; } | awk 'NR==1{print $1}' )
    [[ "$out" =~ ^[0-9]+$ ]] || out=0
    echo "$out"
}

# Human-readable size for display only. "-" when absent, "?" when du said nothing.
human_size() {
    local path="$1" out
    [[ -e "$path" ]] || { echo "-"; return 0; }
    out=$( { du -sh "$path" 2>/dev/null || true; } | awk 'NR==1{print $1}' )
    echo "${out:-?}"
}

# One decimal, GiB at fleet scale, and an honest unit when a number is small
# enough that "0.0 GiB" would say nothing.
fmt_kib() {
    awk -v k="$1" 'BEGIN {
        if (k >= 1048576) printf "%.1f GiB", k / 1048576;
        else if (k >= 1024) printf "%.1f MiB", k / 1024;
        else printf "%d KiB", k;
    }'
}

pct_of_cap() {
    awk -v a="$1" -v b="$2" 'BEGIN { printf "%d", (b > 0 ? a * 100 / b : 0) }'
}

# Keep a long worktree name from pushing the columns out of alignment. Sibling
# worktrees of one repo share a long prefix and differ only in their suffix, so
# the middle is what gets dropped — trimming the tail would print two different
# repositories under the same name.
trunc() {
    local text="$1" width="$2" head tail
    if [[ ${#text} -le $width ]]; then
        printf '%s' "$text"
        return 0
    fi
    if [[ $width -lt 16 ]]; then
        printf '%s...' "${text:0:$((width - 3))}"
        return 0
    fi
    head=$((width - 13))
    tail=$((${#text} - 10))
    printf '%s...%s' "${text:0:$head}" "${text:$tail}"
}

# Newest mtime among the sentinels cargo rewrites on every build. Never the
# directory's own mtime: a directory mtime only moves when entries are added or
# removed at that exact level, and any mv preserves it.
target_mtime_epoch() {
    local target="$1" newest
    [[ -d "$target" ]] || { echo 0; return 0; }
    newest=$(find "$target" -maxdepth 2 \
        \( -name '.cargo-lock' -o -name '.rustc_info.json' -o -name '.fingerprint' \) \
        -exec stat -f '%m' {} + 2>/dev/null | sort -rn | head -1)
    [[ "$newest" =~ ^[0-9]+$ ]] || newest=0
    echo "$newest"
}

age_human() {
    local epoch="$1" delta
    [[ "$epoch" -gt 0 ]] || { echo "never"; return 0; }
    delta=$((NOW - epoch))
    if [[ $delta -lt 3600 ]]; then
        echo "$((delta / 60))m ago"
    elif [[ $delta -lt 86400 ]]; then
        echo "$((delta / 3600))h ago"
    else
        echo "$((delta / 86400))d ago"
    fi
}

# Idleness is asked of a build timestamp, never of a live re-stat, because
# cargo-sweep rewrites .fingerprint as it deletes: a repo idle for months looks
# freshly built the instant the shrink pass touches it. Callers capture the
# epoch before they start reclaiming and pass that same value to every later
# question about the repo.
is_idle_epoch() {
    [[ $((NOW - $1)) -gt $((IDLE_DAYS * 86400)) ]]
}

is_idle() {
    is_idle_epoch "$(target_mtime_epoch "$1")"
}

# Reason text for a drop that passed the idle test.
idle_reason() {
    if [[ "$1" -le 0 ]]; then
        echo "wholesale, never built"
    else
        echo "wholesale, idle $(( (NOW - $1) / 86400 ))d"
    fi
}

# Reason text for the cap's last resort, which drops a tree on age order alone
# and so must not claim the repo cleared the idle threshold.
recency_reason() {
    if [[ "$1" -le 0 ]]; then
        echo "wholesale, never built"
    else
        echo "wholesale, last built $(age_human "$1")"
    fi
}

# --- build lock ------------------------------------------------------------

# True when a cargo build currently holds a lock inside the given target dir.
# Cargo creates .cargo-lock / .cargo-build-lock / .cargo-artifact-lock per profile
# and holds a flock(2) on them for the duration of a build, so probing that same
# lock non-blockingly is an exact answer rather than a heuristic. lsof can answer
# this too, but it enumerates every open file on the system and stalls for minutes
# while the disk is busy — which is precisely when this gets called.
build_locked() {
    local target="$1" locks
    [[ -d "$target" ]] || return 1
    locks=$(find "$target" -maxdepth 2 -name '.cargo-*lock' -type f 2>/dev/null || true)
    [[ -n "$locks" ]] || return 1

    if [[ -z "$PYTHON_BIN" ]]; then
        # No flock probe available: treat a target touched in the last two
        # minutes as owned by a running build.
        [[ -n "$(find "$target" -maxdepth 2 -mmin -2 -print -quit 2>/dev/null || true)" ]]
        return
    fi

    printf '%s\n' "$locks" | "$PYTHON_BIN" -c '
import fcntl, sys
for path in sys.stdin.read().splitlines():
    path = path.strip()
    if not path:
        continue
    try:
        with open(path, "r") as handle:
            fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        sys.exit(0)   # someone holds it: a build is running
sys.exit(1)           # every lock was free
'
}

locked_skip() {
    local label="$1"
    echo "${YELLOW}skip    $label: a cargo build holds the target lock (--force overrides)${NC}"
    grep -qxF "$label" "$SKIPPED_LOCKED" 2>/dev/null || echo "$label" >> "$SKIPPED_LOCKED"
}

# --- discovery -------------------------------------------------------------

# A repository can hold more than one build tree: `target/` plus whatever a side
# build pointed CARGO_TARGET_DIR at. The label keys the protected list, the cap
# ledger and the staging path, so each tree needs its own or two trees in one
# repo would protect, bill and stage as a single entry.
target_label() {
    local root="$1" target="$2" dir
    dir=$(basename "$target")
    if [[ "$dir" == "target" ]]; then
        basename "$root"
    else
        printf '%s[%s]\n' "$(basename "$root")" "$dir"
    fi
}

discover_targets() {
    local target root label
    while IFS= read -r target; do
        [[ -n "$target" ]] || continue
        [[ -e "$target/CACHEDIR.TAG" || -e "$target/.rustc_info.json" ]] || continue
        root=$(dirname "$target")
        [[ -f "$root/Cargo.toml" ]] || continue
        label=$(target_label "$root" "$target")
        printf '%s\t%s\t%s\n' "$root" "$target" "$label"
    done < <(find "$SCAN_ROOT" -maxdepth "$MAX_DEPTH" \
                  \( -name node_modules -o -name .git -o -path "$STAGE" \) -prune -o \
                  -type d -name 'target*' -prune -print 2>/dev/null || true)
}

# A build tree that is a symlink is the one shape this tool refuses to work with:
# per-worktree isolation expects a real directory, and following the link would
# reclaim a tree several repositories share. Report it and move on.
report_symlinked_targets() {
    local link
    while IFS= read -r link; do
        [[ -n "$link" ]] || continue
        echo "${YELLOW}symlink  $(basename "$(dirname "$link")"): $(basename "$link")/ is a symlink -> $(readlink "$link"); per-worktree isolation expects a real directory${NC}"
    done < <(find "$SCAN_ROOT" -maxdepth "$MAX_DEPTH" -name 'target*' -type l 2>/dev/null || true)
}

# Build output whose project root is gone, left behind when a worktree was
# deleted around it. cargo-sweep cannot touch it — it resolves the target dir
# through cargo metadata, and there is no manifest to read — so it stays out of
# the swept set and out of the fleet total, and gets named instead of hidden.
report_orphan_targets() {
    local target root
    while IFS= read -r target; do
        [[ -n "$target" ]] || continue
        [[ -e "$target/CACHEDIR.TAG" || -e "$target/.rustc_info.json" ]] || continue
        root=$(dirname "$target")
        [[ -f "$root/Cargo.toml" ]] && continue
        echo "${YELLOW}orphan   $(basename "$root"): $(human_size "$target") of build output with no Cargo.toml beside it; delete the directory by hand${NC}"
    done < <(find "$SCAN_ROOT" -maxdepth "$MAX_DEPTH" \
                  \( -name node_modules -o -name .git -o -path "$STAGE" \) -prune -o \
                  -type d -name 'target*' -prune -print 2>/dev/null || true)
}

# Least-recently-built first, captured once before the run reclaims anything.
# Every question about build age is answered from this snapshot: cargo-sweep
# rewrites .fingerprint as it deletes, so re-stat'ing mid-run would report the
# repos the sweep just cleaned as the freshest ones on the machine.
snapshot_order() {
    local root target label epoch
    while IFS=$'\t' read -r root target label; do
        [[ -n "$target" ]] || continue
        epoch=$(target_mtime_epoch "$target")
        printf '%s\t%s\t%s\t%s\n' "$epoch" "$root" "$target" "$label"
    done < "$TARGETS" | sort -t"$(printf '\t')" -k1,1n > "$ORDER"
}

# --- accounting ------------------------------------------------------------

# Current size of a target dir. A dry run reads the simulated ledger so the whole
# escalation ladder can run without touching the disk; a real run measures.
current_kib() {
    local target="$1" value
    if [[ $DRY_RUN -eq 1 ]]; then
        value=$(awk -F'\t' -v k="$target" '$1 == k { v = $2 } END { if (v == "") print -1; else print v }' "$SIZES")
        if [[ "$value" == "-1" ]]; then
            value=$(size_kib "$target")
            printf '%s\t%s\n' "$target" "$value" >> "$SIZES"
        fi
        echo "$value"
    else
        size_kib "$target"
    fi
}

set_kib() {
    [[ $DRY_RUN -eq 1 ]] && printf '%s\t%s\n' "$1" "$2" >> "$SIZES"
    return 0
}

fleet_total_kib() {
    local root target label total=0 k
    while IFS=$'\t' read -r root target label; do
        [[ -n "$target" ]] || continue
        k=$(current_kib "$target")
        total=$((total + k))
    done < "$TARGETS"
    echo "$total"
}

record_row() {
    printf '%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$5" >> "$ROWS"
}

record_cap_row() {
    printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$CAP_ROWS"
}

record_blocked() {
    grep -qxF "$1" "$BLOCKED" 2>/dev/null || echo "$1" >> "$BLOCKED"
}

# --- reclaim primitives ----------------------------------------------------

ensure_stage() {
    mkdir -p "$STAGE"
    # Keep Spotlight from indexing hundreds of thousands of artifacts on their
    # way to deletion.
    [[ -e "$STAGE/.metadata_never_index" ]] || : > "$STAGE/.metadata_never_index"
}

# Rename the tree aside, then delete it in the background: the rename is O(1) on
# the same APFS volume so the repo is usable again immediately, where removing
# tens of GB of small files takes minutes. Staging sits outside every repo, so it
# never shows up in anyone's git status. cargo recreates target/ on the next build.
wholesale_reclaim() {
    local target="$1" label="$2" staged
    ensure_stage
    staged="$STAGE/$label.$$"
    mv "$target" "$staged"
    ( rm -rf "$staged" >/dev/null 2>&1 & )
}

# --- cargo-sweep glue ------------------------------------------------------

require_cargo_sweep() {
    command -v cargo-sweep >/dev/null 2>&1 && return 0
    {
        echo "cargo-sweep is not installed — refusing to run."
        echo "  Install: cargo install cargo-sweep"
        echo "  Verify:  cargo sweep --version    (expect cargo-sweep-sweep 0.8.0 or newer)"
    } >&2
    exit 127
}

# Sum the "Cleaned:" / "Would clean:" amounts cargo-sweep reports, in KiB.
parse_clean_kib() {
    awk '
    BEGIN { total = 0 }
    {
        low = tolower($0)
        pos = index(low, "cleaned: ")
        if (pos > 0) {
            rest = substr($0, pos + 9)
        } else {
            pos = index(low, "clean: ")
            if (pos == 0) next
            rest = substr($0, pos + 7)
        }
        n = rest + 0
        if (n <= 0) next
        unit = rest
        sub(/^[0-9.]+[ ]*/, "", unit)
        unit = toupper(substr(unit, 1, 3))
        if (unit ~ /^KIB/) mult = 1
        else if (unit ~ /^MIB/) mult = 1024
        else if (unit ~ /^GIB/) mult = 1048576
        else if (unit ~ /^TIB/) mult = 1073741824
        else if (unit ~ /^KB/) mult = 1000 / 1024
        else if (unit ~ /^MB/) mult = 1000000 / 1024
        else if (unit ~ /^GB/) mult = 1000000000 / 1024
        else if (unit ~ /^TB/) mult = 1000000000000 / 1024
        else if (unit ~ /^B/) mult = 1 / 1024
        else mult = 1024
        total += n * mult
    }
    END { printf "%d", total }
    '
}

# One repo per invocation, never a batch of paths: in non-recursive mode
# cargo-sweep does metadata(path).context(...)?, so a single unparseable
# Cargo.toml aborts the entire invocation and every remaining repo silently goes
# unswept.
SWEEP_OUTPUT=""
# cargo-sweep resolves the tree itself via cargo metadata --no-deps, which finds
# `target/` and nothing else. Naming the discovered tree through CARGO_TARGET_DIR
# is what lets a side build's tree be swept at all: without it cargo-sweep warns
# that the default path does not exist, and the alternate tree would be reported
# as swept while keeping every byte.
run_cargo_sweep() {
    local root="$1" target="$2"
    shift 2
    local rc=0
    if [[ $DRY_RUN -eq 1 ]]; then
        SWEEP_OUTPUT=$(CARGO_TARGET_DIR="$target" cargo sweep --dry-run "$@" "$root" 2>&1) || rc=$?
    else
        SWEEP_OUTPUT=$(CARGO_TARGET_DIR="$target" cargo sweep "$@" "$root" 2>&1) || rc=$?
    fi
    if [[ $rc -ne 0 ]]; then
        echo "${RED}        cargo sweep exited $rc${NC}"
    fi
    printf '%s\n' "$SWEEP_OUTPUT" | grep -Ei 'clean' | sed 's/^/        /' || true
    return 0
}

# --- prelude ---------------------------------------------------------------

acquire_run_lock() {
    if [[ $DRY_RUN -eq 1 ]]; then
        if [[ -d "$RUN_LOCK" ]]; then
            echo "another run is in progress — yielding"
            exit 0
        fi
        return 0
    fi
    if ! mkdir "$RUN_LOCK" 2>/dev/null; then
        echo "another run is in progress — yielding"
        exit 0
    fi
    LOCK_HELD=1
}

# A disk tool must not leak disk. Truncate in place rather than rotating by
# rename: launchd holds the log's file descriptor open for the life of the job.
log_guard() {
    local bytes tmp
    [[ -f "$LOG_FILE" ]] || return 0
    bytes=$(stat -f '%z' "$LOG_FILE" 2>/dev/null || echo 0)
    [[ "$bytes" -gt "$LOG_MAX_BYTES" ]] || return 0
    if [[ $DRY_RUN -eq 1 ]]; then
        echo "${BLUE}would   truncate $LOG_FILE to its last $LOG_KEEP_LINES lines ($bytes bytes)${NC}"
        return 0
    fi
    tmp="$WORK/log.tail"
    tail -n "$LOG_KEEP_LINES" "$LOG_FILE" > "$tmp" 2>/dev/null || true
    cat "$tmp" > "$LOG_FILE"
    rm -f "$tmp"
}

drain_stage() {
    [[ -d "$STAGE" ]] || return 0
    local leftovers
    leftovers=$(find "$STAGE" -mindepth 1 -maxdepth 1 -not -name '.metadata_never_index' -print -quit 2>/dev/null || true)
    [[ -n "$leftovers" ]] || return 0
    if [[ $DRY_RUN -eq 1 ]]; then
        echo "${BLUE}would   drain leftover staging at $STAGE ($(human_size "$STAGE"))${NC}"
        return 0
    fi
    echo "${DIM}drain   leftover staging at $STAGE ($(human_size "$STAGE")) deleting in background${NC}"
    ( rm -rf "${STAGE:?}"/* >/dev/null 2>&1 & )
}

staging_note() {
    [[ -d "$STAGE" ]] || return 0
    local leftovers
    leftovers=$(find "$STAGE" -mindepth 1 -maxdepth 1 -not -name '.metadata_never_index' -print -quit 2>/dev/null || true)
    [[ -n "$leftovers" ]] || return 0
    echo "staging: $(human_size "$STAGE") still deleting in background"
}

prelude() {
    acquire_run_lock
    log_guard
    require_cargo_sweep
    drain_stage
    discover_targets > "$TARGETS"
    snapshot_order
    report_symlinked_targets
    report_orphan_targets
    if [[ ! -s "$TARGETS" ]]; then
        echo "No Cargo target directories found under $SCAN_ROOT (depth $MAX_DEPTH)."
    fi
}

# --- cap enforcement -------------------------------------------------------

# Labels of the KEEP most-recently-built repos, which stage 2 and purge never
# drop wholesale. The snapshot sorts ascending, so its tail is the newest.
protected_labels() {
    [[ "$KEEP" -gt 0 ]] || return 0
    tail -n "$KEEP" "$ORDER" | cut -f4
}

is_protected() {
    local label="$1" protected="$2"
    [[ -n "$protected" ]] || return 1
    printf '%s\n' "$protected" | grep -qxF "$label"
}

# One line per repository, biggest reclaim first. A repo that both shrank and
# then gave up its whole tree is a single entry carrying both reasons.
report_cap_reclaim() {
    [[ -s "$CAP_ROWS" ]] || return 0
    local agg="$WORK/cap-agg.tsv" count freed_total label freed reason
    awk -F'\t' '
        {
            freed[$1] += $2
            if (reason[$1] == "") reason[$1] = $3
            else if (index(reason[$1], $3) == 0) reason[$1] = reason[$1] " + " $3
        }
        END { for (k in freed) printf "%s\t%s\t%s\n", freed[k], k, reason[k] }
    ' "$CAP_ROWS" | sort -t"$(printf '\t')" -k1,1nr > "$agg"
    count=$(wc -l < "$agg" | tr -d ' ')
    freed_total=$(awk -F'\t' '{ s += $1 } END { printf "%d", s }' "$agg")
    if [[ $DRY_RUN -eq 1 ]]; then
        echo "cap: escalation would reclaim $(fmt_kib "$freed_total") from $count repo(s):"
    else
        echo "cap: escalation reclaimed $(fmt_kib "$freed_total") from $count repo(s):"
    fi
    while IFS=$'\t' read -r freed label reason; do
        [[ -n "$label" ]] || continue
        printf '       %-28s %10s   (%s)\n' "$(trunc "$label" 28)" "$(fmt_kib "$freed")" "$reason"
    done < "$agg"
}

enforce_cap() {
    if [[ $NO_CAP -eq 1 ]]; then
        echo "cap: enforcement skipped (--no-cap); ceiling would be $(fmt_kib "$CAP_KIB")"
        return 0
    fi

    local total before_total
    total=$(fleet_total_kib)
    before_total=$total

    if [[ "$total" -le "$CAP_KIB" ]]; then
        echo "cap: $(fmt_kib "$CAP_KIB") — fleet at $(fmt_kib "$total"), headroom $(fmt_kib "$((CAP_KIB - total))")"
        return 0
    fi

    echo "cap: fleet at $(fmt_kib "$total"), over the $(fmt_kib "$CAP_KIB") cap by $(fmt_kib "$((total - CAP_KIB))") — escalating"

    local protected
    protected=$(protected_labels)

    local epoch root target label cur surplus want want_mib freed after reason

    # Stage 1 — shrink to fit, least-recently-built first. cargo-sweep decides
    # WHICH artifacts die (oldest fingerprint-tracked first), which is exactly the
    # right eviction order. It cannot be combined with --time in one call: the
    # criterion argument group is required-and-exclusive, so the cap is a second pass.
    while IFS=$'\t' read -r epoch root target label; do
        [[ "$total" -gt "$CAP_KIB" ]] || break
        [[ -n "$target" ]] || continue
        if [[ $FORCE -eq 0 ]] && build_locked "$target"; then
            echo "cap: skipped $label (build in progress)"
            record_blocked "$label"
            continue
        fi
        cur=$(current_kib "$target")
        [[ "$cur" -gt 0 ]] || continue
        surplus=$((total - CAP_KIB))
        want=$((cur - surplus))
        [[ "$want" -lt 0 ]] && want=0
        want_mib=$((want / 1024))
        echo "cap: ${PREFIX}shrink $label to $(fmt_kib "$want")"
        run_cargo_sweep "$root" "$target" --maxsize "${want_mib}MiB"
        if [[ $DRY_RUN -eq 1 ]]; then
            freed=$(printf '%s\n' "$SWEEP_OUTPUT" | parse_clean_kib)
            [[ "$freed" -gt "$cur" ]] && freed=$cur
        else
            freed=$((cur - $(size_kib "$target")))
            [[ "$freed" -lt 0 ]] && freed=0
        fi
        after=$((cur - freed))
        set_kib "$target" "$after"
        total=$((total - freed))
        reason="shrink to fit"

        # --maxsize only removes fingerprint-tracked artifacts, so each tree has a
        # floor it cannot go below. An idle repo that is still over its number
        # gives up the whole tree rather than pretending the pass succeeded.
        if [[ "$after" -gt "$want" ]] && is_idle_epoch "$epoch"; then
            if [[ $DRY_RUN -eq 1 ]]; then
                echo "cap: would reclaim $label ($(fmt_kib "$after")) — $(idle_reason "$epoch")"
            else
                echo "cap: reclaim $label ($(fmt_kib "$after")) — $(idle_reason "$epoch")"
                wholesale_reclaim "$target" "$label"
            fi
            reason="shrink to fit + $(idle_reason "$epoch")"
            freed=$((freed + after))
            total=$((total - after))
            set_kib "$target" 0
        fi
        [[ "$freed" -gt 0 ]] && record_cap_row "$label" "$freed" "$reason"
    done < "$ORDER"

    # Stage 2 — wholesale, same order, only if stage 1 could not get there.
    # Dropping everything but the newest repo bounds the fleet at one repo's tree,
    # so any sane cap is reachable unless held build locks blocked it.
    while IFS=$'\t' read -r epoch root target label; do
        [[ "$total" -gt "$CAP_KIB" ]] || break
        [[ -n "$target" ]] || continue
        if is_protected "$label" "$protected"; then
            echo "cap: keeping $label (one of the $KEEP most-recently-built)"
            continue
        fi
        if [[ $FORCE -eq 0 ]] && build_locked "$target"; then
            echo "cap: skipped $label (build in progress)"
            record_blocked "$label"
            continue
        fi
        cur=$(current_kib "$target")
        [[ "$cur" -gt 0 ]] || continue
        reason=$(recency_reason "$epoch")
        if [[ $DRY_RUN -eq 1 ]]; then
            echo "cap: would reclaim $label ($(fmt_kib "$cur")) — $reason"
        else
            echo "cap: reclaim $label ($(fmt_kib "$cur")) — $reason"
            wholesale_reclaim "$target" "$label"
        fi
        set_kib "$target" 0
        total=$((total - cur))
        record_cap_row "$label" "$cur" "$reason"
    done < "$ORDER"

    # One authoritative pass produces the reported "after" and the verdict. The
    # loop's arithmetic drives decisions; it never becomes the number reported.
    local measured
    measured=$(fleet_total_kib)

    report_cap_reclaim

    if [[ "$measured" -le "$CAP_KIB" ]]; then
        if [[ $DRY_RUN -eq 1 ]]; then
            echo "cap: fleet would go $(fmt_kib "$before_total") -> $(fmt_kib "$measured"), under the $(fmt_kib "$CAP_KIB") cap"
        else
            echo "cap: fleet $(fmt_kib "$before_total") -> $(fmt_kib "$measured"), under the $(fmt_kib "$CAP_KIB") cap"
        fi
        return 0
    fi

    echo "${RED}cap: STILL OVER — fleet at $(fmt_kib "$measured") vs a $(fmt_kib "$CAP_KIB") cap.${NC}"
    if [[ -s "$BLOCKED" ]]; then
        echo "     Blocked by held build locks: $(paste -sd, - < "$BLOCKED" | sed 's/,/, /g')."
        echo "     Re-run after the builds finish, or pass --force."
    else
        echo "     Every unprotected repository was already reclaimed; lower --keep or raise --cap."
    fi
    return 1
}

# --- report ----------------------------------------------------------------

print_footer() {
    local phase="$1" before="$2" after="$3" delta freed
    delta=$((before - after))
    # A build running in another worktree can add more than this phase removed,
    # which leaves the fleet larger than it started. Report what the phase itself
    # freed — the sum of the per-repo rows — rather than a negative fleet delta.
    freed=$(awk -F'\t' '{ s += $4 } END { printf "%d", s + 0 }' "$ROWS" 2>/dev/null)
    [[ "$freed" =~ ^[0-9]+$ ]] || freed=0
    echo
    echo "------------------------------------------------------------------"
    printf '%-16s %s\n' "fleet before" "$(fmt_kib "$before")"
    if [[ "$delta" -ge 0 ]]; then
        printf '%-16s %s   (-%s reclaimed by %s)\n' "fleet after" "$(fmt_kib "$after")" "$(fmt_kib "$delta")" "$phase"
    else
        printf '%-16s %s   (-%s reclaimed by %s; fleet grew %s during the run — builds were active)\n' \
            "fleet after" "$(fmt_kib "$after")" "$(fmt_kib "$freed")" "$phase" "$(fmt_kib "$((-delta))")"
    fi
    if [[ -s "$ROWS" ]]; then
        echo
        printf '  %-28s %11s %11s %11s  %s\n' "REPOSITORY" "BEFORE" "AFTER" "RECLAIMED" "REASON"
        local label b a f reason
        while IFS=$'\t' read -r label b a f reason; do
            [[ -n "$label" ]] || continue
            printf '  %-28s %11s %11s %11s  %s\n' \
                "$(trunc "$label" 28)" "$(fmt_kib "$b")" "$(fmt_kib "$a")" "$(fmt_kib "$f")" "$reason"
        done < "$ROWS"
    fi
    if [[ -s "$SKIPPED_LOCKED" ]]; then
        echo
        echo "skipped (build lock held): $(paste -sd, - < "$SKIPPED_LOCKED" | sed 's/,/, /g')"
    fi
    echo
}

# --- sweep -----------------------------------------------------------------

cmd_sweep() {
    prelude
    local before_total after_total root target label before after freed
    before_total=$(fleet_total_kib)
    echo "${BLUE}sweep: artifacts unused for ${DAYS}+ days, across $(wc -l < "$TARGETS" | tr -d ' ') target dir(s) under $SCAN_ROOT${NC}"

    while IFS=$'\t' read -r root target label; do
        [[ -n "$target" ]] || continue
        if [[ $FORCE -eq 0 ]] && build_locked "$target"; then
            locked_skip "$label"
            continue
        fi
        before=$(current_kib "$target")
        run_cargo_sweep "$root" "$target" --time "$DAYS"
        if [[ $DRY_RUN -eq 1 ]]; then
            freed=$(printf '%s\n' "$SWEEP_OUTPUT" | parse_clean_kib)
            [[ "$freed" -gt "$before" ]] && freed=$before
            after=$((before - freed))
        else
            after=$(size_kib "$target")
            freed=$((before - after))
            [[ "$freed" -lt 0 ]] && freed=0
        fi
        set_kib "$target" "$after"
        echo "${GREEN}${PREFIX}sweep   $label  $(fmt_kib "$before") -> $(fmt_kib "$after")  (-$(fmt_kib "$freed"))${NC}"
        record_row "$label" "$before" "$after" "$freed" "age sweep (${DAYS}d)"
    done < "$TARGETS"

    after_total=$(fleet_total_kib)
    print_footer "sweep" "$before_total" "$after_total"

    local rc=0
    enforce_cap || rc=$?
    staging_note
    return "$rc"
}

# --- purge -----------------------------------------------------------------

cmd_purge() {
    prelude
    local before_total after_total root target label before after inc sz protected epoch
    before_total=$(fleet_total_kib)
    protected=$(protected_labels)
    echo "${BLUE}purge: incremental caches everywhere, wholesale drop of repos idle ${IDLE_DAYS}+ days, then toolchain garbage${NC}"

    # 1. Incremental caches, every repo including the active one. The largest
    #    single reclaim and the only one whose cost lands on the very next build,
    #    which is why it lives here and never in sweep.
    while IFS=$'\t' read -r root target label; do
        [[ -n "$target" ]] || continue
        if [[ $FORCE -eq 0 ]] && build_locked "$target"; then
            locked_skip "$label"
            continue
        fi
        before=$(current_kib "$target")
        for inc in "$target"/*/incremental; do
            [[ -d "$inc" ]] || continue
            sz=$(size_kib "$inc")
            [[ "$sz" -gt 0 ]] || continue
            if [[ $DRY_RUN -eq 1 ]]; then
                echo "${BLUE}would   drop incremental cache $inc ($(fmt_kib "$sz"))${NC}"
            else
                echo "${GREEN}purge   incremental cache $inc ($(fmt_kib "$sz"))${NC}"
                rm -rf "${inc:?}"/*
            fi
        done
        if [[ $DRY_RUN -eq 1 ]]; then
            after=$(current_kib "$target")
            for inc in "$target"/*/incremental; do
                [[ -d "$inc" ]] || continue
                after=$((after - $(size_kib "$inc")))
            done
            [[ "$after" -lt 0 ]] && after=0
        else
            after=$(size_kib "$target")
        fi
        set_kib "$target" "$after"
        [[ "$before" -gt "$after" ]] && record_row "$label" "$before" "$after" "$((before - after))" "incremental caches"
    done < "$TARGETS"

    # 2. Wholesale drop of idle repos, oldest first, except the KEEP
    #    most-recently-built.
    while IFS=$'\t' read -r epoch root target label; do
        [[ -n "$target" ]] || continue
        [[ -d "$target" ]] || continue
        is_idle_epoch "$epoch" || continue
        if is_protected "$label" "$protected"; then
            echo "${DIM}keep    $label (one of the $KEEP most-recently-built)${NC}"
            continue
        fi
        if [[ $FORCE -eq 0 ]] && build_locked "$target"; then
            locked_skip "$label"
            continue
        fi
        before=$(current_kib "$target")
        [[ "$before" -gt 0 ]] || continue
        if [[ $DRY_RUN -eq 1 ]]; then
            echo "${BLUE}would   reclaim $label ($(fmt_kib "$before")) — $(idle_reason "$epoch")${NC}"
        else
            echo "${GREEN}reclaim $label ($(fmt_kib "$before")) — $(idle_reason "$epoch")${NC}"
            wholesale_reclaim "$target" "$label"
        fi
        record_row "$label" "$before" 0 "$before" "$(idle_reason "$epoch")"
        set_kib "$target" 0
    done < "$ORDER"

    # 3. Toolchain garbage on whatever survived: artifacts built by toolchains
    #    rustup does not have installed. Costs a warm build nothing.
    while IFS=$'\t' read -r root target label; do
        [[ -n "$target" ]] || continue
        [[ -d "$target" ]] || continue
        [[ "$(current_kib "$target")" -gt 0 ]] || continue
        if [[ $FORCE -eq 0 ]] && build_locked "$target"; then
            continue
        fi
        before=$(current_kib "$target")
        if [[ $DRY_RUN -eq 1 ]]; then
            echo "${BLUE}would   drop artifacts from uninstalled toolchains in $label${NC}"
        else
            echo "${GREEN}purge   artifacts from uninstalled toolchains in $label${NC}"
        fi
        run_cargo_sweep "$root" "$target" --installed
        if [[ $DRY_RUN -eq 1 ]]; then
            after=$((before - $(printf '%s\n' "$SWEEP_OUTPUT" | parse_clean_kib)))
            [[ "$after" -lt 0 ]] && after=0
        else
            after=$(size_kib "$target")
        fi
        set_kib "$target" "$after"
        [[ "$before" -gt "$after" ]] && record_row "$label" "$before" "$after" "$((before - after))" "unused toolchains"
    done < "$TARGETS"

    after_total=$(fleet_total_kib)
    print_footer "purge" "$before_total" "$after_total"

    local rc=0
    enforce_cap || rc=$?
    staging_note
    return "$rc"
}

# --- status ----------------------------------------------------------------

installed_cap() {
    [[ -f "$AGENT_DEST" ]] || return 0
    awk '
        /<string>--cap<\/string>/ { want = 1; next }
        want && /<string>/ {
            line = $0
            sub(/.*<string>/, "", line)
            sub(/<\/string>.*/, "", line)
            print line
            exit
        }' "$AGENT_DEST"
}

cmd_status() {
    discover_targets > "$TARGETS"
    local root target label size epoch state total=0 count=0 agent_cap

    printf '%-38s %11s   %-12s %s\n' "REPOSITORY" "SIZE" "LAST BUILT" "STATE"
    while IFS=$'\t' read -r root target label; do
        [[ -n "$target" ]] || continue
        size=$(size_kib "$target")
        epoch=$(target_mtime_epoch "$target")
        if build_locked "$target"; then
            state="building (lock held)"
        elif is_idle "$target"; then
            state="idle"
        else
            state="warm"
        fi
        printf '%-38s %11s   %-12s %s\n' "$(trunc "$label" 38)" "$(fmt_kib "$size")" "$(age_human "$epoch")" "$state"
        total=$((total + size))
        count=$((count + 1))
    done < "$TARGETS"

    agent_cap=$(installed_cap)
    echo "------------------------------------------------------------------"
    printf '%-38s %11s   %s\n' "FLEET TOTAL" "$(fmt_kib "$total")" "across $count target dir(s)"
    printf '%-38s %11s   %s\n' "CAP" "$(fmt_kib "$CAP_KIB")" "(scheduled agent uses: ${agent_cap:-not installed})"
    if [[ "$total" -le "$CAP_KIB" ]]; then
        printf '%-38s %11s   %s\n' "HEADROOM" "$(fmt_kib "$((CAP_KIB - total))")" "$(pct_of_cap "$total" "$CAP_KIB")% of cap used"
    else
        printf '%-38s %11s   %s\n' "OVER CAP BY" "$(fmt_kib "$((total - CAP_KIB))")" "$(pct_of_cap "$total" "$CAP_KIB")% of cap used"
    fi

    local warnings
    warnings=$(report_symlinked_targets; report_orphan_targets)
    if [[ -n "$warnings" ]]; then
        echo
        printf '%s\n' "$warnings"
    fi
    local note
    note=$(staging_note)
    [[ -n "$note" ]] && echo "$note"
    return 0
}

# --- install / uninstall ---------------------------------------------------

render_plist() {
    local content
    content=$(cat "$PLIST_TEMPLATE")
    content=${content//__HOME__/$HOME}
    content=${content//__SCRIPT__/$SCRIPT_PATH}
    content=${content//__CAP__/$CAP_SPEC}
    content=${content//__LOG__/$LOG_FILE}
    printf '%s\n' "$content"
}

cmd_install() {
    local uid
    uid=$(id -u)
    if [[ ! -f "$PLIST_TEMPLATE" ]]; then
        echo "${RED}Missing LaunchAgent template: $PLIST_TEMPLATE${NC}" >&2
        exit 1
    fi

    if [[ $DRY_RUN -eq 1 ]]; then
        echo "${BLUE}would render $PLIST_TEMPLATE -> $AGENT_DEST${NC}"
        render_plist
        return 0
    fi

    mkdir -p "$(dirname "$AGENT_DEST")"
    render_plist > "$AGENT_DEST"
    launchctl bootout "gui/$uid/$AGENT_LABEL" >/dev/null 2>&1 || true
    if ! launchctl bootstrap "gui/$uid" "$AGENT_DEST"; then
        echo "${RED}launchctl bootstrap failed for $AGENT_DEST${NC}" >&2
        exit 1
    fi
    if ! launchctl print "gui/$uid/$AGENT_LABEL" >/dev/null 2>&1; then
        echo "${RED}$AGENT_LABEL did not come up after bootstrap${NC}" >&2
        exit 1
    fi
    echo "${GREEN}installed $AGENT_LABEL — sweep --cap $CAP_SPEC nightly at 02:00 local time${NC}"
    echo "${DIM}log: $LOG_FILE${NC}"
}

cmd_uninstall() {
    local uid
    uid=$(id -u)
    if [[ $DRY_RUN -eq 1 ]]; then
        echo "${BLUE}would bootout gui/$uid/$AGENT_LABEL and remove $AGENT_DEST${NC}"
        return 0
    fi
    launchctl bootout "gui/$uid/$AGENT_LABEL" >/dev/null 2>&1 || true
    if [[ -f "$AGENT_DEST" ]]; then
        rm -f "$AGENT_DEST"
        echo "${GREEN}removed $AGENT_LABEL${NC}"
    else
        echo "${DIM}$AGENT_LABEL was not installed${NC}"
    fi
}

# --- main ------------------------------------------------------------------

case "$COMMAND" in
    sweep) cmd_sweep ;;
    purge) cmd_purge ;;
    status) cmd_status ;;
    install) cmd_install ;;
    uninstall) cmd_uninstall ;;
esac
