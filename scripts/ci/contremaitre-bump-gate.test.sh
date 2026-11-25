#!/usr/bin/env bash
# ABOUTME: Pins contremaitre-bump-gate.sh against real git remotes and stubbed bun/gh: what a red copy parks,
# ABOUTME: what a flake or main's own red excuses, which failures it names, when a parked bump is retried, and the one stall issue
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The suites themselves are stubbed: `bun`/`bunx` print vitest- and jest-shaped
# output chosen by STUB_MODE and by the en copy on disk, so the bumped copy and
# main's copy can fail differently. The failure-name parser is also fed the
# exact shapes vitest 4, jest 29 and `bun run --filter` printed on 2026-11-25.
set -uo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/contremaitre-bump-gate.sh"
[ -x "$SCRIPT" ] || { echo "FAIL: $SCRIPT not executable"; exit 1; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/bump-gate-test.XXXXXX")" || exit 1
trap 'rm -rf "$WORK"' EXIT
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@e GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@e

pass=0; fail=0
ok()  { echo "  ✅ $1"; pass=$((pass+1)); }
bad() { echo "  ❌ $1"; fail=$((fail+1)); }
check() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 — expected '$3', got '$2'"; fi; }
has() { if grep -qF -- "$3" "$2"; then ok "$1"; else bad "$1 — '$3' not in $2:"; sed 's/^/      /' "$2"; fi; }
lacks() { if grep -qF -- "$3" "$2"; then bad "$1 — '$3' found in $2"; else ok "$1"; fi; }

BOT_NAME="github-actions[bot]"
BOT_EMAIL="41898282+github-actions[bot]@users.noreply.github.com"
NEW=bbbbbbbbcccc
SUBJECT="chore(contremaitre): bump dravr-contremaitre rev to bbbbbbbb"

# ---------------------------------------------------------------------------
# Stubs. bun answers for every suite; only the web suite ever fails, as
# STUB_MODE says, reading the en copy the gate has on disk.
# ---------------------------------------------------------------------------
mkdir -p "$WORK/bin"
cat >"$WORK/bin/bun" <<'SH'
#!/usr/bin/env bash
copy="$(cat "$FIXROOT/packages/i18n/src/locales/en/translation.json")"
case "$PWD" in
  */frontend) ;;
  *) echo "Tests: 3 passed"; exit 0 ;;
esac
new=false; [[ "$copy" == *reworded* ]] && new=true
case "$STUB_MODE" in
  green) echo " Test Files  1 passed (1)"; exit 0 ;;
  copy)
    $new || { echo " Test Files  1 passed (1)"; exit 0; }
    printf ' \033[31mFAIL\033[39m  src/quote.test.tsx > Quote > reads the old sentence\n'; exit 1 ;;
  preexisting)
    echo " FAIL  src/flaky.test.tsx > Flaky > was red on main"; exit 1 ;;
  both)
    echo " FAIL  src/flaky.test.tsx > Flaky > was red on main"
    $new && echo " FAIL  src/quote.test.tsx > Quote > reads the old sentence"
    exit 1 ;;
  unnamed)
    $new || { echo " Test Files  1 passed (1)"; exit 0; }
    echo "Segmentation fault"; exit 1 ;;
  unnamed-everywhere) echo "Segmentation fault"; exit 1 ;;
  flaky)
    # Red on its first run only, whatever the copy.
    n=$(( $(cat "$FLAKE_COUNT" 2>/dev/null || echo 0) + 1 )); echo "$n" >"$FLAKE_COUNT"
    [ "$n" -eq 1 ] && { echo " FAIL  src/quote.test.tsx > Quote > reads the old sentence"; exit 1; }
    echo " Test Files  1 passed (1)"; exit 0 ;;
  flaky-names)
    # Red on every run of the new copy, a different test each time.
    $new || { echo " Test Files  1 passed (1)"; exit 0; }
    n=$(( $(cat "$FLAKE_COUNT" 2>/dev/null || echo 0) + 1 )); echo "$n" >"$FLAKE_COUNT"
    echo " FAIL  src/t${n}.test.tsx > T > run ${n}"; exit 1 ;;
  rewrites) echo changed >>"$FIXROOT/tracked.txt"; exit 0 ;;
