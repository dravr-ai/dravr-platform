#!/usr/bin/env bash
# ABOUTME: Fixture test for sweep-landed-branches.sh — which branches count as landed, which survive a sweep
# ABOUTME: Squash shapes, partial landings, renames, modes, a blobless clone, and a sweep against a fake gh
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# This script deletes branches, so the expensive mistake is a false "landed".
# Every way a squash can differ from its branch is pinned as landed, and every
# way a branch can still hold work main lacks is pinned as kept — including a
# rename whose delete half never landed and a mode-only change. The sweep runs
# end to end against local bare remotes and a fake gh, so the exclusions
# (protected, open pull request, too young, dependabot, default) and the
# fail-closed exits are exercised by the real code path, not restated.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/sweep-landed-branches.sh}"

failures=0
pass() { echo "  ✅ $1"; }
fail() {
    echo "  ❌ $1"
    failures=$((failures + 1))
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Fixtures never read the developer's git config, and commit on a fixed clock
# two days back so the sweep's age floor sees them as old.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.com
export GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.com
OLD_DATE="$(( $(date +%s) - 48 * 3600 )) +0000"
export GIT_AUTHOR_DATE="$OLD_DATE" GIT_COMMITTER_DATE="$OLD_DATE"

# repo <name> -> a repo on main with three files
repo() {
    local r="$TMP/$1"
    git init -q -b main "$r"
    printf 'a1\na2\na3\na4\na5\na6\na7\na8\na9\na10\n' >"$r/a.txt"
    printf 'b1\nb2\n' >"$r/b.txt"
    git -C "$r" add -A && git -C "$r" commit -qm base
    echo "$r"
}

commit_all() { git -C "$1" add -A && git -C "$1" commit -qm "$2"; }

# expect_landed <label> <dir> <branch> <want-exit> [<want-output-substring>]
expect_landed() {
    local label="$1" dir="$2" branch="$3" want="$4" text="${5:-}" got=0
    "$UNDER_TEST" landed "$dir" main "$branch" >"$TMP/out.txt" 2>&1 || got=$?
    if [ "$got" = "$want" ] && { [ -z "$text" ] || grep -qF -- "$text" "$TMP/out.txt"; }; then
        pass "$label"
    else
        fail "$label"
        echo "      exit: got $got, expected $want${text:+, output containing '$text'}"
        sed 's/^/      /' "$TMP/out.txt"
    fi
}

echo "sweep-landed-branches.sh fixture tests"
echo " landed:"

# A squash: the branch's two commits land as one new commit on main.
r=$(repo squash)
git -C "$r" checkout -qb work
sed -i.bak 's/^a2$/A2/' "$r/a.txt" && rm "$r/a.txt.bak" && commit_all "$r" one
echo c >"$r/c.txt" && commit_all "$r" two
git -C "$r" checkout -q main && git -C "$r" merge -q --squash work && git -C "$r" commit -qm squash
expect_landed "squash-merged branch is landed" "$r" work 0 "every changed path"

# Main edits the same line after the squash: merge-tree conflicts, content landed.
sed -i.bak 's/^A2$/AA2/' "$r/a.txt" && rm "$r/a.txt.bak" && commit_all "$r" later
expect_landed "main rewriting a landed line afterwards does not un-land it" "$r" work 0

# The squash carried an extra hunk in the same file: the blob never matched,
# but every hunk of the branch is on main.
r=$(repo tweak)
git -C "$r" checkout -qb work
sed -i.bak 's/^a2$/A2/' "$r/a.txt" && rm "$r/a.txt.bak" && commit_all "$r" one
git -C "$r" checkout -q main
sed -i.bak -e 's/^a2$/A2/' -e 's/^a9$/A9/' "$r/a.txt" && rm "$r/a.txt.bak" && commit_all "$r" squash-plus
expect_landed "squash with an extra hunk on main is landed by its hunks" "$r" work 0

# A commit after the squash that main never got.
r=$(repo extra)
git -C "$r" checkout -qb work
echo c >"$r/c.txt" && commit_all "$r" one
git -C "$r" checkout -q main && git -C "$r" merge -q --squash work && git -C "$r" commit -qm squash
git -C "$r" checkout -q work && echo more >>"$r/b.txt" && commit_all "$r" after && git -C "$r" checkout -q main
expect_landed "a commit main never got keeps the branch" "$r" work 1 "not landed: b.txt"

# Two files changed, one landed.
r=$(repo partial)
git -C "$r" checkout -qb work
echo c >"$r/c.txt" && echo d >"$r/d.txt" && commit_all "$r" two-files
git -C "$r" checkout -q main && echo c >"$r/c.txt" && commit_all "$r" half
expect_landed "half a branch is not landed" "$r" work 1 "not landed: d.txt"

# A rename: both halves must be on main.
r=$(repo rename)
git -C "$r" checkout -qb work && git -C "$r" mv b.txt bee.txt && commit_all "$r" rename
git -C "$r" checkout -q main && git -C "$r" mv b.txt bee.txt && commit_all "$r" squash
expect_landed "a landed rename is landed" "$r" work 0
r=$(repo rename-half)
git -C "$r" checkout -qb work && git -C "$r" mv b.txt bee.txt && commit_all "$r" rename
git -C "$r" checkout -q main && cp "$r/b.txt" "$r/bee.txt" && commit_all "$r" copy-only
expect_landed "a rename whose delete never landed is kept" "$r" work 1 "not landed: b.txt"

# A deletion main also made.
r=$(repo delete)
git -C "$r" checkout -qb work && git -C "$r" rm -q b.txt && commit_all "$r" drop
git -C "$r" checkout -q main && git -C "$r" rm -q b.txt && commit_all "$r" squash
expect_landed "a landed deletion is landed" "$r" work 0

# A mode-only change main does not have: same object, different mode.
r=$(repo mode)
git -C "$r" checkout -qb work && chmod +x "$r/b.txt" && commit_all "$r" exec
git -C "$r" checkout -q main && echo e >"$r/e.txt" && commit_all "$r" unrelated
expect_landed "a mode change main lacks is kept" "$r" work 1 "not landed: b.txt"

# A path that is a glob pattern is matched literally: read as a pattern,
# `[g].md` matches main's `g.md`, whose content is the branch's.
r=$(repo glob)
git -C "$r" checkout -qb work && echo G >"$r/[g].md" && commit_all "$r" glob
git -C "$r" checkout -q main && echo G >"$r/g.md" && commit_all "$r" lookalike
expect_landed "a glob-shaped path is not confused with the file its pattern matches" "$r" work 1 "not landed: [g].md"

# Fast path, and the two refusals.
r=$(repo ancestor)
git -C "$r" branch old && echo e >"$r/e.txt" && commit_all "$r" ahead
expect_landed "a branch main already contains is landed" "$r" old 0 "ancestor"
git -C "$r" checkout -q --orphan alien && git -C "$r" rm -rqf . && echo x >"$r/x" && commit_all "$r" alien
git -C "$r" checkout -q main
expect_landed "a branch sharing no history is kept" "$r" alien 1 "no shared history"
expect_landed "an unknown ref is an error, not a verdict" "$r" no-such-branch 2

# A git failure inside the check is an error, never a verdict. An unread path
# list is not an empty one, and two unread tree entries are not two "absent"
# ones: read as a verdict, either calls this unlanded branch landed, and a
# landed branch is deleted. A wrapper fails git on one named argument.
REAL_GIT="$(command -v git)"
mkdir -p "$TMP/failgit"
cat >"$TMP/failgit/git" <<FAILGIT
#!/usr/bin/env bash
for a in "\$@"; do
    if [ "\$a" = "\$FAIL_GIT_ARG" ]; then echo "fixture: git \$a refuses" >&2; exit 1; fi
done
exec "$REAL_GIT" "\$@"
FAILGIT
chmod +x "$TMP/failgit/git"
r=$(repo gitfail)
git -C "$r" checkout -qb work && echo d >"$r/d.txt" && commit_all "$r" d
git -C "$r" checkout -q main && echo e >"$r/e.txt" && commit_all "$r" unrelated
for arg in --name-only ls-tree read-tree; do
    PATH="$TMP/failgit:$PATH" FAIL_GIT_ARG="$arg" \
        expect_landed "a failing git $arg is an error, not a verdict" "$r" work 2 "failed"
done

# The sweep reads a blobless bare clone: rule 2 must fetch blobs lazily.
r=$(repo blobless)
git -C "$r" config uploadpack.allowFilter true
git -C "$r" checkout -qb work && sed -i.bak 's/^a2$/A2/' "$r/a.txt" && rm "$r/a.txt.bak" && commit_all "$r" one
git -C "$r" checkout -q main
sed -i.bak -e 's/^a2$/A2/' -e 's/^a9$/A9/' "$r/a.txt" && rm "$r/a.txt.bak" && commit_all "$r" squash-plus
git -C "$r" checkout -q -b gone && echo z >"$r/z.txt" && commit_all "$r" z && git -C "$r" checkout -q main
git clone -q --bare --filter=blob:none "file://$r" "$TMP/blobless.git"
expect_landed "a blobless bare clone resolves hunks through lazy fetches" "$TMP/blobless.git" work 0
expect_landed "a blobless bare clone still keeps unlanded work" "$TMP/blobless.git" gone 1 "not landed: z.txt"

# ---------------------------------------------------------------- sweep
echo " sweep:"

# Remotes live under $TMP/remotes/<repo>.git; git reaches them through an
# env-scoped rewrite of the https URL the script builds, which survives the
# script's GIT_CONFIG_GLOBAL=/dev/null because it is not a config file.
mkdir -p "$TMP/remotes" "$TMP/bin"
src=$(repo sweep-src)
git -C "$src" checkout -qb landed && echo l >"$src/l.txt" && commit_all "$src" l
git -C "$src" checkout -q main && echo l >"$src/l.txt" && commit_all "$src" squash-l
for b in protected-one pr-head pr-base fork-branch dependabot/cargo/x; do git -C "$src" branch "$b" landed; done
git -C "$src" checkout -qb unlanded && echo u >"$src/u.txt" && commit_all "$src" u
git -C "$src" checkout -q main
# young landed an hour ago: content on main, tip inside the age floor.
NOW_DATE="$(( $(date +%s) - 3600 )) +0000"
git -C "$src" checkout -qb young && echo y >"$src/y.txt"
GIT_COMMITTER_DATE="$NOW_DATE" commit_all "$src" y
git -C "$src" checkout -q main && echo y >"$src/y.txt"
GIT_COMMITTER_DATE="$NOW_DATE" commit_all "$src" squash-y
git clone -q --bare "$src" "$TMP/remotes/repo-a.git"
git -C "$TMP/remotes/repo-a.git" config uploadpack.allowFilter true

# repo-p is private: one landed branch, one holding work, both named so that
# any leak into a redacted log is greppable.
priv=$(repo sweep-priv)
git -C "$priv" checkout -qb secret-landed && echo s >"$priv/secret-path.txt" && commit_all "$priv" s
git -C "$priv" checkout -q main && echo s >"$priv/secret-path.txt" && commit_all "$priv" squash-s
git -C "$priv" checkout -qb secret-work && echo w >"$priv/secret-work-path.txt" && commit_all "$priv" w
git -C "$priv" checkout -q main
git clone -q --bare "$priv" "$TMP/remotes/repo-p.git"
git -C "$TMP/remotes/repo-p.git" config uploadpack.allowFilter true
# repo.t and -erepo-a are private repositories inside a private-ci.sh window:
# public right now, not declared permanently public, so a public log must not
# name them. GitHub allows a dot and a leading hyphen in a repository name, so
# each is matched against --itemize as an exact literal, never as a pattern or
# an option: `-erepo-a` read by grep is `-e repo-a`, a declared name.
for r in repo.t -erepo-a; do
    git clone -q --bare "$priv" "$TMP/remotes/$r.git"
    git -C "$TMP/remotes/$r.git" config uploadpack.allowFilter true
done

# The fake gh answers the five reads the sweep makes, per repository, with
# GitHub-shaped JSON run through the script's own --jq filter, so the filters
# are under test too: an archived repository must be dropped, and a pull
# request's base is held while a fork's head of the same name is not.
# FAKE_GH_FAIL makes the org listing fail, FAKE_GH_EMPTY makes it list
# nothing, and FAKE_GH_PRIVATE adds the private repo-p; repo-gone, whose
# visibility the listing does not report and which lists a branch but has no
# remote, so its clone fails with an error that names it; and repo.t and
# -erepo-a, public for the length of a window. The fake reads a remote with
# --git-dir, so a run under safe.bareRepository=explicit still lists it.
cat >"$TMP/bin/gh" <<'FAKE'
#!/usr/bin/env bash
[ "$1" = auth ] && exit 0
filter=.
for ((i = 1; i <= $#; i++)); do
    if [ "${!i}" = --jq ]; then j=$((i + 1)); filter=${!j}; fi
done
name=${2#repos/*/}
name=${name%%/*}
case "$2" in
    orgs/*/repos*)
        [ -n "${FAKE_GH_FAIL:-}" ] && exit 1
        if [ -n "${FAKE_GH_EMPTY:-}" ]; then jq -r "$filter" <<<'[]'; exit 0; fi
        json='[{"name":"repo-a","default_branch":"main","private":false,"archived":false},
               {"name":"repo-old","default_branch":"main","private":false,"archived":true}'
        [ -n "${FAKE_GH_PRIVATE:-}" ] && json+=',{"name":"repo-p","default_branch":"main","private":true,"archived":false},
               {"name":"repo-gone","default_branch":"main","archived":false},
               {"name":"repo.t","default_branch":"main","private":false,"archived":false},
               {"name":"-erepo-a","default_branch":"main","private":false,"archived":false}'
        json+=']' ;;
    *branches\?protected=true*)
        json='[]'; [ "$name" = repo-a ] && json='[{"name":"protected-one"}]' ;;
    *branches*)
        if [ "$name" = repo-gone ] || [ "$name" = repo-old ]; then
            json='[{"name":"ghost"}]'
        else
            json=$(git --git-dir="$FAKE_REMOTES/$name.git" for-each-ref --format='{"name":"%(refname:lstrip=2)"}' refs/heads \
                | jq -s .) || exit 1
        fi ;;
    *pulls*)
        json='[]'
        [ "$name" = repo-a ] && json='[
            {"base":{"ref":"main"},"head":{"ref":"pr-head","repo":{"full_name":"fixture-org/repo-a"}}},
            {"base":{"ref":"pr-base"},"head":{"ref":"fork-branch","repo":{"full_name":"someone/fork"}}}]' ;;
    *) exit 1 ;;
