#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: retro — collects one period's evidence (commits, review findings, carnet, gates, vault) into a compact digest
# ABOUTME: The retro skill reasons over this digest; the script only counts and quotes, it never proposes
#
# Every Tier 1x gate began as an incident somebody noticed twice (carnet#662). This script is the
# noticing, made mechanical: it reads what the period left behind and prints the repeats. The
# proposals — which repeat becomes a gate, which standard is added or cut — are the skill's job
# (SKILL.md), because they are judgements and this script must stay deterministic: run twice over
# the same inputs, it prints the same digest.
#
# Sources, each optional except git; a source that cannot be read prints why, never a silent gap:
#   git        commits on --ref since --since: types, carnet refs, Reviewed-Standards trailers,
#              `review:` commits, files repeatedly fixed, gates and instruction files added/changed
#   carnet     issues and comments created in the window, over REST (`gh api` when present, else
#              curl through the agent proxy) — review-standards run records (carnet#660 thread),
#              recurring title terms, "Directive from JF" comments
#   vault      Work Log notes dated in the window: kinds, and the bullets under a "Lessons" heading
#   transcripts  local Claude Code transcripts for this repo modified in the window: tool mix,
#              errors, largest results, searches that came back empty, files read over and over
#
# Portability: macOS bash 3.2 (no associative arrays, no mapfile) and GNU/Linux alike; jq, awk,
# sed, git. Date arithmetic is `date -v` (BSD) falling back to `date -d` (GNU) — the BSD form is
# first because GNU rejects `-v` outright, so the chain cannot succeed wrongly. File age uses a
# `touch -t` reference file and `find -newer`, which both platforms share.
set -uo pipefail

DAYS=7
SINCE=""
REF="origin/main"
VAULT=""
OFFLINE=0
SNAPSHOT_IN=""
SNAPSHOT_OUT=""
TRANSCRIPT_DIRS=""
LIMIT=8

