#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: carnet — file, claim, release, label and close issues in the private register
# ABOUTME: A claim records who holds an issue and which Claude Code session, so a peer session sees it first
#
# The register is the tracker named by registre.toml (`tracker = "owner/repo"`), overridable
# with REGISTRE_TRACKER. Every dravr-* repo files into the same one, so this script is shared
# through dravr-build-config and must stay portable: macOS bash 3.2 (no associative arrays,
# no mapfile, no ${var,,}), BSD sed, jq, gh.
#
# A claim is three native GitHub facts that move together:
#   assignee            = the human accountable (gh login)
#   label  in-progress  = a live session holds the issue
#   newest marker line  = which session — an HTML comment carrying one JSON object:
#       <!-- carnet-claim {"v":1,"session":"<uuid>","name":"…","user":"…","host":"…","pid":N,"repo":"…","branch":"…","at":"…"} -->
#       <!-- carnet-release {…,"reason":"…"} -->
# `status` reads the newest marker; the label and assignee are the list-view mirrors of it.
set -euo pipefail

MARKER_VERSION=1
CLAIM_LABEL="in-progress"
DRY_RUN=0

# ------------------------------------------------------------------ output helpers
say()  { printf '%s\n' "$*"; }
warn() { printf '⚠️  %s\n' "$*" >&2; }
die()  { printf '❌ %s\n' "$1" >&2; exit "${2:-1}"; }
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required (brew install $1)"; }
now()  { date -u +%Y-%m-%dT%H:%M:%SZ; }

# Every mutating gh call goes through run(), so --dry-run shows exactly what would change.
run() {
    if [ "$DRY_RUN" = 1 ]; then
        printf '[dry-run] %s\n' "$*" >&2
        return 0
    fi
    "$@"
}

usage() {
    cat <<'EOF'
carnet — the private register, from the command line

  carnet.sh claim   <n> [--steal]                 hold carnet#n: assign me, label in-progress, marker comment
  carnet.sh release <n> [--reason <why>]          let go of carnet#n (inverse of claim)
  carnet.sh release --all [--reason <why>] [--session <uuid>]
                                                   release everything this session (or <uuid>) still holds
  carnet.sh status  [<n>] [--short]               who holds carnet#n — or every in-progress issue
  carnet.sh mine    [--verify]                    what this session holds, from its local ledger
  carnet.sh create  --title <t> [--label <l>]... (--body <b> | --body-file <f> | stdin) [--claim]
                                                   file an issue: "[<project>] <t>", project label, private tracker
  carnet.sh close   <n> --why <text> [--commit <sha>]
                                                   close with a mandatory reason and the commit that resolved it
  carnet.sh label   <n> +<label> -<label> ...     add / remove labels

  --dry-run   print the gh commands that would mutate the tracker instead of running them

Exit codes: 0 done · 1 error · 2 refused (held by another live session, or on another host)
EOF
}

# ------------------------------------------------------------------ repo + tracker
need gh
need jq
need git

REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || die "run this inside a git checkout"