esac
jq -r "$filter" <<<"$json"
FAKE
chmod +x "$TMP/bin/gh"

run_sweep() {
    PATH="$TMP/bin:$PATH" FAKE_REMOTES="$TMP/remotes" GH_TOKEN="${SWEEP_TOKEN-fixture}" \
        GIT_CONFIG_COUNT=2 \
        GIT_CONFIG_KEY_0="url.file://$TMP/remotes/.insteadOf" GIT_CONFIG_VALUE_0="https://github.com/fixture-org/" \
        GIT_CONFIG_KEY_1=protocol.file.allow GIT_CONFIG_VALUE_1=always \
        "$UNDER_TEST" sweep --org fixture-org "$@"
}
heads() { git -C "$TMP/remotes/repo-a.git" for-each-ref --format='%(refname:lstrip=2)' refs/heads | sort | tr '\n' ' '; }

got=0; run_sweep --dry-run >"$TMP/sweep.txt" 2>&1 || got=$?
if [ $got -eq 0 ] && grep -qF 'would delete 2,' "$TMP/sweep.txt" \
    && [ "$(heads)" = "dependabot/cargo/x fork-branch landed main pr-base pr-head protected-one unlanded young " ]; then
    pass "a dry run names the landed branch and deletes nothing"
else
    fail "a dry run names the landed branch and deletes nothing"; echo "      exit $got, heads: $(heads)"; sed 's/^/      /' "$TMP/sweep.txt"