esac
SH
printf '#!/usr/bin/env bash\necho "Tests: 1 passed"\n' >"$WORK/bin/bunx"
cat >"$WORK/bin/gh" <<'SH'
#!/usr/bin/env bash
echo "$*" >>"$GH_LOG"
if [[ "$1 $2" == "issue list" ]]; then
  [[ -n "${STUB_LIST_FAILS:-}" ]] && exit 1
  printf '%s' "${STUB_OPEN_ISSUE:-}"
fi
exit 0
SH
chmod +x "$WORK/bin/"*
mkdir -p "$WORK/broken"
printf '#!/usr/bin/env bash\nexit 3\n' >"$WORK/broken/python3"
chmod +x "$WORK/broken/python3"
export PATH="$WORK/bin:$PATH" GH_LOG="$WORK/gh.log" RUN_URL="https://example.invalid/run/1" FLAKE_COUNT="$WORK/flake.count"

# fixture — a bare remote whose main pins aaaaaaaa, and a clone at $WORK/repo
# whose HEAD is the bot's bump to $NEW with a reworded en copy.
fixture() {
    rm -rf "$WORK/remote" "$WORK/repo"
    git init -q --bare "$WORK/remote"
    git --git-dir="$WORK/remote" symbolic-ref HEAD refs/heads/main
    git init -q -b main "$WORK/repo"
    (
        cd "$WORK/repo" || exit 1
        mkdir -p packages/i18n/src/locales/en packages/i18n/__tests__ frontend frontend-mobile crates
        echo probe >packages/i18n/__tests__/probe.test.ts
        printf 'tracker = "dravr-ai/dravr-carnet"\n' >registre.toml
        printf 'dravr-contremaitre = { git = "https://github.com/dravr-ai/dravr-contremaitre.git", rev = "aaaaaaaa" }\n' >Cargo.toml
        echo '{"greet": "old"}' >packages/i18n/src/locales/en/translation.json
        echo base >tracked.txt
        git add -A && git commit -q -m base
        git remote add origin "$WORK/remote"
        git push -q origin main
        sed -i.bak "s/aaaaaaaa/${NEW}/" Cargo.toml && rm Cargo.toml.bak
        echo '{"greet": "reworded"}' >packages/i18n/src/locales/en/translation.json
        git add -A
        GIT_AUTHOR_NAME="$BOT_NAME" GIT_AUTHOR_EMAIL="$BOT_EMAIL" \
            GIT_COMMITTER_NAME="$BOT_NAME" GIT_COMMITTER_EMAIL="$BOT_EMAIL" \
            git commit -q -m "$SUBJECT" -m "Syncs the platform string copy to match: 1 locale file(s) changed."
    )
    export FIXROOT="$WORK/repo"
}
gate() { (cd "$WORK/repo" && "$SCRIPT" "$@") >"$WORK/out" 2>&1; echo $?; }

echo "==== contremaitre-bump-gate.sh ===="

# ---------------------------------------------------------------------------
echo "-- failure names, from real runner output"
printf ' ❯ src/a.test.tsx (2 tests | 1 failed) 9ms\n     × reads it 7ms\n\033[31m FAIL \033[39m src/a.test.tsx > Section — coach > reads it\n' >"$WORK/vitest.log"
"$SCRIPT" failure-ids web "$WORK/vitest.log" >"$WORK/ids"
has "vitest: the test, through its colour codes" "$WORK/ids" "web: src/a.test.tsx > Section — coach > reads it"
has "vitest: the file" "$WORK/ids" "web: src/a.test.tsx"
lacks "vitest: the per-file × line is not a second name" "$WORK/ids" "reads it 7ms"
printf 'FAIL __tests__/b.test.ts (5.2 s)\n  ● Console\n  ● Outer › reads the copy\nPASS __tests__/c.test.ts\nTests: 1 failed\n' >"$WORK/jest.log"
"$SCRIPT" failure-ids mobile "$WORK/jest.log" >"$WORK/ids"
has "jest: the test under its file" "$WORK/ids" "mobile: __tests__/b.test.ts › Outer › reads the copy"
lacks "jest: a console block is not a test" "$WORK/ids" "› Console"
lacks "jest: a passing file is not named" "$WORK/ids" "c.test.ts"
printf '@pierre/i18n test:  FAIL  __tests__/z.test.ts > probe fails\n' >"$WORK/pkg.log"
"$SCRIPT" failure-ids packages "$WORK/pkg.log" >"$WORK/ids"
has "bun --filter: the package is kept" "$WORK/ids" "packages: @pierre/i18n __tests__/z.test.ts > probe fails"

