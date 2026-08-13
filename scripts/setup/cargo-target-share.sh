#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Shares one Cargo target directory per repository across all of that repo's git worktrees.
# ABOUTME: Provides status/link/unlink/prune so each worktree stops carrying a full copy of the build.

set -euo pipefail

# Where the shared per-repo target directories live. One subdirectory per repo,
# keyed by the repo that owns the .git directory, so every worktree of a repo
# resolves to the same key.
SHARED_ROOT="${CARGO_SHARED_TARGET_ROOT:-$HOME/.cargo-target}"

# Root scanned for candidate repositories when no paths are given.
SCAN_ROOT="${CARGO_TARGET_SHARE_ROOT:-$HOME/workspace}"

COMMAND="status"
RECLAIM=0
FORCE=0
DRY_RUN=0
ALL_REPOS=0
PURGE_INCREMENTAL=0
STALE_DAYS=45
PATHS=""

RED=$'\033[0;31m'
GREEN=$'\033[0;32m'
YELLOW=$'\033[0;33m'
BLUE=$'\033[0;34m'
DIM=$'\033[2m'
NC=$'\033[0m'

usage() {
    cat <<'EOF'
Usage: cargo-target-share.sh <command> [options] [repo-path...]

Points each repository's target/ at a single shared directory so that N git
worktrees of one repo share one build tree instead of carrying N copies.

Commands:
  status            Show what is linked, shared sizes, and reclaimable bytes (default)
  link              Point target/ at the shared per-repo directory
  unlink            Restore a private, empty target/ directory
  prune             Drop incremental caches and target dirs for repos gone stale

Options:
  --reclaim         During link, delete a redundant local target/ when a shared
                    one already holds the artifacts. Without this, link reports
                    the redundant directory and skips it.
  --stale-days N    prune: drop shared dirs untouched for N days (default: 45)
  --purge-incremental
                    prune: also delete incremental caches. Reclaims the most by
                    far, but costs the next build in every repo, so it is opt-in
                    and a scheduled prune should leave it off.
  --all             Consider every Cargo repo under the scan root, not just dravr-*
  --force           Proceed even when a cargo build holds the target lock
  --dry-run         Print what would happen, change nothing
  -h, --help        Show this help

Environment:
  CARGO_SHARED_TARGET_ROOT   Shared root (default: ~/.cargo-target)
  CARGO_TARGET_SHARE_ROOT    Scan root (default: ~/workspace)

Note: worktrees of the same repo now serialize their builds on one target lock.
A second concurrent cargo build prints "Blocking waiting for file lock on build
directory" and waits rather than failing. Third-party dependencies stay warm
across worktrees; workspace crates rebuild when you alternate between branches.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        status|link|unlink|prune) COMMAND="$1"; shift ;;
        --reclaim) RECLAIM=1; shift ;;
        --force) FORCE=1; shift ;;
        --dry-run) DRY_RUN=1; shift ;;
        --all) ALL_REPOS=1; shift ;;
        --purge-incremental) PURGE_INCREMENTAL=1; shift ;;
        --stale-days) STALE_DAYS="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        -*) echo "${RED}Unknown option: $1${NC}" >&2; usage >&2; exit 1 ;;
        *) PATHS="${PATHS}${PATHS:+$'\n'}$1"; shift ;;
    esac
done