fi

# The same git failure, end to end: the landed branch must survive and the run
# must go red, so the failure is read rather than swept past.
got=0; PATH="$TMP/failgit:$PATH" FAIL_GIT_ARG=ls-tree run_sweep >"$TMP/sweep.txt" 2>&1 || got=$?
if [ $got -eq 1 ] && grep -qF '`landed`: content check failed' "$TMP/sweep.txt" \
    && [ "$(heads)" = "dependabot/cargo/x fork-branch landed main pr-base pr-head protected-one unlanded young " ]; then
    pass "a git failure inside the check fails the run and deletes nothing"
else
    fail "a git failure inside the check fails the run and deletes nothing"; echo "      exit $got, heads: $(heads)"; sed 's/^/      /' "$TMP/sweep.txt"
fi

got=0; run_sweep >"$TMP/sweep.txt" 2>&1 || got=$?
if [ $got -eq 0 ] && [ "$(heads)" = "dependabot/cargo/x main pr-base pr-head protected-one unlanded young " ] \
    && grep -qF 'Restore: `gh api -X POST repos/fixture-org/repo-a/git/refs -f ref=refs/heads/landed' "$TMP/sweep.txt" \
    && grep -qF '`unlanded` (48h): kept, not landed: u.txt' "$TMP/sweep.txt" \
    && grep -qF '`protected-one`: protected' "$TMP/sweep.txt" \
    && grep -qF '`pr-head`: head or base of an open pull request' "$TMP/sweep.txt" \
    && grep -qF '`pr-base`: head or base of an open pull request' "$TMP/sweep.txt" \
    && grep -qF '`fork-branch` @' "$TMP/sweep.txt" && ! grep -qF 'repo-old' "$TMP/sweep.txt"; then
    pass "a sweep deletes only the old landed branches and prints how to restore them"