# ---------------------------------------------------------------------------
echo "-- test: the verdict"
fixture
check "every suite green is green" "$(STUB_MODE=green gate test "$WORK/t1")" 0

check "a copy that breaks a test is red" "$(STUB_MODE=copy gate test "$WORK/t2")" 1
has "the failure names the broken test" "$WORK/t2/failures.txt" "web: src/quote.test.tsx > Quote > reads the old sentence"
check "the bumped copy is back on disk after the comparison" \
    "$(cat "$WORK/repo/packages/i18n/src/locales/en/translation.json")" '{"greet": "reworded"}'
check "the tree is clean after the comparison" "$(git -C "$WORK/repo" status --porcelain)" ""

check "a failure main's copy shares is not the bump's" "$(STUB_MODE=preexisting gate test "$WORK/t3")" 0
has "a pre-existing red says so" "$WORK/out" "failures pre-exist on main"

check "a new failure beside a pre-existing one is red" "$(STUB_MODE=both gate test "$WORK/t4")" 1
has "only the new failure is the bump's" "$WORK/t4/failures.txt" "Quote > reads the old sentence"
lacks "the shared failure is not blamed on the bump" "$WORK/t4/failures.txt" "Flaky"

check "a red nothing names is still red" "$(STUB_MODE=unnamed gate test "$WORK/t5")" 1
has "an unnamed red hands over the log tail" "$WORK/t5/failures.txt" "Segmentation fault"
check "an unnamed red main's copy shares is not the bump's" "$(STUB_MODE=unnamed-everywhere gate test "$WORK/t5b")" 0

rm -f "$FLAKE_COUNT"
check "a red that passes on its rerun on the same copy is a flake" "$(STUB_MODE=flaky gate test "$WORK/t8")" 0
has "a flake says so" "$WORK/out" "green (flaky reds)"
check "a flake parks nothing" "$(wc -c <"$WORK/t8/failures.txt" | tr -d ' ')" 0
rm -f "$FLAKE_COUNT"
check "a red naming no test twice is a flake too" "$(STUB_MODE=flaky-names gate test "$WORK/t9")" 0
rm -f "$FLAKE_COUNT"
check "the rerun happens on the new copy" "$(STUB_MODE=copy gate test "$WORK/t10"; ls "$WORK/t10/web.new2.log" >/dev/null 2>&1 && echo rerun)" "1
rerun"

check "a parser that crashes is a gate that cannot judge, never a red" \
    "$(STUB_MODE=copy PATH="$WORK/broken:$PATH" gate test "$WORK/t11")" 2
has "the crash names itself" "$WORK/out" "failed unexpectedly"
check "the bumped copy is back on disk after a crash" \
    "$(git -C "$WORK/repo" status --porcelain)" ""

echo x >>"$WORK/repo/tracked.txt"
check "a dirty tree is not judged" "$(STUB_MODE=green gate test "$WORK/t6")" 2
git -C "$WORK/repo" checkout -q -- tracked.txt
check "tests that rewrite a tracked file are not judged" "$(STUB_MODE=rewrites gate test "$WORK/t7")" 2
git -C "$WORK/repo" checkout -q -- tracked.txt
(cd "$WORK/repo" && git rm -q -r packages/i18n/__tests__ && git commit -q --amend --no-edit)
check "no shared-package test file is a gate that cannot judge, never a green" "$(STUB_MODE=green gate test "$WORK/t12")" 2
has "it says the suite would have run nothing" "$WORK/out" "would pass having run nothing"

# ---------------------------------------------------------------------------
echo "-- park and prune"
fixture
git -C "$WORK/repo" push -q origin main:refs/heads/contremaitre-bump/99999999
check "the lane's bump parks" "$(gate park "$NEW")" 0
check "the stall branch holds the bump" \
    "$(git --git-dir="$WORK/remote" rev-parse refs/heads/contremaitre-bump/bbbbbbbb)" "$(git -C "$WORK/repo" rev-parse HEAD)"
check "a superseded stall branch is deleted" \
    "$(git --git-dir="$WORK/remote" for-each-ref --format='%(refname)' refs/heads/contremaitre-bump/99999999)" ""