TRACKER=${REGISTRE_TRACKER:-}
if [ -z "$TRACKER" ]; then
    [ -f "$REPO_ROOT/registre.toml" ] || die "no registre.toml at $REPO_ROOT — this repo names no register (see .build/README.md)"
    TRACKER=$(sed -n 's/^tracker[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$REPO_ROOT/registre.toml" | head -1)
fi
[ -n "$TRACKER" ] || die "registre.toml has no tracker key"

# The repo name comes from origin, never from the checkout's basename: a worktree is named
# after its branch (pierre_mcp_server-feature-x), which is not a project.
ORIGIN_URL=$(git -C "$REPO_ROOT" remote get-url origin 2>/dev/null || true)
ORIGIN_SLUG=$(printf '%s' "$ORIGIN_URL" | sed -E 's#^(git@github\.com:|https://github\.com/|ssh://git@github\.com/)##; s#\.git$##; s#/$##')
REPO_NAME=${ORIGIN_SLUG##*/}
[ -n "$REPO_NAME" ] || REPO_NAME=$(basename "$REPO_ROOT")
PROJECT=${REPO_NAME#dravr-}
BRANCH=$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
BRANCH=${BRANCH//--/-}

# ------------------------------------------------------------------ session identity
# Claude Code exports the session id and its own pid; the per-process file under the config
# dir carries the name set by /rename (or the derived one). Outside Claude Code the claim is
# a manual one: it still records the human, and nothing auto-releases it.
CFG=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
LEDGER_DIR="$CFG/carnet-claims"
SESSION_ID=${CLAUDE_CODE_SESSION_ID:-}
SESSION_PID=${CLAUDE_PID:-}
HOST=$(hostname -s 2>/dev/null || hostname)
NAME=""
USER_LOGIN=""
SHORT_ID=""

session_name() {
    local f="$CFG/sessions/${SESSION_PID:-none}.json" n=""
    if [ -n "$SESSION_PID" ] && [ -f "$f" ]; then
        n=$(jq -r --arg sid "$SESSION_ID" 'select(.sessionId == $sid) | .name // empty' "$f" 2>/dev/null || true)
    fi
    [ -n "$n" ] || n=${SESSION_ID:0:8}
    [ -n "$n" ] || n="shell"
    # "--" would end the HTML comment the marker lives in.
    printf '%s' "${n//--/-}"
}

gh_login() {
    local cache="$LEDGER_DIR/gh-login" login
    if [ -f "$cache" ] && [ -n "$(find "$cache" -mmin -1440 2>/dev/null)" ]; then
        cat "$cache"
        return 0
    fi
    login=$(gh api user -q .login 2>/dev/null) || die "gh api user failed — run: gh auth login"
    mkdir -p "$LEDGER_DIR"
    printf '%s' "$login" > "$cache"
    printf '%s' "$login"
}

load_identity() {
    NAME=$(session_name)
    USER_LOGIN=$(gh_login)
    SHORT_ID=${SESSION_ID:0:8}
    [ -n "$SHORT_ID" ] || SHORT_ID=manual
}

identity_json() {
    jq -cn \
        --arg v "$MARKER_VERSION" --arg session "${SESSION_ID:-manual}" --arg name "$NAME" \
        --arg user "$USER_LOGIN" --arg host "$HOST" --arg pid "${SESSION_PID:-$$}" \
        --arg repo "$REPO_NAME" --arg branch "$BRANCH" --arg at "$(now)" \
        '{v:($v|tonumber), session:$session, name:$name, user:$user, host:$host,
          pid:($pid|tonumber), repo:$repo, branch:$branch, at:$at}'
}

# ------------------------------------------------------------------ local ledger
# One JSON-lines file per session: line 1 is the identity, then one line per held issue.
# The SessionEnd hook reads it to release what the session still holds; `mine` reads it
# without a single API call.
ledger_file() { [ -n "$SESSION_ID" ] && printf '%s' "$LEDGER_DIR/$SESSION_ID.jsonl"; }

ledger_add() {
    local f
    f=$(ledger_file) || return 0
    mkdir -p "$LEDGER_DIR"
    [ -s "$f" ] || identity_json | jq -c '. + {kind:"identity"}' > "$f"
    jq -cn --arg t "$TRACKER" --argjson n "$1" --arg at "$(now)" \
        '{kind:"claim", tracker:$t, issue:$n, at:$at}' >> "$f"
}

ledger_drop() {
    local f tmp
    f=$(ledger_file) || return 0
    [ -f "$f" ] || return 0
    tmp=$(mktemp)
    jq -c --arg t "$TRACKER" --argjson n "$1" \
        'select((.kind == "claim" and .tracker == $t and .issue == $n) | not)' "$f" > "$tmp"
    mv "$tmp" "$f"
    # A ledger holding only its identity line is finished.
    if [ "$(jq -c 'select(.kind == "claim")' "$f" | wc -l | tr -d ' ')" = 0 ]; then rm -f "$f"; fi
}

ledger_issues() { # <file>
    [ -f "$1" ] || return 0
    jq -r --arg t "$TRACKER" 'select(.kind == "claim" and .tracker == $t) | .issue' "$1"
}

ledger_has() {
    local f
    f=$(ledger_file) || return 1
    ledger_issues "$f" | grep -qx "$1"
}

# ------------------------------------------------------------------ tracker reads
issue_json() {
    gh issue view "$1" -R "$TRACKER" --json number,title,url,state,labels,assignees 2>/dev/null \
        || die "carnet#$1 does not exist in $TRACKER"
}

# Newest marker comment on the issue, or nothing. Claims and releases share one stream, so the
# last one wins.
last_marker() {
    gh api "repos/$TRACKER/issues/$1/comments" --paginate -q '.[].body' 2>/dev/null \
        | grep -oE '<!-- carnet-(claim|release) \{.*\} -->' | tail -1 || true
}
marker_kind() { local k=${1#<!-- carnet-}; printf '%s' "${k%% *}"; }
marker_json() { local j=${1#<!-- carnet-* }; printf '%s' "${j% -->}"; }

# 0 = that session is running on this host · 1 = it ended · 2 = another host, unknowable here.
# Claude Code removes sessions/<pid>.json on exit, so "file with matching id + live pid" is alive.
holder_alive() { # <host> <pid> <session>
    [ "$1" = "$HOST" ] || return 2
    local f
    for f in "$CFG/sessions/$2.json" "$HOME"/.claude*/sessions/"$2".json; do
        [ -f "$f" ] || continue
        [ "$(jq -r '.sessionId // empty' "$f" 2>/dev/null)" = "$3" ] || continue
        kill -0 "$2" 2>/dev/null && return 0
    done
    return 1
}

has_label() { jq -e --arg l "$2" '.labels[]? | select(.name == $l)' >/dev/null 2>&1 <<<"$1"; }

# ------------------------------------------------------------------ claim
cmd_claim() { # <n> <steal>
    local n=$1 steal=$2 issue marker holder extra="" prev_user="" st
    issue=$(issue_json "$n")
    [ "$(jq -r .state <<<"$issue")" = "OPEN" ] || die "carnet#$n is closed — nothing to claim"

    marker=$(last_marker "$n")
    if [ -n "$marker" ] && [ "$(marker_kind "$marker")" = claim ]; then
        holder=$(marker_json "$marker")
        local hs hn hu hh hp ha
        hs=$(jq -r .session <<<"$holder"); hn=$(jq -r .name <<<"$holder"); hu=$(jq -r .user <<<"$holder")
        hh=$(jq -r .host <<<"$holder");    hp=$(jq -r .pid <<<"$holder");  ha=$(jq -r .at <<<"$holder")
        if [ "$hs" = "${SESSION_ID:-manual}" ]; then
            say "🔒 carnet#$n is already held by this session ($NAME)"
            return 0
        fi
        if holder_alive "$hh" "$hp" "$hs"; then st=0; else st=$?; fi
        case $st in
            0)  [ "$steal" = 1 ] || die "carnet#$n is held by @$hu · session $hn (${hs:0:8}) on $hh, still running since $ha. Re-run with --steal to take it over." 2
                warn "stealing carnet#$n from live session $hn (${hs:0:8}) of @$hu"
                extra="⚠️ Stolen from live session **$hn** (\`${hs:0:8}\`) of @$hu, held since $ha — that session no longer owns this issue." ;;
            1)  extra="♻️ Took over from ended session **$hn** (\`${hs:0:8}\`) of @$hu, held since $ha." ;;
            2)  [ "$steal" = 1 ] || die "carnet#$n is held by @$hu · session $hn (${hs:0:8}) on host $hh since $ha — liveness cannot be checked from $HOST. Re-run with --steal to take it over." 2
                warn "stealing carnet#$n from session $hn (${hs:0:8}) of @$hu on $hh"
                extra="⚠️ Stolen from session **$hn** (\`${hs:0:8}\`) of @$hu on $hh, held since $ha — that session no longer owns this issue." ;;
        esac
        prev_user=$hu
    fi

    local body
    body=$(mktemp)
    {
        printf '<!-- carnet-claim %s -->\n' "$(identity_json)"
        printf '🔒 Claimed by @%s · session **%s** (`%s`) on %s · `%s` @ `%s` · %s\n' \
            "$USER_LOGIN" "$NAME" "$SHORT_ID" "$HOST" "$REPO_NAME" "$BRANCH" "$(now)"
        [ -z "$extra" ] || printf '\n%s\n' "$extra"
    } > "$body"

    if [ -n "$prev_user" ] && [ "$prev_user" != "$USER_LOGIN" ]; then
        run gh issue edit "$n" -R "$TRACKER" --remove-assignee "$prev_user" >/dev/null
    fi
    run gh issue edit "$n" -R "$TRACKER" --add-assignee "$USER_LOGIN" --add-label "$CLAIM_LABEL" >/dev/null
    run gh issue comment "$n" -R "$TRACKER" --body-file "$body" >/dev/null
    rm -f "$body"
    [ "$DRY_RUN" = 1 ] || ledger_add "$n"
    say "🔒 carnet#$n claimed by @$USER_LOGIN · session $NAME ($SHORT_ID) · $(jq -r .title <<<"$issue")"
}

# ------------------------------------------------------------------ release
release_comment() { # <reason> [extra lines...]
    local reason=$1; shift
    printf '<!-- carnet-release %s -->\n' "$(identity_json | jq -c --arg r "$reason" '. + {reason:$r}')"
    printf '🔓 Released by @%s · session **%s** (`%s`) · %s\n' "$USER_LOGIN" "$NAME" "$SHORT_ID" "$reason"
    local line
    for line in "$@"; do printf '%s\n' "$line"; done
}

cmd_release() { # <n> <reason>
    local n=$1 reason=${2:-done} marker holder held_by_me=0
    marker=$(last_marker "$n")
    if [ -n "$marker" ] && [ "$(marker_kind "$marker")" = claim ]; then
        holder=$(marker_json "$marker")
        if [ "$(jq -r .session <<<"$holder")" = "${SESSION_ID:-manual}" ]; then
            held_by_me=1
        else
            die "carnet#$n is held by @$(jq -r .user <<<"$holder") · session $(jq -r .name <<<"$holder") — take it over with 'claim $n --steal' rather than releasing someone else's claim" 2
        fi
    fi
    if [ $held_by_me = 0 ] && ! ledger_has "$n"; then
        die "carnet#$n is not held by this session — nothing to release"
    fi

    local body
    body=$(mktemp)
    release_comment "$reason" > "$body"
    run gh issue edit "$n" -R "$TRACKER" --remove-label "$CLAIM_LABEL" --remove-assignee "$USER_LOGIN" >/dev/null
    run gh issue comment "$n" -R "$TRACKER" --body-file "$body" >/dev/null
    rm -f "$body"
    [ "$DRY_RUN" = 1 ] || ledger_drop "$n"
    say "🔓 carnet#$n released ($reason)"
}

cmd_release_all() { # <reason> <session-or-empty>
    local reason=${1:-done} sid=$2 f n rc=0
    if [ -n "$sid" ]; then
        # The SessionEnd hook path: the session's own process file may already be gone, so the
        # identity comes from the ledger it wrote when it claimed.
        f="$LEDGER_DIR/$sid.jsonl"
        [ -s "$f" ] || { say "session ${sid:0:8} holds nothing"; return 0; }
        local id
        id=$(head -1 "$f")
        SESSION_ID=$sid
        NAME=$(jq -r .name <<<"$id"); USER_LOGIN=$(jq -r .user <<<"$id"); SHORT_ID=${sid:0:8}
        SESSION_PID=$(jq -r .pid <<<"$id")
    else
        f=$(ledger_file) || { say "not inside a Claude Code session — nothing to release"; return 0; }
        [ -s "$f" ] || { say "this session holds nothing"; return 0; }
    fi
    # Each release runs in a subshell: die() exits the shell it is in, and one refused issue
    # must not stop the others from being released.
    for n in $(ledger_issues "$f"); do
        ( cmd_release "$n" "$reason" ) || { rc=1; warn "carnet#$n was not released"; }
    done
    return $rc
}

# ------------------------------------------------------------------ close
cmd_close() { # <n> <why> <commit>
    local n=$1 why=$2 commit=$3 issue marker holder commit_url=""
    [ -n "$why" ] || die "close needs --why \"<what resolved it>\" — a close without a reason is invisible debt"
    issue=$(issue_json "$n")
    [ "$(jq -r .state <<<"$issue")" = "OPEN" ] || die "carnet#$n is already closed"

    if [ -n "$commit" ]; then
        git -C "$REPO_ROOT" cat-file -e "$commit^{commit}" 2>/dev/null \
            || die "commit $commit is not in this repository (worktrees share objects, so a typo is the likely cause)"
        commit_url="https://github.com/$ORIGIN_SLUG/commit/$(git -C "$REPO_ROOT" rev-parse "$commit")"
    fi

    marker=$(last_marker "$n")
    if [ -n "$marker" ] && [ "$(marker_kind "$marker")" = claim ]; then
        holder=$(marker_json "$marker")
        if [ "$(jq -r .session <<<"$holder")" != "${SESSION_ID:-manual}" ]; then
            die "carnet#$n is held by @$(jq -r .user <<<"$holder") · session $(jq -r .name <<<"$holder") — 'claim $n' (or 'claim $n --steal') first, then close" 2
        fi
    fi

    local body
    body=$(mktemp)
    {
        printf '<!-- carnet-release %s -->\n' "$(identity_json | jq -c '. + {reason:"closed"}')"
        printf '✅ Closed by @%s · session **%s** (`%s`) · `%s` @ `%s`\n\n' "$USER_LOGIN" "$NAME" "$SHORT_ID" "$REPO_NAME" "$BRANCH"
        printf '**Why:** %s\n' "$why"
        [ -z "$commit_url" ] || printf '\n**Commit:** %s\n' "$commit_url"
    } > "$body"
    if has_label "$issue" "$CLAIM_LABEL" || [ "$(jq '.assignees | length' <<<"$issue")" != 0 ]; then
        run gh issue edit "$n" -R "$TRACKER" --remove-label "$CLAIM_LABEL" --remove-assignee "$USER_LOGIN" >/dev/null
    fi
    run gh issue comment "$n" -R "$TRACKER" --body-file "$body" >/dev/null
    run gh issue close "$n" -R "$TRACKER" >/dev/null
    rm -f "$body"
    [ "$DRY_RUN" = 1 ] || ledger_drop "$n"
    say "✅ carnet#$n closed · $(jq -r .title <<<"$issue")"
}

# ------------------------------------------------------------------ create
cmd_create() { # <title> <body> <body_file> <claim> labels...
    local title=$1 body=$2 body_file=$3 claim=$4; shift 4
    [ -n "$title" ] || die "create needs --title"

    # The whole point of a private register: an entry states where a defence is incomplete.
    local private
    private=$(gh repo view "$TRACKER" --json isPrivate -q .isPrivate 2>/dev/null || echo unknown)
    [ "$private" = true ] || die "tracker $TRACKER is not private (isPrivate=$private) — refusing to file"

    case "$title" in
        "["*) ;;
        *) title="[$PROJECT] $title" ;;
    esac

    local bf
    bf=$(mktemp)
    if [ -n "$body_file" ]; then
        [ -f "$body_file" ] || die "no such body file: $body_file"
        cat "$body_file" > "$bf"
    elif [ -n "$body" ]; then
        printf '%s\n' "$body" > "$bf"
    elif [ ! -t 0 ]; then
        cat > "$bf"
    fi
    [ -s "$bf" ] || die "create needs a body: where it is (file + symbol), what is incomplete, what the fix looks like"

    local args=(gh issue create -R "$TRACKER" --title "$title" --body-file "$bf" --label "$REPO_NAME")
    local l has_limitation=0
    for l in "$@"; do
        [ "$l" = "$REPO_NAME" ] && continue
        args+=(--label "$l")
        [ "$l" = limitation ] && has_limitation=1
    done

    local url n
    if [ "$DRY_RUN" = 1 ]; then
        run "${args[@]}"
        url="https://github.com/$TRACKER/issues/0"
    else
        url=$("${args[@]}") || die "gh issue create failed"
    fi
    rm -f "$bf"
    n=${url##*/}
    say "📝 $url"
    say "   $title"
    [ $has_limitation = 0 ] || say "   marker: LIMITATION(registre#$n): <name the limited item on this line>"
    [ "$claim" = 1 ] && [ "$n" != 0 ] && cmd_claim "$n" 0
    return 0
}

# ------------------------------------------------------------------ label
cmd_label() { # <n> [+label|-label|label]...
    local n=$1; shift
    [ $# -gt 0 ] || die "label needs at least one +label or -label"
    local a args=(gh issue edit "$n" -R "$TRACKER")
    for a in "$@"; do
        case "$a" in
            -*) args+=(--remove-label "${a#-}") ;;
            +*) args+=(--add-label "${a#+}") ;;
            *)  args+=(--add-label "$a") ;;
        esac
    done
    run "${args[@]}" >/dev/null
    say "🏷  carnet#$n: $*"
}

# ------------------------------------------------------------------ status
status_line() { # <n> <short>
    local n=$1 short=$2 issue marker holder title state line="" hint=""
    issue=$(issue_json "$n")
    title=$(jq -r .title <<<"$issue"); state=$(jq -r .state <<<"$issue")

    if [ "$state" != OPEN ]; then
        line="carnet#$n · closed"
    else
        marker=$(last_marker "$n")
        if [ -n "$marker" ] && [ "$(marker_kind "$marker")" = claim ]; then
            holder=$(marker_json "$marker")
            local hs hn hu hh hp ha hb live
            hs=$(jq -r .session <<<"$holder"); hn=$(jq -r .name <<<"$holder"); hu=$(jq -r .user <<<"$holder")
            hh=$(jq -r .host <<<"$holder");    hp=$(jq -r .pid <<<"$holder");  ha=$(jq -r .at <<<"$holder")
            hb=$(jq -r .branch <<<"$holder")
            if [ "$hs" = "${SESSION_ID:-manual}" ]; then
                line="carnet#$n · held by THIS session ($hn) · $hb · since $ha"
            else
                if holder_alive "$hh" "$hp" "$hs"; then live="running"; else
                    case $? in 1) live="session ended — stale, 'claim $n' takes it over" ;; *) live="other host" ;; esac
                fi
                line="carnet#$n · held by @$hu · session $hn (${hs:0:8}) on $hh [$live] · $hb · since $ha"
            fi
            has_label "$issue" "$CLAIM_LABEL" || line="$line · (label missing)"
        else
            line="carnet#$n · open · unclaimed"
            has_label "$issue" "$CLAIM_LABEL" && line="$line · (stale in-progress label, no claim marker)"
            hint="claim before the first edit: .build/skills/carnet/carnet.sh claim $n"
        fi
    fi

    if [ "$short" = 1 ]; then
        say "$line · $title"
    else
        say "$line"
        say "   $title"
        say "   $(jq -r .url <<<"$issue")"
        [ -z "$hint" ] || say "   → $hint"
    fi
}