else
    fail "a sweep deletes only the old landed branches and prints how to restore them"; echo "      exit $got, heads: $(heads)"; sed 's/^/      /' "$TMP/sweep.txt"
fi

got=0; run_sweep --min-age-hours 0 >"$TMP/sweep.txt" 2>&1 || got=$?
if [ $got -eq 0 ] && [ "$(heads)" = "dependabot/cargo/x main pr-base pr-head protected-one unlanded " ] \
    && grep -qF '`young` @' "$TMP/sweep.txt" && grep -qF '(1h): deleted' "$TMP/sweep.txt"; then
    pass "only the age floor held back the young landed branch"
else
    fail "only the age floor held back the young landed branch"; echo "      exit $got, heads: $(heads)"; sed 's/^/      /' "$TMP/sweep.txt"
fi

# Someone pushes to a checked branch before the delete reaches the remote. A
# git wrapper moves the remote ref the moment the sweep's push starts, after
# the content check has passed on the old tip: the lease must refuse the
# delete, and the branch must survive at the new tip.
git -C "$src" push -q "$TMP/remotes/repo-a.git" landed:refs/heads/racer
mkdir -p "$TMP/race"
cat >"$TMP/race/git" <<RACE
#!/usr/bin/env bash
for a in "\$@"; do
    if [ "\$a" = push ]; then
        "$REAL_GIT" -C "$TMP/remotes/repo-a.git" update-ref refs/heads/racer "\$RACE_TO"
        break
    fi
