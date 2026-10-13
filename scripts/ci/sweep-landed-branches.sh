#!/usr/bin/env bash
# ABOUTME: Deletes every org branch whose content is already on its repo's default branch
# ABOUTME: The cleanup a Claude Code Web session cannot do — its git proxy refuses a branch delete
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A branch's work reaches main as a squash, a new SHA, so the branch is dead the
# moment it lands and the session that landed it deletes it. A cloud session
# cannot: its git proxy refuses both `push --delete` and the REST ref delete, so
# it hands the cleanup to a human, and the human's inbox is where it stops.
# 2026-10-13: ~70 branches across 19 repos, two of them handed off by name.
#
# Squash also defeats every "is it merged" test git offers: the branch tip is
# never an ancestor of main, and `git merge-tree` reports a conflict whenever
# main has since edited a line the branch touched. So landed is decided by
# content, file by file, over every path the branch changed since its fork
# point (renames split into their delete and add halves):
#
#   1. main held exactly the branch's version (mode and object) at some commit
#      after the fork point — a later edit on main does not un-land it; or
#   2. the branch's diff for that path reverse-applies cleanly onto main's tip,
#      i.e. every hunk is present even though main's file differs elsewhere.
#
# One path failing both makes the branch NOT landed, and it is kept. A branch
# sharing no history with the default branch is kept; so is the default branch,
# a protected branch, the head or base of an open pull request, dependabot's
# own branches, and anything whose tip is younger than --min-age-hours.
#
# The delete is a compare-and-swap push (`--force-with-lease` on the SHA that
# was checked), so a branch someone pushed to after the check survives. Every
# deletion is printed with its SHA and the one command that restores it — except
# under --redact-private, which the public CI lane passes: a private repository
# is then only counted, and its deletions are read back from that repository's
# own activity log.
#
# Usage:
#   sweep-landed-branches.sh landed <git-dir> <main-ref> <branch-ref>
#       exit 0 landed, 1 not landed (names the first path main lacks), 2 error
#   sweep-landed-branches.sh sweep [--dry-run] [--redact-private] [--min-age-hours N] [--org O]
#       exit 0 swept, 1 on an empty token or any listing, clone, content-check
#       or delete failure, 2 on a bad argument
#
# `sweep` reads GH_TOKEN (repo scope across the org) for both the API and git,
# through gh's credential helper, so the token never appears in a URL or a log.
set -euo pipefail

# A changed path is data, never a pattern: `docs/[draft].md` must not glob.
export GIT_LITERAL_PATHSPECS=1

# ---------------------------------------------------------------------------
# landed: the content test. Pure git, no network beyond lazy blob fetches.
# ---------------------------------------------------------------------------

# entry <git-dir> <commit> <path> -> "mode type object", or "absent"; returns 2
# when git fails, because two unread entries would compare equal as "absent".
entry() {
    local line
    line=$(git -C "$1" ls-tree -z "$2" -- "$3" | tr '\0' '\n' | head -1 | cut -f1) || return 2
    if [ -n "$line" ]; then echo "$line"; else echo "absent"; fi
}

# main_held <git-dir> <fork-point> <main> <branch> <path>: rule 1; 2 on a git failure
main_held() {
    local dir="$1" mb="$2" main="$3" branch="$4" path="$5" want held c
    want=$(entry "$dir" "$branch" "$path") || return 2
    held=$(entry "$dir" "$main" "$path") || return 2
    [ "$held" = "$want" ] && return 0
    while IFS= read -r c; do
        held=$(entry "$dir" "$c" "$path") || return 2
        [ "$held" = "$want" ] && return 0
    done < <(git -C "$dir" rev-list "$mb..$main" -- "$path")
    return 1
}