# Resolve the sharing key for a directory: the name of the directory that owns
# the .git directory. For a worktree, --git-common-dir points at the main
# repository's .git, so every worktree of a repo collapses onto one key.
repo_key() {
    local dir="$1" common
    common=$(git -C "$dir" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || return 1
    [[ -n "$common" ]] || return 1
    basename "$(dirname "$common")"
}

# Size of a path, tolerant of concurrent writers. du exits non-zero when files
# disappear mid-scan, which happens constantly against a target dir that a build
# or a background rm is touching; under pipefail that would abort the script.
human_size() {
    local path="$1" out
    [[ -e "$path" ]] || { echo "-"; return 0; }
    out=$(du -sh "$path" 2>/dev/null | cut -f1) || out=""
    echo "${out:-?}"
}

PYTHON_BIN="$(command -v python3 2>/dev/null || true)"
[[ -x "$PYTHON_BIN" ]] || PYTHON_BIN=""

# True when a cargo build currently holds a lock inside the given target dir.
# Cargo creates .cargo-lock / .cargo-build-lock / .cargo-artifact-lock per
# profile and holds a flock(2) on them for the duration of a build, so probing
# that same lock non-blockingly is an exact answer. lsof can answer this too,
# but it enumerates every open file on the system and stalls for minutes while
# the disk is busy — which is precisely when this gets called.
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

# Cheap proxy for how warm a build tree is: the number of compiled artifacts
# under its deps/ directories. Reading a couple of directory listings costs
# milliseconds, where du has to walk hundreds of thousands of files.
target_warmth() {
    local target="$1" total=0 deps n
    for deps in "$target"/*/deps; do
        [[ -d "$deps" ]] || continue
        n=$(find "$deps" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l | tr -d ' ')
        total=$((total + n))
    done
    echo "$total"
}

# Emit "path<TAB>key" for every repo under consideration.
discover_repos() {
    local dir key
    if [[ -n "$PATHS" ]]; then
        while IFS= read -r dir; do
            [[ -n "$dir" ]] || continue
            key=$(repo_key "$dir") || { echo "${RED}Not a git repo: $dir${NC}" >&2; continue; }
            printf '%s\t%s\n' "$(cd "$dir" && pwd)" "$key"
        done <<< "$PATHS"
        return
    fi
    for dir in "$SCAN_ROOT"/*/; do
        dir="${dir%/}"
        [[ -f "$dir/Cargo.toml" ]] || continue
        key=$(repo_key "$dir") || continue
        if [[ $ALL_REPOS -eq 0 && "$key" != dravr-* ]]; then continue; fi
        printf '%s\t%s\n' "$dir" "$key"
    done
}

ensure_shared_root() {
    [[ $DRY_RUN -eq 1 ]] && return 0
    mkdir -p "$SHARED_ROOT"
    # Keep Spotlight from indexing hundreds of thousands of build artifacts that
    # just moved into a freshly-created directory outside the old workspace tree.
    [[ -e "$SHARED_ROOT/.metadata_never_index" ]] || : > "$SHARED_ROOT/.metadata_never_index"
}

cmd_status() {
    printf "%-52s %-22s %-10s %s\n" "WORKTREE" "SHARED KEY" "SIZE" "STATE"
    printf "%-52s %-22s %-10s %s\n" "--------" "----------" "----" "-----"
    local dir key target shared state size total_local=0
    while IFS=$'\t' read -r dir key; do
        [[ -n "$dir" ]] || continue
        target="$dir/target"
        shared="$SHARED_ROOT/$key"
        if [[ -L "$target" ]]; then
            if [[ "$(readlink "$target")" == "$shared" ]]; then
                if [[ -d "$shared" ]]; then
                    state="${GREEN}shared${NC}"
                else
                    state="${RED}DANGLING — run: $0 link${NC}"
                fi
            else
                state="${YELLOW}linked elsewhere -> $(readlink "$target")${NC}"
            fi
            size="-"
        elif [[ -d "$target" ]]; then
            size=$(human_size "$target")
            total_local=$((total_local + 1))
            if [[ -d "$shared" ]]; then
                state="${YELLOW}private (redundant; shared dir exists)${NC}"
            else
                state="${RED}private${NC}"
            fi
        else
            state="${DIM}no target${NC}"
            size="-"
        fi
        printf "%-52s %-22s %-10s %b\n" "$(basename "$dir")" "$key" "$size" "$state"
    done < <(discover_repos)

    echo
    echo "${BLUE}Shared directories under $SHARED_ROOT:${NC}"
    if [[ -d "$SHARED_ROOT" ]]; then
        local sub
        for sub in "$SHARED_ROOT"/*/; do
            [[ -d "$sub" ]] || continue
            printf "  %-30s %s\n" "$(basename "${sub%/}")" "$(human_size "${sub%/}")"
        done
        printf "  %-30s %s\n" "TOTAL" "$(human_size "$SHARED_ROOT")"
    else
        echo "  (none yet — run: $0 link)"
    fi
    echo
    echo "${DIM}$total_local worktree(s) still hold a private target/.${NC}"
}

# Per sharing key, choose the warmest unlocked private target/ as the one to move
# into the shared location. Adopting the richest tree keeps the most third-party
# dependency artifacts available, so every other worktree of that key reuses them
# instead of starting cold. Picking alphabetically instead would routinely
# discard a large warm tree in favour of a small partial one.
select_donors() {
    local inventory="$1" key
    cut -f1 "$inventory" | sort -u | while IFS= read -r key; do
        [[ -n "$key" ]] || continue
        shared_populated "$key" && continue
        awk -F'\t' -v k="$key" '$1 == k && $4 == "free"' "$inventory" \
            | sort -t"$(printf '\t')" -k2,2nr | head -1 | cut -f3
    done
}

# key <TAB> warmth <TAB> path <TAB> free|locked, for every real target/ dir.
build_inventory() {
    local dir key target warmth locked
    while IFS=$'\t' read -r dir key; do
        [[ -n "$dir" ]] || continue
        target="$dir/target"
        [[ -d "$target" && ! -L "$target" ]] || continue
        warmth=$(target_warmth "$target")
        locked="free"
        if [[ $FORCE -eq 0 ]] && build_locked "$target"; then locked="locked"; fi
        printf '%s\t%s\t%s\t%s\n' "$key" "$warmth" "$dir" "$locked"
    done < <(discover_repos)
}

# Keys whose shared directory a dry run would have created by this point. Without
# this, a dry run reports every non-donor worktree as "donor busy", because the
# donor's move never actually happened and the shared path is still absent.
PENDING_KEYS=""

# A shared tree that exists but is empty holds no artifacts, so it must never be
# treated as a reason to delete a populated private target/. This happens for
# real: linking a fresh worktree creates the shared dir before the main repo has
# donated anything to it.
shared_populated() {
    local shared="$SHARED_ROOT/$1"
    [[ -d "$shared" ]] || return 1
    [[ -n "$(find "$shared" -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null || true)" ]]
}

shared_ready() {
    local key="$1"
    shared_populated "$key" && return 0
    [[ $DRY_RUN -eq 1 && -n "$PENDING_KEYS" ]] && grep -qxF "$key" "$PENDING_KEYS" && return 0
    return 1
}

note_pending() {
    [[ $DRY_RUN -eq 1 && -n "$PENDING_KEYS" ]] && echo "$1" >> "$PENDING_KEYS"
    return 0
}

# Keep the target symlink and the staging dirs out of git status. This belongs in
# .git/info/exclude rather than .gitignore: the symlink only exists on machines
# where this tool has been run, so it is local state, not repo content. It is
# needed because a "target/" rule with a trailing slash matches directories only
# and does not cover a symlink — linking makes target/ show up as untracked in
# every repo whose .gitignore is written that way.
ensure_local_ignore() {
    local dir="$1" common exclude rule
    common=$(git -C "$dir" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || return 0
    [[ -n "$common" ]] || return 0
    exclude="$common/info/exclude"
    mkdir -p "$common/info"
    for rule in '/target' 'target.reclaim.*' 'target.stale.*'; do
        grep -qxF "$rule" "$exclude" 2>/dev/null || echo "$rule" >> "$exclude"
    done
}

# Wire up one worktree. is_donor=1 means this worktree's existing target/ is the
# tree that becomes the shared one for its key.
link_one() {
    local dir="$1" key="$2" is_donor="$3"
    local target="$dir/target" shared="$SHARED_ROOT/$key" sz staged

    if [[ -L "$target" ]]; then
        if [[ "$(readlink "$target")" == "$shared" ]]; then
            if [[ -d "$shared" ]]; then
                ensure_local_ignore "$dir"
                echo "${DIM}ok      $(basename "$dir") -> $key${NC}"
            elif [[ $DRY_RUN -eq 1 ]]; then
                echo "${BLUE}would   $(basename "$dir"): recreate missing $shared${NC}"
            else
                # Self-heal a link left dangling by a prune or a manual delete.
                mkdir -p "$shared"
                ensure_local_ignore "$dir"
                echo "${GREEN}repair  $(basename "$dir") -> $key ${DIM}(shared dir was missing)${NC}"
            fi
        else
            echo "${YELLOW}skip    $(basename "$dir"): target/ already links to $(readlink "$target")${NC}"
        fi
        return 0
    fi

    if [[ $FORCE -eq 0 ]] && build_locked "$target"; then
        echo "${YELLOW}skip    $(basename "$dir"): a cargo build holds the target lock (use --force to override)${NC}"
        return 0
    fi

    if [[ ! -e "$target" ]]; then
        if [[ $DRY_RUN -eq 1 ]]; then
            note_pending "$key"
            echo "${BLUE}would   $(basename "$dir"): create $shared and link${NC}"
        else
            mkdir -p "$shared"
            ln -s "$shared" "$target"
            ensure_local_ignore "$dir"
            echo "${GREEN}link    $(basename "$dir") -> $key ${DIM}(new)${NC}"
        fi
        return 0
    fi

    if [[ "$is_donor" -eq 1 ]]; then
        # Move this worktree's artifacts in wholesale. Same volume, so the
        # rename is instant no matter how many gigabytes it covers.
        if [[ $DRY_RUN -eq 1 ]]; then
            note_pending "$key"
            echo "${BLUE}would   $(basename "$dir"): move target/ -> $shared${NC}"
        else
            # An empty shared dir may already be sitting there, created when a
            # worktree of this repo was linked first. Clear it so the move
            # installs this tree in its place instead of nesting inside it.
            if [[ -d "$shared" ]]; then rmdir "$shared" 2>/dev/null || true; fi
            mv "$target" "$shared"
            ln -s "$shared" "$target"
            ensure_local_ignore "$dir"
            echo "${GREEN}adopt   $(basename "$dir") -> $key ${DIM}(moved existing artifacts, nothing rebuilt)${NC}"
        fi
        return 0
    fi

    if ! shared_ready "$key"; then
        # No shared tree for this key yet and this worktree was not the chosen
        # donor, which happens when the donor's build lock was held.
        echo "${YELLOW}hold    $(basename "$dir"): no shared tree for $key yet (donor busy) — re-run later${NC}"
        return 0
    fi

    # A shared tree already holds these artifacts, so this local copy is
    # redundant. Deleting it is the point, but it is destructive: opt in.
    sz=$(human_size "$target")
    if [[ $RECLAIM -eq 0 ]]; then
        echo "${YELLOW}hold    $(basename "$dir"): $sz redundant at $target — re-run with --reclaim to delete${NC}"
        return 0
    fi
    if [[ $DRY_RUN -eq 1 ]]; then
        echo "${BLUE}would   $(basename "$dir"): reclaim $sz and link -> $key${NC}"
        return 0
    fi
    # Rename first so the symlink goes live immediately, then delete in the
    # background: removing tens of GB of small files takes minutes.
    staged="$dir/target.reclaim.$$"
    mv "$target" "$staged"
    ln -s "$shared" "$target"
            ensure_local_ignore "$dir"
    ( rm -rf "$staged" >/dev/null 2>&1 & )
    echo "${GREEN}reclaim $(basename "$dir") -> $key ${DIM}($sz freeing in background)${NC}"
}

cmd_link() {
    ensure_shared_root
    local inventory donors dir key
    inventory=$(mktemp)
    donors=$(mktemp)
    PENDING_KEYS=$(mktemp)
    build_inventory > "$inventory"
    select_donors "$inventory" > "$donors"

    # Donors first: a redundant worktree can only be reclaimed once the shared
    # tree it is about to fall back on actually exists.
    while IFS= read -r dir; do
        [[ -n "$dir" ]] || continue
        key=$(repo_key "$dir") || continue
        link_one "$dir" "$key" 1
    done < "$donors"

    while IFS=$'\t' read -r dir key; do
        [[ -n "$dir" ]] || continue
        if grep -qxF "$dir" "$donors"; then continue; fi
        link_one "$dir" "$key" 0
    done < <(discover_repos)

    rm -f "$inventory" "$donors" "$PENDING_KEYS"
    PENDING_KEYS=""
}

cmd_unlink() {
    local dir key target
    while IFS=$'\t' read -r dir key; do
        [[ -n "$dir" ]] || continue
        target="$dir/target"
        if [[ ! -L "$target" ]]; then
            echo "${DIM}skip    $(basename "$dir"): target/ is not a link${NC}"
            continue
        fi
        if [[ $DRY_RUN -eq 1 ]]; then
            echo "${BLUE}would   $(basename "$dir"): restore private empty target/${NC}"
            continue
        fi
        rm "$target"
        mkdir -p "$target"
        echo "${GREEN}unlink  $(basename "$dir") ${DIM}(private target/, next build is cold)${NC}"
    done < <(discover_repos)
}

# True when nothing in this shared tree has been built recently. The directory's
# own mtime is useless for this: mv preserves it, so a tree adopted from a repo
# carries whatever timestamp it had years ago, and a directory mtime only moves
# when entries are added or removed at that exact level. Cargo rewrites these
# sentinels on every build, so they are what actually dates a tree.
is_stale() {
    local sub="$1" fresh
    fresh=$(find "$sub" -maxdepth 2 \
        \( -name '.rustc_info.json' -o -name '.cargo-lock' -o -name '.fingerprint' \) \
        -mtime -"$STALE_DAYS" -print -quit 2>/dev/null || true)
    [[ -z "$fresh" ]]
}

cmd_prune() {
    [[ -d "$SHARED_ROOT" ]] || { echo "Nothing to prune: $SHARED_ROOT does not exist."; return 0; }

    local inc before sub key

    # Incremental caches are the biggest single reclaim and the only one that
    # costs something real: dropping them slows the next build in every repo.
    # That makes them wrong to do on a schedule, so they are opt-in.
    if [[ $PURGE_INCREMENTAL -eq 0 ]]; then
        echo "${DIM}Keeping incremental caches (pass --purge-incremental to drop them)${NC}"
    else
        echo "${BLUE}Dropping incremental caches${NC}"
        while IFS= read -r inc; do
            [[ -n "$inc" ]] || continue
            # Deleting artifacts out from under a running build breaks it. link
            # already refuses on a held lock; prune has to as well.
            if [[ $FORCE -eq 0 ]] && build_locked "${inc%/*/incremental}"; then
                echo "${YELLOW}skip    ${inc%/*/incremental}: build in progress${NC}"
                continue
            fi
            before=$(human_size "$inc")
            if [[ $DRY_RUN -eq 1 ]]; then
                echo "${BLUE}would   purge $inc ($before)${NC}"
            else
                rm -rf "${inc:?}"/*
                echo "${GREEN}purged  $inc ${DIM}($before reclaimed)${NC}"
            fi
        done < <(find "$SHARED_ROOT" -maxdepth 3 -type d -name incremental 2>/dev/null)
    fi

    echo
    echo "${BLUE}Dropping shared dirs untouched for ${STALE_DAYS}+ days${NC}"
    for sub in "$SHARED_ROOT"/*/; do
        [[ -d "$sub" ]] || continue
        sub="${sub%/}"
        key=$(basename "$sub")
        is_stale "$sub" || continue
        if [[ $FORCE -eq 0 ]] && build_locked "$sub"; then
            echo "${YELLOW}skip    $key: build in progress${NC}"
            continue
        fi
        if [[ $DRY_RUN -eq 1 ]]; then
            echo "${BLUE}would   drop $key ($(human_size "$sub"), stale)${NC}"
        else
            echo "${GREEN}drop    $key ${DIM}($(human_size "$sub") freeing in background)${NC}"
            mv "$sub" "$sub.stale.$$"
            # Recreate immediately: worktrees point their target/ symlink here, and
            # a dangling link fails the next build with "Not a directory".
            mkdir -p "$sub"
            ( rm -rf "$sub.stale.$$" >/dev/null 2>&1 & )
        fi
    done

    if command -v cargo-sweep >/dev/null 2>&1; then
        echo
        echo "${BLUE}cargo-sweep: removing artifacts unused for ${STALE_DAYS} days${NC}"
        # Sweep the repositories, never SHARED_ROOT. cargo-sweep searches for Cargo
        # projects, and the shared trees are bare target dirs with no manifest, so
        # aiming it at the shared root finds nothing and silently does nothing while
        # still reporting success. Going in through the repos resolves target/ via
        # the symlink onto those same trees. One worktree per key is enough: the
        # others reach the identical tree, and sweeping it twice just walks it twice.
        local swept="" dir key
        local sweep_args=()
        while IFS=$'\t' read -r dir key; do
            [[ -n "$dir" ]] || continue
            case "$swept" in *"|$key|"*) continue ;; esac
            swept="$swept|$key|"
            if [[ $FORCE -eq 0 ]] && build_locked "$SHARED_ROOT/$key"; then
                echo "${YELLOW}  skip $key: build in progress${NC}"
                continue
            fi
            sweep_args+=("$dir")
        done < <(discover_repos)
        if [[ ${#sweep_args[@]} -eq 0 ]]; then
            echo "${DIM}  no linked repositories to sweep${NC}"
        elif [[ $DRY_RUN -eq 1 ]]; then
            cargo sweep --time "$STALE_DAYS" --dry-run "${sweep_args[@]}" 2>&1 \
                | grep -Ei 'clean' | sed 's/^/  /' || true
        else
            cargo sweep --time "$STALE_DAYS" "${sweep_args[@]}" 2>&1 \
                | grep -Ei 'clean' | sed 's/^/  /' || true
        fi
    else
        echo
        echo "${DIM}cargo-sweep not installed — age-based artifact pruning skipped.${NC}"
        echo "${DIM}  Install for finer-grained pruning: cargo install cargo-sweep${NC}"
    fi
    echo
    echo "Remaining: $(human_size "$SHARED_ROOT")"
}

case "$COMMAND" in
    status) cmd_status ;;
    link)   cmd_link ;;
    unlink) cmd_unlink ;;
    prune)  cmd_prune ;;
esac