done
exec "$REAL_GIT" "\$@"
RACE
chmod +x "$TMP/race/git"
race_to="$(git -C "$TMP/remotes/repo-a.git" rev-parse refs/heads/unlanded)"
got=0; PATH="$TMP/race:$PATH" RACE_TO="$race_to" run_sweep >"$TMP/sweep.txt" 2>&1 || got=$?
if [ $got -eq 1 ] && grep -qF '`racer`: delete refused' "$TMP/sweep.txt" \
    && [ "$(git -C "$TMP/remotes/repo-a.git" rev-parse -q --verify refs/heads/racer)" = "$race_to" ]; then
    pass "a push landing between the check and the delete keeps the branch"
else
    fail "a push landing between the check and the delete keeps the branch"
    echo "      exit $got, racer: $(git -C "$TMP/remotes/repo-a.git" rev-parse -q --verify refs/heads/racer || echo deleted)"
    sed 's/^/      /' "$TMP/sweep.txt"
fi

# Run locally, the sweep itemizes private repositories like any other.
got=0; FAKE_GH_PRIVATE=1 run_sweep --dry-run >"$TMP/sweep.txt" 2>&1 || got=$?
if [ $got -eq 1 ] && grep -qF '`repo-p` `secret-landed`' "$TMP/sweep.txt" \
    && grep -qF '`repo-gone`: clone failed' "$TMP/sweep.txt"; then
    pass "without --redact-private a private repository is itemized"