usage() {
    cat <<'EOF'
retro — the period's evidence, as a compact digest for the retro skill

  retro.sh [--days N | --since YYYY-MM-DD] [--ref <ref>] [options]

  --days N             window = the last N days (default 7)
  --since YYYY-MM-DD   window start, inclusive (UTC); overrides --days
  --ref <ref>          the branch whose history is read (default origin/main)
  --vault <dir>        dravr-vault checkout (default: ../dravr-vault beside the main checkout)
  --transcripts <dir>  a Claude Code projects dir to read (repeatable; default: every
                       ~/.claude*/projects/* whose name contains the repo's directory name)
  --offline            no network: the carnet section reads --carnet-from or is skipped
  --carnet-from <dir>  read issues.json / comments.json from a saved snapshot, not the tracker
  --save-carnet <dir>  write the fetched issues.json / comments.json there (reproducible digest)
  --limit N            rows per list (default 8)

Exit: 0 = digest printed · 2 = error (not a git checkout, bad argument, jq missing)
EOF
}

die() { printf 'retro: %s\n' "$1" >&2; exit 2; }

while [ $# -gt 0 ]; do
    case "$1" in
        --days) DAYS=${2:-}; shift 2 ;;
        --since) SINCE=${2:-}; shift 2 ;;
        --ref) REF=${2:-}; shift 2 ;;
        --vault) VAULT=${2:-}; shift 2 ;;
        --transcripts) TRANSCRIPT_DIRS="$TRANSCRIPT_DIRS${TRANSCRIPT_DIRS:+
}${2:-}"; shift 2 ;;
        --offline) OFFLINE=1; shift ;;
        --carnet-from) SNAPSHOT_IN=${2:-}; shift 2 ;;
        --save-carnet) SNAPSHOT_OUT=${2:-}; shift 2 ;;
        --limit) LIMIT=${2:-}; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; die "unknown argument: $1" ;;
    esac
done

command -v git >/dev/null 2>&1 || die "git is required"
command -v jq  >/dev/null 2>&1 || die "jq is required"
case $DAYS in ''|*[!0-9]*) die "--days takes a whole number of days" ;; esac
case $LIMIT in ''|*[!0-9]*) die "--limit takes a whole number" ;; esac

REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || die "run this inside a git checkout"
cd "$REPO_ROOT" || die "cannot enter $REPO_ROOT"
git rev-parse --verify -q "$REF^{commit}" >/dev/null || die "ref '$REF' does not resolve (git fetch origin?)"

if [ -z "$SINCE" ]; then
    SINCE=$(date -u -v-"$DAYS"d +%Y-%m-%d 2>/dev/null || date -u -d "$DAYS days ago" +%Y-%m-%d 2>/dev/null)
fi
case $SINCE in
    [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]) ;;
    *) die "--since must be YYYY-MM-DD (got '$SINCE')" ;;
esac
SINCE_TS="${SINCE}T00:00:00Z"
TODAY=$(date -u +%Y-%m-%d)

# The main checkout, not this worktree: the vault and the transcripts are named after it.
COMMON_DIR=$(cd "$(git rev-parse --git-common-dir)" && pwd -P)
MAIN_ROOT=${COMMON_DIR%/.git}
REPO_NAME=${MAIN_ROOT##*/}
[ -n "$VAULT" ] || VAULT="${MAIN_ROOT%/*}/dravr-vault"

TRACKER=${REGISTRE_TRACKER:-}
if [ -z "$TRACKER" ] && [ -f "$REPO_ROOT/registre.toml" ]; then
    TRACKER=$(sed -n 's/^tracker[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$REPO_ROOT/registre.toml" | head -1)
fi

WORK=$(mktemp -d "${TMPDIR:-/tmp}/retro.XXXXXX") || die "mktemp failed"
trap 'rm -rf "$WORK"' EXIT

say() { printf '%s\n' "$*"; }
cut_line() { awk -v n="${1:-160}" '{ s=$0; gsub(/[[:space:]]+/, " ", s); if (length(s) > n) s = substr(s, 1, n - 1) "…"; print s }'; }

# ------------------------------------------------------------------ git
# Commits with at least one parent: in a shallow clone the boundary commit has none and its diff
# is the whole tree, which would report every file in the repo as "added this week".
git log "$REF" --since="$SINCE_TS" --min-parents=1 --format='%x1e%h%x1f%s%x1f%(trailers:key=Reviewed-Standards,valueonly,separator=%x1d)%x1f%b' \
    > "$WORK/log.raw" 2>/dev/null || die "git log failed on $REF"
# One record per commit: sha \t type \t subject \t trailer \t carnet refs (space-separated)
awk 'BEGIN { RS = "\036"; FS = "\037" }
     NF >= 2 {
        sha = $1; subj = $2; tr = $3; body = $4
        gsub(/\n/, " ", tr); gsub(/\035/, " | ", tr)
        type = "other"
        if (match(subj, /^[a-z]+(\([^)]*\))?!?:/)) { type = substr(subj, 1, RLENGTH); sub(/\(.*$/, "", type); sub(/!?:$/, "", type) }
        refs = ""; text = subj " " body
        while (match(text, /carnet#[0-9]+/)) { r = substr(text, RSTART + 7, RLENGTH - 7); if (index(" " refs " ", " " r " ") == 0) refs = refs (refs == "" ? "" : " ") r; text = substr(text, RSTART + RLENGTH) }
        printf "%s\t%s\t%s\t%s\t%s\n", sha, type, subj, tr, refs
     }' "$WORK/log.raw" > "$WORK/commits.tsv"
COMMITS=$(wc -l < "$WORK/commits.tsv" | tr -d ' ')
TIP=$(git rev-parse --short "$REF")

# "covers <sha>…" in a later trailer clears an earlier commit (carnet#660's follow-up form).
awk -F'\t' '{ n = split($4, a, /[ |]+/); c = 0; for (i = 1; i <= n; i++) { if (a[i] == "covers") { c = 1; continue } if (c && a[i] ~ /^[0-9a-f]+$/) print a[i] } }' \
    "$WORK/commits.tsv" > "$WORK/covered.txt"

section_commits() {
    say "## 1. Commits on $REF since $SINCE — $COMMITS (tip $TIP)"
    say ""
    if [ "$COMMITS" = 0 ]; then say "No commits in the window."; say ""; return; fi
    printf 'By type: %s\n' "$(cut -f2 "$WORK/commits.tsv" | sort | uniq -c | sort -rn | awk '{ printf "%s%s %s", (NR > 1 ? ", " : ""), $2, $1 }')"
    local trailed review
    trailed=$(awk -F'\t' '$4 != ""' "$WORK/commits.tsv" | wc -l | tr -d ' ')
    review=$(awk -F'\t' '$2 == "review"' "$WORK/commits.tsv" | wc -l | tr -d ' ')
    # shellcheck disable=SC2016  # the backticks are markdown, not an expansion
    printf 'Reviewed-Standards trailer: %s of %s commits · `review:` fix-up commits: %s\n' "$trailed" "$COMMITS" "$review"
    say ""
    say "**feat/fix/perf/refactor landings with no trailer and no later \`covers\`** (review skipped):"
    # FILENAME, not NR == FNR: covered.txt is empty in any window without a `covers` trailer,
    # and NR == FNR would then read every commit as a covered sha and report none unreviewed.
    awk -F'\t' 'FILENAME == ARGV[1] { cov[$1] = 1; next }
                ($2 == "feat" || $2 == "fix" || $2 == "perf" || $2 == "refactor") && $4 == "" {
                    hit = 0; for (c in cov) if (index($1, c) == 1 || index(c, $1) == 1) hit = 1
                    if (!hit) print "- " $1 " " $3 }' "$WORK/covered.txt" "$WORK/commits.tsv" \
        | cut_line 150 > "$WORK/unreviewed.txt"
    if [ -s "$WORK/unreviewed.txt" ]; then
        printf '%s total\n' "$(wc -l < "$WORK/unreviewed.txt" | tr -d ' ')"; head -n "$LIMIT" "$WORK/unreviewed.txt"
    else say "- none"; fi
    say ""
    say "**\`review:\` commits** (what the reviewer had to fix after the implementer):"
    awk -F'\t' '$2 == "review" { print "- " $1 " " $3 }' "$WORK/commits.tsv" | cut_line 150 | head -n "$LIMIT"
    [ "$review" = 0 ] && say "- none"
    say ""
    say "**carnet issues referenced by 3+ commits** (churn — one issue landing in pieces, or fix after fix):"
    tr ' ' '\n' < <(cut -f5 "$WORK/commits.tsv") | grep -E '^[0-9]+$' | sort | uniq -c | sort -rn \
        | awk -v l="$LIMIT" '$1 >= 3 && n < l { printf "- carnet#%s ×%s\n", $2, $1; n++ }' > "$WORK/churn.txt"
    if [ -s "$WORK/churn.txt" ]; then cat "$WORK/churn.txt"; else say "- none"; fi
    say ""
    say "**Files touched by 3+ \`fix\` commits** (the same place breaking again):"
    git log "$REF" --since="$SINCE_TS" --min-parents=1 --format='%x1e%s' --name-only 2>/dev/null \
        | awk 'BEGIN { RS = "\036" } { n = split($0, l, "\n"); if (l[1] !~ /^fix(\(|!|:)/) next; for (i = 2; i <= n; i++) if (l[i] != "") print l[i] }' \
        | grep -v -E '(^|/)(Cargo\.(lock|toml)|bun\.lockb?|package\.json)$' \
        | sort | uniq -c | sort -rn | awk -v l="$LIMIT" '$1 >= 3 && n < l { printf "- %s ×%s\n", $2, $1; n++ }' > "$WORK/hot.txt"
    if [ -s "$WORK/hot.txt" ]; then cat "$WORK/hot.txt"; else say "- none"; fi
    say ""
}

# ------------------------------------------------------------------ gates and instruction surface
section_gates() {
    say "## 3. Gates added or changed"
    say ""
    git log "$REF" --since="$SINCE_TS" --min-parents=1 --diff-filter=A --format='%x1e%h' --name-only -- \
        'scripts/ci/check-*' '.github/workflows/*' 'scripts/ci/*.py' 2>/dev/null \
        | awk 'BEGIN { RS = "\036" } { n = split($0, l, "\n"); for (i = 2; i <= n; i++) if (l[i] != "" && l[i] !~ /\.test\.sh$/) print "- added " l[i] " (" l[1] ")" }' > "$WORK/gates.txt"
    git log "$REF" --since="$SINCE_TS" --min-parents=1 -p --format='%x1e%h' -- scripts/ci/pre-push-validate.sh 2>/dev/null \
        | awk 'BEGIN { RS = "\036" } { n = split($0, l, "\n"); for (i = 2; i <= n; i++) if (l[i] ~ /^\+[^+].*echo "Tier [0-9]/) { t = l[i]; sub(/^.*echo "/, "", t); sub(/".*$/, "", t); print "- new tier \"" t "\" (" l[1] ")" } }' >> "$WORK/gates.txt"
    if [ -s "$WORK/gates.txt" ]; then cat "$WORK/gates.txt"; else say "- no new gate file or pre-push tier in the window"; fi
    say ""
    say "Commits that changed an existing gate (a check tightened or a gap closed):"
    git log "$REF" --since="$SINCE_TS" --min-parents=1 --diff-filter=M --format='- %h %s' -- 'scripts/ci/check-*' scripts/ci/pre-push-validate.sh 2>/dev/null \
        | cut_line 150 > "$WORK/gatechg.txt"
    if [ -s "$WORK/gatechg.txt" ]; then printf '%s total\n' "$(wc -l < "$WORK/gatechg.txt" | tr -d ' ')"; head -n "$LIMIT" "$WORK/gatechg.txt"; else say "- none"; fi
    local pointer
    pointer=$(git log "$REF" --since="$SINCE_TS" --min-parents=1 --format=%h -- .build 2>/dev/null | wc -l | tr -d ' ')
    say "- .build submodule pointer moved $pointer time(s) (shared hooks/validation live there)"
    say ""
    say "Existing gates, for de-duplication before proposing one:"
    local g names=""
    for g in scripts/ci/check-*.sh scripts/ci/check-*.py; do
        case $g in *.test.sh) continue ;; esac
        [ -f "$g" ] || continue
        g=${g##*/check-}; names="$names${g%.*} "
    done
    printf '%s\n' "$names" | cut_line 2000
    say ""
}

section_instructions() {
    say "## 4. Instruction surface"
    say ""
    local agents_bytes budget
    agents_bytes=$(wc -c < AGENTS.md 2>/dev/null | tr -d ' ')
    # shellcheck disable=SC2016  # matching the literal ${AGENTS_MD_BUDGET:-N} default
    budget=$(sed -n 's/^BUDGET="\${AGENTS_MD_BUDGET:-\([0-9]*\)}"$/\1/p' scripts/ci/check-agents-md-budget.sh 2>/dev/null | head -1)
    if [ -n "$agents_bytes" ]; then
        if [ -n "$budget" ]; then
            printf -- '- AGENTS.md: %s of %s bytes (%s%%) — removing text is this step'"'"'s job (check-agents-md-budget.sh)\n' "$agents_bytes" "$budget" "$((agents_bytes * 100 / budget))"
        else printf -- '- AGENTS.md: %s bytes (no budget gate found)\n' "$agents_bytes"; fi
    fi
    [ -f docs/coding-standards.md ] && printf -- '- docs/coding-standards.md: %s bytes, %s sections\n' \
        "$(wc -c < docs/coding-standards.md | tr -d ' ')" "$(grep -c '^## ' docs/coding-standards.md)"
    say ""
    say "Instruction files changed in the window (+added/−removed lines):"
    git log "$REF" --since="$SINCE_TS" --min-parents=1 --numstat --format= -- AGENTS.md docs/coding-standards.md \
        '.claude/skills' '.agents/skills' '.claude/rules' 'scripts/ci/README.md' 2>/dev/null \
        | awk -F'\t' '$1 ~ /^[0-9]+$/ { f = $3; sub(/\/[^\/]*$/, "/…", f); if ($3 ~ /^(AGENTS\.md|docs\/)/) f = $3; a[f] += $1; d[f] += $2 }
                      END { for (k in a) printf "- %s +%d/−%d\n", k, a[k], d[k] }' | sort > "$WORK/instr.txt"
    if [ -s "$WORK/instr.txt" ]; then head -n "$((LIMIT * 2))" "$WORK/instr.txt"; else say "- none"; fi
    say ""
    # A skill nobody points at is either reached by its description alone or not at all; the
    # transcripts section adds whether a session actually invoked it.
    : > "$WORK/skills.tsv"
    local d name bytes refs
    for d in .claude/skills/*; do
        [ -f "$d/SKILL.md" ] || continue
        name=${d##*/}
        bytes=$(wc -c < "$d/SKILL.md" | tr -d ' ')
        refs=$( { cat AGENTS.md docs/*.md 2>/dev/null; for f in .claude/skills/*/SKILL.md; do [ "$f" = "$d/SKILL.md" ] || cat "$f"; done; cut -f3 "$WORK/commits.tsv"; } \
                | grep -c -F -- "$name")
        printf '%s\t%s\t%s\n' "$name" "$bytes" "$refs" >> "$WORK/skills.tsv"
    done
    printf 'Skills: %s, %s bytes of SKILL.md in total.\n' "$(wc -l < "$WORK/skills.tsv" | tr -d ' ')" \
        "$(awk -F'\t' '{ s += $2 } END { print s + 0 }' "$WORK/skills.tsv")"
    say "Skills named nowhere else (AGENTS.md, docs/, other skills, this window's commit subjects):"
    awk -F'\t' '$3 == 0 { printf "%s (%s B) ", $1, $2 }' "$WORK/skills.tsv" | cut_line 1500
    say ""
}

# ------------------------------------------------------------------ carnet
api_get() { # <path> — REST only: GraphQL is refused by the agent proxy (see carnet.sh)
    if command -v gh >/dev/null 2>&1; then
        gh api "$1" 2>/dev/null
    else
        curl -fsS -H 'Accept: application/vnd.github+json' "https://api.github.com/$1" 2>/dev/null
    fi
}

# Page by number, never by the Link header: the proxy refuses the repositories/{id} paths that
# header names, so `gh api --paginate` dies on page two. Past 20 pages the fetch fails with that
# reason instead of returning the first 2000 records as if they were the whole window.
fetch_all() { # <path with ?query> <out>
    local page=1 n
    : > "$2.pages"
    while :; do
        [ "$page" -le 20 ] || { printf 'more than 2000 records at %s, narrow the window; ' "${1%%\?*}"; return 1; }
        api_get "$1&per_page=100&page=$page" > "$2.page" || return 1
        n=$(jq 'length' "$2.page" 2>/dev/null) || return 1
        cat "$2.page" >> "$2.pages"
        [ "$n" -lt 100 ] && break
        page=$((page + 1))
    done
    jq -s 'add // []' "$2.pages" > "$2"
}

load_carnet() {
    if [ -n "$SNAPSHOT_IN" ]; then
        [ -f "$SNAPSHOT_IN/issues.json" ] && [ -f "$SNAPSHOT_IN/comments.json" ] || { echo "snapshot $SNAPSHOT_IN lacks issues.json/comments.json"; return 1; }
        cp "$SNAPSHOT_IN/issues.json" "$SNAPSHOT_IN/comments.json" "$WORK/" || return 1
        return 0
    fi
    [ "$OFFLINE" = 0 ] || { echo "--offline and no --carnet-from"; return 1; }
    [ -n "$TRACKER" ] || { echo "no tracker (registre.toml or REGISTRE_TRACKER)"; return 1; }
    fetch_all "repos/$TRACKER/issues?state=all&since=$SINCE_TS" "$WORK/issues.json" || { echo "fetching $TRACKER issues failed"; return 1; }
    fetch_all "repos/$TRACKER/issues/comments?since=$SINCE_TS" "$WORK/comments.json" || { echo "fetching $TRACKER comments failed"; return 1; }
    if [ -n "$SNAPSHOT_OUT" ]; then
        mkdir -p "$SNAPSHOT_OUT" && cp "$WORK/issues.json" "$WORK/comments.json" "$SNAPSHOT_OUT/"
    fi
}

# Themes a review-standards finding is filed under. A label matches every theme whose pattern it
# hits, so "Abstractions / single source" counts once under single-source. Edit this table, not
# the matching code, when the standards grow a section. Order is display order only.
# shellcheck disable=SC2016  # `pub` is a literal in a finding label
THEMES='comments-true	comment|evergreen|history|prose|say what the code does|docs? follow
single-source	single source|duplicat|written twice|second parser|one source|abstractions
pub-visibility	adr-027|`pub`|visibility|test placement
tests-coverage	tests? cover|coverage|no test|test gap|tests at|^ *tests *$
tests-content	tautolog|content, not success|concrete content|brittle|mock|file read says why|tests: a file
backend-twin	migrations_pg|postgres|sqlite|backend|twin
style-lints	style|lint|rustfmt|fmt|import|clones?|^ *rust *$
ci-gates	ci gate|gates and scripts|size ratchet|file size
correctness-spec	correctness|spec
declared-unconsumed	consume|phantom|new functions|advertised capability|stubs?|fabricated
error-handling	error handling
security	security|url safety|tenan|auth|template/query
numeric-input	numeric|pagination|input domain|bounds
dependencies	dependenc|feature flag
mirrored-strings	mirrored|contremaitre|locale'

section_review_findings() {
    say "## 2. review-standards findings, by theme"
    say ""
    [ -f "$WORK/comments.json" ] || { say "(carnet unavailable: $CARNET_WHY)"; say ""; return; }
    # Run records are comments that say they are one; a label is the bold lead of a bullet or the
    # first cell of a table row, and a row counts only when it reports something found.
    jq -r --arg since "$SINCE_TS" '
        .[] | select(.created_at >= $since)
            | select(.body | test("review-standards run|standards review|pilot record|findings by (standards )?section"; "i"))
            | (.issue_url | split("/") | last) as $n | (.created_at[0:10]) as $d | (.id | tostring) as $id
            | .body | split("\n")[]
            | select(test("^\\s{0,1}[-*] \\*\\*|^\\|[^-|]"))
            | select(test("nothing found|found nothing|checked|^\\|\\s*(standards )?section|open question|standards-related|post-squash|rules checked"; "i") | not)
            | "\($n)\t\($d)\t\($id)\t\(.)"' "$WORK/comments.json" > "$WORK/findings.raw" 2>/dev/null
    jq -r --arg since "$SINCE_TS" '[.[] | select(.created_at >= $since) | select(.body | test("review-standards run|standards review|pilot record|findings by (standards )?section"; "i"))] | length' \
        "$WORK/comments.json" > "$WORK/runs.n" 2>/dev/null
    awk -F'\t' '{
        line = $4; label = line
        if (line ~ /^\|/) { split(line, c, "|"); label = c[2]; rest = c[3]; if (rest ~ /^[[:space:]]*(0|none|—|-)?[[:space:]]*$/) next }
        else { sub(/^[[:space:]]*[-*] \*\*/, "", label); sub(/\*\*.*$/, "", label) }
        printf "%s\t%s\t%s\t%s\t%s\n", $1, $2, tolower(label), line, $3 }' "$WORK/findings.raw" > "$WORK/findings.tsv"
    printf '%s review runs recorded in the window; %s finding rows.\n\n' "$(cat "$WORK/runs.n")" "$(wc -l < "$WORK/findings.tsv" | tr -d ' ')"
    say "| theme | runs | rows | caught twice |"
    say "|---|---|---|---|"
    printf '%s\n' "$THEMES" > "$WORK/themes.tsv"
    awk -F'\t' 'NR == FNR { th[++k] = $1; re[k] = $2; next }
        { for (i = 1; i <= k; i++) if ($3 ~ re[i]) { rows[i]++; key = i SUBSEP $5; if (!(key in seen)) { seen[key] = 1; runs[i]++ } } }
        END { for (i = 1; i <= k; i++) if (rows[i]) printf "| %s | %d | %d | %s |\n", th[i], runs[i], rows[i], (runs[i] >= 2 ? "**yes**" : "") }' \
        "$WORK/themes.tsv" "$WORK/findings.tsv"
    say ""
    say "Unclassified finding labels (extend THEMES in retro.sh if one recurs):"
    awk -F'\t' 'NR == FNR { re[++k] = $2; next } { m = 0; for (i = 1; i <= k; i++) if ($3 ~ re[i]) m = 1; if (!m) print "- " $3 }' \
        "$WORK/themes.tsv" "$WORK/findings.tsv" | sort -u | cut_line 100 | head -n "$LIMIT"
    say ""
    say "Evidence for every theme caught twice (carnet#issue date — the row as recorded):"
    awk -F'\t' 'NR == FNR { th[++k] = $1; re[k] = $2; next }
        { rec[++r] = $0; for (i = 1; i <= k; i++) if ($3 ~ re[i]) { key = i SUBSEP $5; if (!(key in seen)) { seen[key] = 1; runs[i]++ } } }
        END { for (i = 1; i <= k; i++) if (runs[i] >= 2) { print "### " th[i]
                for (j = 1; j <= r; j++) { split(rec[j], f, "\t"); if (f[3] ~ re[i]) { l = f[4]; sub(/^[[:space:]]*[-*][[:space:]]*/, "", l); print "- #" f[1] " " f[2] " — " l } } } }' \
        "$WORK/themes.tsv" "$WORK/findings.tsv" | cut_line 220 | awk -v l="$LIMIT" '/^###/ { n = 0; print; next } n < l { print; n++ }'
    say ""
}

section_carnet() {
    say "## 5. Carnet in the window"
    say ""
    [ -f "$WORK/issues.json" ] || { say "(carnet unavailable: $CARNET_WHY)"; say ""; return; }
    jq -r --arg since "$SINCE_TS" '
        [.[] | select(.pull_request | not)] as $all
        | ($all | map(select(.created_at >= $since))) as $new
        | ($all | map(select(.closed_at != null and .closed_at >= $since))) as $closed
        | "Created: \($new | length) · closed: \($closed | length) · still open of those created: \($new | map(select(.state == "open")) | length)",
          "Labels on created issues: \([$new[].labels[].name] | group_by(.) | map("\(.[0]) \(length)") | sort_by(-(split(" ")[-1] | tonumber)) | join(", "))"
    ' "$WORK/issues.json"
    say ""
    say "Recurring title terms (in 3+ issues created in the window):"
    jq -r --arg since "$SINCE_TS" '.[] | select(.pull_request | not) | select(.created_at >= $since) | .title' "$WORK/issues.json" \
        | sed 's/^\[[^]]*\][[:space:]]*//' | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9#\n-' ' ' \
        | awk '{ delete s; for (i = 1; i <= NF; i++) if (length($i) >= 4 && !($i in s)) { s[$i] = 1; print $i } }' \
        | grep -v -x -E 'with|from|that|this|when|what|into|than|then|they|them|their|there|have|does|done|only|each|every|still|never|over|after|before|while|about|which|where|without|instead|should|could|would|platform|dravr|carnet' \
        | sort | uniq -c | sort -rn | awk -v l="$((LIMIT * 2))" '$1 >= 3 && n < l { printf "%s%s ×%s", (n ? ", " : ""), $2, $1; n++ } END { print "" }'
    say ""
    say "Issues filed about red CI or a broken gate:"
    jq -r --arg since "$SINCE_TS" '.[] | select(.pull_request | not) | select(.created_at >= $since)
        | select(.title | test("\\bCI\\b|red on main|gate|flak|pre-push|clippy"; "i")) | "- #\(.number) [\(.state)] \(.title)"' "$WORK/issues.json" \
        | cut_line 160 | head -n "$LIMIT"
    say ""
    say "Directives from JF in the window (requirements that lived only in a comment):"
    [ -f "$WORK/comments.json" ] || { say "(comments unavailable: $CARNET_WHY)"; say ""; return; }
    jq -r --arg since "$SINCE_TS" '.[] | select(.created_at >= $since) | select(.body | test("Directive from JF"; "i"))
        | "- #\(.issue_url | split("/") | last) \(.created_at[0:10]) — \(.body | gsub("\\*"; "") | split("\n") | map(select(test("\\S"))) | .[0])"' "$WORK/comments.json" \
        | cut_line 220 | head -n "$LIMIT"
    say ""
}

# ------------------------------------------------------------------ vault
section_vault() {
    say "## 6. Vault Work Log dated in the window"
    say ""
    [ -d "$VAULT/Work Log" ] || { say "(vault not found at $VAULT — pass --vault)"; say ""; return; }
    : > "$WORK/notes.tsv"
    local f date kind
    while IFS= read -r f; do
        date=$(sed -n '1,20s/^date:[[:space:]]*\([0-9-]*\).*/\1/p' "$f" | head -1)
        [ -n "$date" ] || continue
        { [ "$date" \> "$SINCE" ] || [ "$date" = "$SINCE" ]; } || continue
        kind=$(sed -n '1,20s/^kind:[[:space:]]*\([a-z-]*\).*/\1/p' "$f" | head -1)
        printf '%s\t%s\t%s\n' "$date" "${kind:-?}" "$f" >> "$WORK/notes.tsv"
    done < <(find "$VAULT/Work Log" -name '*.md' -type f 2>/dev/null)
    printf '%s notes. By kind: %s\n\n' "$(wc -l < "$WORK/notes.tsv" | tr -d ' ')" \
        "$(cut -f2 "$WORK/notes.tsv" | sort | uniq -c | sort -rn | awk '{ printf "%s%s %s", (NR > 1 ? ", " : ""), $2, $1 }')"
    say "Post-mortems, audits and decisions:"
    sort "$WORK/notes.tsv" | awk -F'\t' '$2 ~ /^(post-mortem|audit|decision)$/ { n = $3; sub(/^.*\//, "", n); sub(/\.md$/, "", n); print "- " $1 " " $2 ": " n }' | head -n "$LIMIT"
    say ""
    say "Lessons recorded (bullets under a lessons / follow-up heading):"
    sort "$WORK/notes.tsv" | cut -f3 | while IFS= read -r f; do
        awk -v t="${f##*/}" 'BEGIN { sub(/\.md$/, "", t) }
             /^#+ / { on = ($0 ~ /[Ll]esson|[Ff]ollow-up|[Ww]hat we change|[Dd]ifferently/); next }
             on && /^[-*] / && n < 3 { sub(/^[-*] /, ""); print "- (" t ") " $0; n++ }' "$f"
    done | cut_line 220 | head -n "$((LIMIT * 2))"
    say ""
}

# ------------------------------------------------------------------ transcripts
section_transcripts() {
    say "## 7. Session transcripts (this machine only)"
    say ""
    local dirs d
    dirs=$TRANSCRIPT_DIRS
    if [ -z "$dirs" ]; then
        for d in "$HOME"/.claude*/projects/*"$REPO_NAME"*; do [ -d "$d" ] && dirs="$dirs${dirs:+
}$d"; done
    fi
    [ -n "$dirs" ] || { say "(no transcripts for $REPO_NAME under ~/.claude*/projects)"; say ""; return; }
    # TZ=UTC: touch -t reads local time, and the window starts at midnight UTC.
    TZ=UTC touch -t "$(printf '%s' "$SINCE" | tr -d '-')0000" "$WORK/since.ref" 2>/dev/null || { say "(cannot build the window reference file)"; say ""; return; }
    : > "$WORK/tlist"
    printf '%s\n' "$dirs" | while IFS= read -r d; do
        [ -d "$d" ] && find "$d" -name '*.jsonl' -type f -newer "$WORK/since.ref" 2>/dev/null
    done | sort -u > "$WORK/tlist"
    local files
    files=$(wc -l < "$WORK/tlist" | tr -d ' ')
    [ "$files" -gt 0 ] || { say "(no transcript modified since $SINCE)"; say ""; return; }
    : > "$WORK/tools.tsv"
    while IFS= read -r f; do
        jq -r -s '
            [.[] | select(.type == "assistant") | .message.content[]? | select(type == "object" and .type == "tool_use")
                 | {id, name, key: ((.input.skill // .input.file_path // .input.pattern // ((.input.command // "") | sub("^\\s*cd [^&;]*(&&|;)\\s*"; "") | split(" ") | .[0:3] | join(" "))) | tostring | gsub("[\t\n]"; " ") | .[0:90])}] as $uses
            | ([.[] | select(.type == "user") | .message.content[]? | select(type == "object" and .type == "tool_result")
                 | {key: .tool_use_id, value: {err: (.is_error // false), size: (.content | tostring | length),
                                              empty: ((.content | tostring) | test("^\"?(No files found|No matches found|Found 0 )"))}}] | from_entries) as $r
            | $uses[] | [.name, .key, ($r[.id].err // false), ($r[.id].size // 0), ($r[.id].empty // false)] | @tsv' "$f" 2>/dev/null >> "$WORK/tools.tsv"
    done < "$WORK/tlist"
    printf '%s transcript files (sessions and subagents), %s tool calls.\n\n' "$files" "$(wc -l < "$WORK/tools.tsv" | tr -d ' ')"
    say "| tool | calls | errors | result KB |"
    say "|---|---|---|---|"
    awk -F'\t' '{ c[$1]++; if ($3 == "true") e[$1]++; b[$1] += $4 } END { for (k in c) printf "%d\t| %s | %d | %d | %d |\n", b[k], k, c[k], e[k], b[k] / 1024 }' "$WORK/tools.tsv" \
        | sort -rn | cut -f2 | head -n "$LIMIT"
    say ""
    say "Largest single results (token cost):"
    awk -F'\t' '{ printf "%d\t- %s %s — %d KB\n", $4, $1, $2, $4 / 1024 }' "$WORK/tools.tsv" | sort -rn | cut -f2 | head -n 5 | cut_line 160
    say ""
    say "Calls that errored, by tool and command:"
    awk -F'\t' '$3 == "true" { print $1 " " $2 }' "$WORK/tools.tsv" | sort | uniq -c | sort -rn | head -n "$LIMIT" | cut_line 140
    say ""
    say "Searches that came back empty (where the agent looked and the thing was not):"
    awk -F'\t' '$5 == "true" { print $1 " " $2 }' "$WORK/tools.tsv" | sort | uniq -c | sort -rn | head -n "$LIMIT" | cut_line 140
    say ""
    say "Files read 3+ times (a navigation hint or a missing summary):"
    awk -F'\t' '$1 == "Read" { print $2 }' "$WORK/tools.tsv" | sort | uniq -c | sort -rn | awk '$1 >= 3' | head -n "$LIMIT" | cut_line 140
    say ""
    say "Skills invoked:"
    awk -F'\t' '$1 == "Skill" { print $2 }' "$WORK/tools.tsv" | sort | uniq -c | sort -rn | awk '{ printf "%s%s ×%s", (NR > 1 ? ", " : ""), $2, $1 } END { print "" }'
    say ""
}

# ------------------------------------------------------------------ main
CARNET_WHY=$(load_carnet) || true
[ -f "$WORK/issues.json" ] || CARNET_WHY=${CARNET_WHY:-unavailable}

say "# Retro digest — $REPO_NAME, $SINCE → $TODAY"
say ""
say "Window: $REF since $SINCE (UTC) · tracker: ${TRACKER:-none} · vault: $VAULT"
say ""
section_commits
section_review_findings
section_gates
section_instructions
section_carnet
section_vault
section_transcripts