git -C "$WORK/repo" commit -q --amend -m "chore: something else"
check "a commit that is not the lane's bump never parks" "$(gate park "$NEW")" 2
check "the refused park pushed nothing" \
    "$(git --git-dir="$WORK/remote" rev-parse refs/heads/contremaitre-bump/bbbbbbbb)" "$(git -C "$WORK/repo" rev-parse 'HEAD@{1}')"
git -C "$WORK/repo" commit -q --amend -m "$SUBJECT"
(cd "$WORK/repo" && GIT_AUTHOR_NAME="$BOT_NAME" GIT_AUTHOR_EMAIL="$BOT_EMAIL" \
    GIT_COMMITTER_NAME="$BOT_NAME" GIT_COMMITTER_EMAIL="$BOT_EMAIL" git commit -q --amend --no-edit --reset-author)
check "a retry replaces the lane's own parked tip" "$(gate park "$NEW")" 0
check "the stall branch holds the retried bump" \
    "$(git --git-dir="$WORK/remote" rev-parse refs/heads/contremaitre-bump/bbbbbbbb)" "$(git -C "$WORK/repo" rev-parse HEAD)"
(cd "$WORK/repo" && git checkout -q --detach HEAD && echo fix >frontend/quote.test.tsx && git add -A \
    && git commit -q -m "fix(test): the new copy" && git push -q origin HEAD:refs/heads/contremaitre-bump/bbbbbbbb)
human_tip="$(git -C "$WORK/repo" rev-parse HEAD)"
git -C "$WORK/repo" checkout -q HEAD~1
check "a retry never overwrites a human's commit on the stall branch" "$(gate park "$NEW")" 2
has "it says why" "$WORK/out" "which the lane did not make; left as it is"
check "the human's commit is still the tip" \
    "$(git --git-dir="$WORK/remote" rev-parse refs/heads/contremaitre-bump/bbbbbbbb)" "$human_tip"
check "prune exits clean" "$(gate prune)" 0
check "prune deletes every stall branch" "$(git --git-dir="$WORK/remote" for-each-ref refs/heads/contremaitre-bump)" ""

# ---------------------------------------------------------------------------
echo "-- current-stall: when the poll stops retrying"
fixture
gate park "$NEW" >/dev/null
git -C "$WORK/repo" fetch -q origin
git -C "$WORK/repo" checkout -q -B main origin/main
check "a parked bump on an unmoved main is current" "$(gate current-stall "$NEW")" 0
(cd "$WORK/repo" && mkdir -p frontend && echo t >frontend/quote.test.tsx && git add -A && git commit -q -m "client test")
check "a main whose client code moved still stops: only a dispatch retries" "$(gate current-stall "$NEW")" 0
git -C "$WORK/repo" checkout -q -B main origin/main
check "another target has no current stall" "$(gate current-stall dddddddd0000)" 1
(cd "$WORK/repo" && git checkout -q --detach origin/contremaitre-bump/bbbbbbbb \
    && mkdir -p frontend && echo fix >frontend/fix.test.tsx && git add -A && git commit -q -m "fix(test): on the stall branch" \
    && git push -q origin HEAD:refs/heads/contremaitre-bump/bbbbbbbb && git fetch -q origin && git checkout -q main)
check "a human's fix on top of the parked bump still stops the poll" "$(gate current-stall "$NEW")" 0
(cd "$WORK/repo" && git checkout -q --detach origin/contremaitre-bump/bbbbbbbb~1 \
    && git push -q -f origin HEAD:refs/heads/contremaitre-bump/bbbbbbbb && git fetch -q origin && git checkout -q main)
(cd "$WORK/repo" && git checkout -q --detach origin/contremaitre-bump/bbbbbbbb \
    && git commit -q --amend --no-edit --reset-author \
    && git push -q -f origin HEAD:refs/heads/contremaitre-bump/bbbbbbbb \
    && git fetch -q origin && git checkout -q main)
check "a human-authored stall tip is not current" "$(gate current-stall "$NEW")" 1