cmd_status_all() {
    local nums n
    nums=$(gh issue list -R "$TRACKER" --label "$CLAIM_LABEL" --state open --limit 100 --json number -q '.[].number' 2>/dev/null) \
        || die "gh issue list failed"
    [ -n "$nums" ] || { say "no issue in $TRACKER is in progress"; return 0; }
    for n in $nums; do status_line "$n" 1; done
}

cmd_mine() { # <verify>
    local f n
    f=$(ledger_file) || die "not inside a Claude Code session"
    [ -s "$f" ] || { say "this session ($NAME) holds nothing"; return 0; }
    say "session $NAME ($SHORT_ID) holds:"
    for n in $(ledger_issues "$f"); do
        if [ "$1" = 1 ]; then status_line "$n" 1; else
            say "  carnet#$n · since $(jq -r --argjson n "$n" 'select(.kind == "claim" and .issue == $n) | .at' "$f")"
        fi
    done
}

# ------------------------------------------------------------------ main
sub=${1:-help}
[ $# -gt 0 ] && shift

case "$sub" in
    help|-h|--help) usage; exit 0 ;;
    label)
        [ $# -ge 1 ] || die "label needs an issue number"
        n=$1; shift
        [[ $n =~ ^[0-9]+$ ]] || die "not an issue number: $n"
        # "-bug" is a label to remove, not a flag, so this subcommand keeps its own arguments.
        rest=()
        for a in "$@"; do
            if [ "$a" = --dry-run ]; then DRY_RUN=1; else rest+=("$a"); fi
        done
        load_identity
        cmd_label "$n" ${rest[@]+"${rest[@]}"}
        exit 0 ;;
esac

steal=0 reason="" session="" all=0 why="" commit="" title="" body="" body_file="" claim=0 short=0 verify=0
labels=()
positional=()
while [ $# -gt 0 ]; do
    case "$1" in
        --steal)     steal=1 ;;
        --dry-run)   DRY_RUN=1 ;;
        --all)       all=1 ;;
        --claim)     claim=1 ;;
        --short)     short=1 ;;
        --verify)    verify=1 ;;
        --reason)    reason=${2:-}; shift ;;
        --session)   session=${2:-}; shift ;;
        --why)       why=${2:-}; shift ;;
        --commit)    commit=${2:-}; shift ;;
        --title)     title=${2:-}; shift ;;
        --body)      body=${2:-}; shift ;;
        --body-file) body_file=${2:-}; shift ;;
        --label)     labels+=("${2:-}"); shift ;;
        -h|--help)   usage; exit 0 ;;
        -*)          die "unknown flag: $1 (see: carnet.sh help)" ;;
        *)           positional+=("$1") ;;
    esac
    shift
done

issue_arg() {
    local n=${positional[0]:-}
    [ -n "$n" ] || die "$sub needs an issue number"
    [[ $n =~ ^[0-9]+$ ]] || die "not an issue number: $n"
    printf '%s' "$n"
}

load_identity

case "$sub" in
    claim)   cmd_claim "$(issue_arg)" "$steal" ;;
    release)
        if [ "$all" = 1 ]; then cmd_release_all "${reason:-done}" "$session"
        else cmd_release "$(issue_arg)" "${reason:-done}"; fi ;;
    close)   cmd_close "$(issue_arg)" "$why" "$commit" ;;
    create)  cmd_create "$title" "$body" "$body_file" "$claim" ${labels[@]+"${labels[@]}"} ;;
    status)
        if [ -n "${positional[0]:-}" ]; then status_line "$(issue_arg)" "$short"
        else cmd_status_all; fi ;;
    mine)    cmd_mine "$verify" ;;
    *)       die "unknown subcommand: $sub (see: carnet.sh help)" ;;
esac