# hunks_present <git-dir> <fork-point> <main> <branch> <path> <index-file>: rule 2
# `git apply` only warns when a file's mode differs from the patch's, so the
# mode is compared on its own: a branch that made a script executable has not
# landed while main's copy is still 100644. Returns 2 on a git failure.
hunks_present() {
    local dir="$1" mb="$2" main="$3" branch="$4" path="$5" idx="$6" want held
    git -C "$dir" diff --no-renames --binary "$mb" "$branch" -- "$path" \
        | GIT_INDEX_FILE="$idx" git -C "$dir" apply --cached -R --check >/dev/null 2>&1 \
        || return 1
    want=$(entry "$dir" "$branch" "$path") || return 2
    [ "$want" = absent ] && return 0
    held=$(entry "$dir" "$main" "$path") || return 2
    [ "${want%% *}" = "${held%% *}" ]
}

cmd_landed() {
    if [ $# -ne 3 ]; then
        echo "usage: $0 landed <git-dir> <main-ref> <branch-ref>" >&2
        return 2
    fi
    local dir="$1" main="$2" branch="$3" mb path changed held idx rc=0
    local -a misses=()

    git -C "$dir" rev-parse -q --verify "$main^{commit}" >/dev/null \
        || { echo "unknown ref: $main" >&2; return 2; }
    git -C "$dir" rev-parse -q --verify "$branch^{commit}" >/dev/null \
        || { echo "unknown ref: $branch" >&2; return 2; }
    if ! mb=$(git -C "$dir" merge-base "$main" "$branch"); then
        echo "no shared history with $main"
        return 1
    fi

    # Fast path: an ordinary merge, or a branch that never moved off main.
    if git -C "$dir" merge-base --is-ancestor "$branch" "$main"; then
        echo "landed: tip is an ancestor of $main"
        return 0
    fi

    # Rule 1 reads trees only, so on a blobless clone it costs no fetch. The
    # path list goes through a file so git's exit status is checked: a failed
    # diff is an error, never an empty list that reads as nothing left to land.
    changed=$(mktemp)
    if ! git -C "$dir" diff --no-renames --name-only -z "$mb" "$branch" >"$changed"; then
        rm -f "$changed"
        echo "listing the paths $branch changed failed" >&2
        return 2
    fi
    while IFS= read -r -d '' path; do
        held=0
        main_held "$dir" "$mb" "$main" "$branch" "$path" || held=$?
        case $held in
            0) ;;
            1) misses+=("$path") ;;
            *) rm -f "$changed"; echo "reading $path failed" >&2; return 2 ;;
        esac
    done <"$changed"
    rm -f "$changed"

    if [ ${#misses[@]} -eq 0 ]; then
        echo "landed: every changed path is on $main"
        return 0
    fi

    # Rule 2 reads blobs; run it only for the paths rule 1 missed, and stop at
    # the first one that is truly absent.
    idx=$(mktemp)
    if ! GIT_INDEX_FILE="$idx" git -C "$dir" read-tree "$main"; then
        rm -f "$idx"
        echo "reading $main into an index failed" >&2
        return 2
    fi
    for path in "${misses[@]}"; do
        hunks_present "$dir" "$mb" "$main" "$branch" "$path" "$idx" || rc=$?
        if [ $rc -ne 0 ]; then break; fi
    done
    rm -f "$idx"
    case $rc in
        0) echo "landed: every changed path is on $main" ;;
        1) echo "not landed: $path" ;;
        *) echo "reading $path failed" >&2; rc=2 ;;
    esac
    return $rc
}

# ---------------------------------------------------------------------------
# sweep: enumerate the org, test each candidate branch, delete the landed ones.
# ---------------------------------------------------------------------------

report() {
    echo "$*"
    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then echo "$*" >>"$GITHUB_STEP_SUMMARY"; fi
}