# ---------------------------------------------------------------------------
echo "-- pin-landed-by-hand: the stall squashed onto main"
fixture
check "the lane's own bump is not a hand landing" "$(gate pin-landed-by-hand 6)" 1
(cd "$WORK/repo" && git commit -q --amend --reset-author --no-edit)
check "a human's squash of the stall is" "$(gate pin-landed-by-hand 6)" 0
(cd "$WORK/repo" && echo x >frontend/later.tsx && git add -A && git commit -q -m later)
check "a later commit that leaves the pin alone changes nothing" "$(gate pin-landed-by-hand 6)" 0
(cd "$WORK/repo" && git reset -q --hard HEAD~1 \
    && GIT_COMMITTER_DATE="2026-01-01T00:00:00" git commit -q --amend --no-edit)
check "an old hand landing is outside the window" "$(gate pin-landed-by-hand 6)" 1

# ---------------------------------------------------------------------------
echo "-- the one stall issue"
fixture
mkdir -p "$WORK/f" && echo "web: src/quote.test.tsx > Quote > reads the old sentence" >"$WORK/f/failures.txt"
: >"$GH_LOG"
check "a first stall files" "$(STUB_OPEN_ISSUE='' gate file-stall aaaaaaaa1111 "$NEW" "$WORK/f")" 0
has "it opens an issue on the register" "$GH_LOG" "issue create --repo dravr-ai/dravr-carnet --title [platform] dravr-contremaitre bump stalled (aaaaaaaa -> bbbbbbbb): copy breaks unit tests"
has "it labels the issue with a label the register has" "$GH_LOG" "--label dravr-platform"
lacks "it asks for no label the register lacks" "$GH_LOG" "--label dependency"
has "the body names the failing test" "$GH_LOG" "Quote > reads the old sentence"
has "the body names the stall branch" "$GH_LOG" "git switch -c feature/contremaitre-bump-bbbbbbbb origin/contremaitre-bump/bbbbbbbb"
has "the body carries the run" "$GH_LOG" "$RUN_URL"
: >"$GH_LOG"
check "a later stall edits" "$(STUB_OPEN_ISSUE='42' gate file-stall aaaaaaaa1111 "$NEW" "$WORK/f")" 0
has "it edits the open issue" "$GH_LOG" "issue edit 42 --repo dravr-ai/dravr-carnet"
lacks "it never opens a second" "$GH_LOG" "issue create"
: >"$GH_LOG"
check "a stall that could not park still files" "$(STUB_OPEN_ISSUE='' PARK_OUTCOME="not parked" gate file-stall aaaaaaaa1111 "$NEW" "$WORK/f")" 0
has "the body says the bump could not be parked" "$GH_LOG" "The bump could not be parked"
: >"$GH_LOG"
check "a test job with no verdict files" "$(STUB_OPEN_ISSUE='' CLIENTS_RESULT=cancelled gate file-stall aaaaaaaa1111 "$NEW" "$WORK/nothing")" 0
has "its title says no verdict" "$GH_LOG" "bbbbbbbb): client tests reached no verdict"
: >"$GH_LOG"
check "a landed bump closes the issue" "$(STUB_OPEN_ISSUE='42' gate close-stall "landed")" 0
has "it closes that issue saying why" "$GH_LOG" "issue close 42 --repo dravr-ai/dravr-carnet --reason completed --comment landed"
: >"$GH_LOG"
check "a hand landing closes only the stall for its rev" "$(STUB_OPEN_ISSUE='42' gate close-stall "landed" "$NEW")" 0
has "it asks for the issue naming that rev" "$GH_LOG" 'contains(" -> bbbbbbbb)")'
: >"$GH_LOG"
check "nothing open, nothing closed" "$(STUB_OPEN_ISSUE='' gate close-stall "landed")" 0
lacks "no close without an open issue" "$GH_LOG" "issue close"
: >"$GH_LOG"
check "a register search that failed is not 'nothing open'" "$(STUB_LIST_FAILS=1 gate close-stall "landed")" 2
lacks "a failed search reports no empty register" "$WORK/out" "no open stall issue"
: >"$GH_LOG"
check "a stall still files when the search failed" "$(STUB_LIST_FAILS=1 gate file-stall aaaaaaaa1111 "$NEW" "$WORK/f")" 0
has "it opens the issue it could not find" "$GH_LOG" "issue create --repo dravr-ai/dravr-carnet"
has "and warns that it searched nothing" "$WORK/out" "could not search"
check "prune over an unreachable remote is not a clean prune" "$(REMOTE=nowhere gate prune)" 2

echo ""
echo "passed: $pass, failed: $fail"
[ "$fail" -eq 0 ]