else
    fail "without --redact-private a private repository is itemized"; echo "      exit $got"; sed 's/^/      /' "$TMP/sweep.txt"
fi

# --redact-private with no --itemize names nothing, public or not.
got=0; FAKE_GH_PRIVATE=1 run_sweep --redact-private --dry-run >"$TMP/sweep.txt" 2>&1 || got=$?
if [ $got -eq 1 ] && ! grep -qE 'repo[-.]|secret|ghost|`unlanded`' "$TMP/sweep.txt" \
    && grep -qF '🔒 5 repositories swept without itemizing' "$TMP/sweep.txt"; then
    pass "--redact-private without --itemize names no repository at all"
else
    fail "--redact-private without --itemize names no repository at all"; echo "      exit $got"; sed 's/^/      /' "$TMP/sweep.txt"
fi

# The public lane's run log names no private repository, branch or path —
# not even through git's clone error, nor for a repository public only for a
# private-ci window, nor for a private one wrongly listed in --itemize, nor
# for one whose visibility the listing does not report, nor through a stale
# --itemize entry that only resembles a name (repo.tools holds repo.t as a
# substring, repo-t matches it as a pattern) — yet still sweeps and counts them.
priv_heads() { git -C "$TMP/remotes/$1.git" for-each-ref --format='%(refname:lstrip=2)' refs/heads | sort | tr '\n' ' '; }
got=0; FAKE_GH_PRIVATE=1 run_sweep --redact-private --itemize repo-a --itemize repo-p --itemize repo-gone \
    --itemize repo-t --itemize repo.tools >"$TMP/sweep.txt" 2>&1 || got=$?
if [ $got -eq 1 ] && [ "$(priv_heads repo-p)" = "main secret-work " ] && [ "$(priv_heads repo.t)" = "main secret-work " ] \
    && [ "$(priv_heads -erepo-a)" = "main secret-work " ] \
    && ! grep -qE 'repo-p|repo-gone|repo\.t|-erepo|secret|ghost' "$TMP/sweep.txt" \
    && grep -qF '🔒 4 repositories swept without itemizing' "$TMP/sweep.txt" \
    && grep -qF 'deleted 3, kept 3 unlanded, skipped 0, errors 1.' "$TMP/sweep.txt" \
    && grep -qF 'an unnamed repository: clone failed' "$TMP/sweep.txt" \
    && grep -qF '`repo-a` `unlanded`' "$TMP/sweep.txt"; then
    pass "--redact-private sweeps undeclared repositories without naming them"