cmd_sweep() {
    local dry_run=0 redact=0 min_age_hours=24 org="dravr-ai"
    while [ $# -gt 0 ]; do
        case "$1" in
            --dry-run) dry_run=1 ;;
            --redact-private) redact=1 ;;
            --min-age-hours) min_age_hours="$2"; shift ;;
            --org) org="$2"; shift ;;
            *) echo "unknown argument: $1" >&2; return 2 ;;
        esac
        shift
    done
    case "$min_age_hours" in
        ''|*[!0-9]*) echo "--min-age-hours wants a whole number, got '$min_age_hours'" >&2; return 2 ;;
    esac
    if [ -z "${GH_TOKEN:-}" ]; then
        echo "GH_TOKEN is empty — a sweep with no credential lists nothing and reads as clean" >&2
        return 1
    fi
    # Git speaks HTTPS through gh's credential helper and inherits nothing: a
    # developer config rewriting https:// to ssh://, or a keychain helper
    # answering first with another identity, would route around GH_TOKEN. The
    # clone records the helper in its own config, so the lazy blob fetches a
    # blobless clone makes later authenticate the same way.
    export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GIT_TERMINAL_PROMPT=0

    local work repos now quiet=0 hidden=0
    local deleted=0 kept=0 skipped=0 errors=0
    local p_deleted=0 p_kept=0 p_skipped=0 p_errors=0
    work=$(mktemp -d)
    # shellcheck disable=SC2064 # expand now: $work is local and gone by EXIT
    trap "rm -rf '$work'" EXIT
    now=$(date +%s)

    # item <deleted|kept|skipped|errors> <line> [<line for a redacted repo>]
    # Counts every outcome; a redacted repo's line is replaced by the second
    # form, or dropped when there is none, because a public run log must never
    # name a private repository, its branches or its paths.
    item() {
        case "$1" in
            deleted) deleted=$((deleted + 1)); p_deleted=$((p_deleted + quiet)) ;;
            kept) kept=$((kept + 1)); p_kept=$((p_kept + quiet)) ;;
            skipped) skipped=$((skipped + 1)); p_skipped=$((p_skipped + quiet)) ;;
            errors) errors=$((errors + 1)); p_errors=$((p_errors + quiet)) ;;
        esac
        if [ $quiet -eq 0 ]; then
            if [ -n "$2" ]; then report "$2"; fi
        elif [ -n "${3:-}" ]; then
            report "$3"
        fi
    }

    if ! repos=$(gh api "orgs/$org/repos?per_page=100&type=all" --paginate \
            --jq '.[] | select(.archived | not) | "\(.name)\t\(.default_branch)\t\(.private)"'); then
        echo "listing the repositories of $org failed" >&2
        return 1
    fi
    if [ -z "$repos" ]; then
        echo "$org lists no repositories — the token reaches nothing" >&2
        return 1
    fi

    local mode="deleting"
    [ $dry_run -eq 1 ] && mode="dry run, deleting nothing"
    report "## Landed-branch sweep — $org ($mode, tips older than ${min_age_hours}h)"
    report ""

    local name default private dir branches protected prs b sha age verdict
    local private_err="- ❌ a private repository"
    while IFS=$'\t' read -r name default private; do
        quiet=0
        if [ $redact -eq 1 ] && [ "$private" = true ]; then
            quiet=1; hidden=$((hidden + 1))
        fi
        # gh's stderr names the URL, so it goes to a file and reaches the log
        # only on a line a redacted repository never prints.
        if ! branches=$(gh api "repos/$org/$name/branches?per_page=100" --paginate --jq '.[].name' 2>"$work/gh.err"); then
            item errors "- ❌ \`$name\`: listing branches failed — $(tail -1 "$work/gh.err")" \
                "$private_err: listing branches failed"
            continue
        fi
        branches=$(echo "$branches" | grep -vxF "$default" | grep -v '^dependabot/' || true)
        [ -z "$branches" ] && continue
        # An open pull request's base is held as well as its head: deleting
        # the base closes the pull request.
        if ! protected=$(gh api "repos/$org/$name/branches?protected=true&per_page=100" --paginate --jq '.[].name' 2>"$work/gh.err") \
            || ! prs=$(gh api "repos/$org/$name/pulls?state=open&per_page=100" --paginate \
                --jq ".[] | .base.ref, (select(.head.repo.full_name == \"$org/$name\") | .head.ref)" 2>"$work/gh.err"); then
            item errors "- ❌ \`$name\`: listing protected branches or open pull requests failed — $(tail -1 "$work/gh.err")" \
                "$private_err: listing protected branches or open pull requests failed"
            continue
        fi

        dir="$work/$name.git"
        if ! git clone --quiet --bare --filter=blob:none --config 'credential.helper=!gh auth git-credential' \
                "https://github.com/$org/$name.git" "$dir" 2>"$work/clone.err"; then
            item errors "- ❌ \`$name\`: clone failed — $(tail -1 "$work/clone.err")" "$private_err: clone failed"
            continue
        fi

        while IFS= read -r b; do
            if grep -qxF "$b" <<<"$protected"; then
                item skipped "- ⏭️ \`$name\` \`$b\`: protected"; continue
            fi
            if grep -qxF "$b" <<<"$prs"; then
                item skipped "- ⏭️ \`$name\` \`$b\`: head or base of an open pull request"; continue
            fi
            if ! sha=$(git -C "$dir" rev-parse -q --verify "refs/heads/$b^{commit}"); then
                item errors "- ❌ \`$name\` \`$b\`: listed by the API but absent from the clone" \
                    "$private_err: a listed branch is absent from the clone"
                continue
            fi
            age=$(( (now - $(git -C "$dir" log -1 --format=%ct "$sha")) / 3600 ))
            if [ "$age" -lt "$min_age_hours" ]; then
                item skipped ""; continue
            fi

            local rc=0
            verdict=$(cmd_landed "$dir" "refs/heads/$default" "$sha" 2>&1) || rc=$?
            if [ $rc -eq 1 ]; then
                item kept "- 🟡 \`$name\` \`$b\` (${age}h): kept, ${verdict}"; continue
            elif [ $rc -ne 0 ]; then
                item errors "- ❌ \`$name\` \`$b\`: content check failed — $verdict" \
                    "$private_err: a content check failed"
                continue
            fi

            if [ $dry_run -eq 1 ]; then
                item deleted "- 🗑️ \`$name\` \`$b\` @ \`${sha:0:12}\` (${age}h): would delete, ${verdict}"
                continue
            fi
            if git -C "$dir" push --quiet --force-with-lease="refs/heads/$b:$sha" origin ":refs/heads/$b" 2>"$work/push.err"; then
                item deleted "- 🗑️ \`$name\` \`$b\` @ \`${sha:0:12}\` (${age}h): deleted, ${verdict}. Restore: \`gh api -X POST repos/$org/$name/git/refs -f ref=refs/heads/$b -f sha=$sha\`"
            else
                item errors "- ❌ \`$name\` \`$b\`: delete refused — $(tail -1 "$work/push.err")" \
                    "$private_err: a delete was refused"
            fi
        done <<<"$branches"
        rm -rf "$dir"
    done <<<"$repos"

    report ""
    local verb="deleted"
    [ $dry_run -eq 1 ] && verb="would delete"
    if [ $hidden -gt 0 ]; then
        # Every deletion is still recoverable: the repository's own activity
        # log records the ref and the SHA it pointed at, readable only by
        # those with access to the repository.
        report "- 🔒 $hidden private repositories swept without itemizing, because this log is public:" \
            "$verb $p_deleted, kept $p_kept unlanded, skipped $p_skipped, errors $p_errors." \
            "A deleted branch's SHA is in its repository's activity log:" \
            "\`gh api 'repos/$org/<repo>/activity?activity_type=branch_deletion'\`;" \
            "run \`sweep --dry-run\` locally for the itemized view."
        report ""
    fi
    report "**$verb $deleted, kept $kept unlanded, skipped $skipped, errors $errors**"
    [ $errors -eq 0 ]
}

case "${1:-}" in
    landed) shift; cmd_landed "$@" ;;
    sweep) shift; cmd_sweep "$@" ;;
    *)
        echo "usage: $0 landed <git-dir> <main-ref> <branch-ref>" >&2
        echo "       $0 sweep [--dry-run] [--redact-private] [--min-age-hours N] [--org O]" >&2
        exit 2
        ;;
esac