else
    fail "--redact-private sweeps undeclared repositories without naming them"
    echo "      exit $got, repo-p: $(priv_heads repo-p), repo.t: $(priv_heads repo.t), -erepo-a: $(priv_heads -erepo-a)"
    sed 's/^/      /' "$TMP/sweep.txt"
fi

# A git failure inside a redacted repository's clone names nothing either.
# Real git refuses a bare repository under safe.bareRepository=explicit and
# prints the directory it refused, on a stderr the sweep does not capture:
# the clone's path must not carry the repository's name.
got=0; FAKE_GH_PRIVATE=1 GIT_CONFIG_PARAMETERS="'safe.bareRepository'='explicit'" \
    run_sweep --redact-private --itemize repo-a >"$TMP/sweep.txt" 2>&1 || got=$?
if [ $got -eq 1 ] && grep -qF 'safe.bareRepository' "$TMP/sweep.txt" \
    && grep -qF 'an unnamed repository: a listed branch is absent from the clone' "$TMP/sweep.txt" \
    && ! grep -qE 'repo-p|repo-gone|repo\.t|-erepo|secret|ghost' "$TMP/sweep.txt"; then
    pass "a git error inside a redacted repository's clone does not name it"
else
    fail "a git error inside a redacted repository's clone does not name it"; echo "      exit $got"; sed 's/^/      /' "$TMP/sweep.txt"
fi

# A value flag with no value, or with the next flag as its value, is a usage
# error before anything is listed: `--itemize --dry-run` must never become a
# real sweep that itemizes a repository called --dry-run.
for args in "--itemize" "--itemize --dry-run" "--org --dry-run" "--min-age-hours"; do
    got=0
    # shellcheck disable=SC2086 # each case is a word list on purpose
    PATH="$TMP/bin:$PATH" GH_TOKEN=fixture "$UNDER_TEST" sweep $args >"$TMP/sweep.txt" 2>&1 || got=$?
    if [ $got -eq 2 ] && grep -qF 'wants a value' "$TMP/sweep.txt"; then
        pass "sweep $args is a usage error"
    else
        fail "sweep $args is a usage error"; echo "      exit $got"; sed 's/^/      /' "$TMP/sweep.txt"
    fi
done

# expect_sweep_fails <label> <reason-substring> <env-assignment...>
# The reason is asserted too: a run failing on some later error would pass a
# bare exit-code check while the guard under test never fired.
expect_sweep_fails() {
    local label="$1" reason="$2" got=0; shift 2
    env "$@" bash -c "$(declare -f run_sweep); TMP='$TMP' UNDER_TEST='$UNDER_TEST' run_sweep" >"$TMP/sweep.txt" 2>&1 || got=$?
    if [ $got -eq 1 ] && grep -qF -- "$reason" "$TMP/sweep.txt"; then
        pass "$label"
    else
        fail "$label"; echo "      exit $got, expected 1 with '$reason'"; sed 's/^/      /' "$TMP/sweep.txt"
    fi
}
expect_sweep_fails "an empty token fails closed instead of sweeping nothing" "GH_TOKEN is empty" SWEEP_TOKEN=
expect_sweep_fails "a failed org listing fails the run" "listing the repositories" FAKE_GH_FAIL=1
expect_sweep_fails "an org that lists no repositories fails closed" "lists no repositories" FAKE_GH_EMPTY=1

echo
if [ $failures -gt 0 ]; then
    echo "❌ $failures fixture test(s) failed"
    exit 1
fi
echo "✅ all fixture tests passed"
