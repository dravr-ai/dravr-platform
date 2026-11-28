#!/usr/bin/env bash
# ABOUTME: Fixture test for web-shared-mirror.sh — syncs into throwaway repos and breaks each invariant
# ABOUTME: (hand edit, drift, ignore and export rules, a new package, wrong submodule) to prove it holds, offline
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Runs the real script against this repo's own packages/ at HEAD, and against
# synthetic commits built on HEAD with a throwaway index (written to the object
# store, never to a ref), into local repositories in a temp dir; nothing touches
# the network.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
UNDER_TEST="$REPO_ROOT/scripts/ci/web-shared-mirror.sh"

pass=0; fail=0
ok() { echo "  ✅ $1"; pass=$((pass + 1)); }
ko() { echo "  ❌ $1"; fail=$((fail + 1)); }

WORK="$(mktemp -d)"
probe="$REPO_ROOT/packages/.web-shared-mirror-probe-$$"
trap 'rm -rf "$WORK" "$probe"' EXIT
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1

new_repo() { git init -q -b main "$1"; }
head_sha() { git -C "$1" rev-parse HEAD; }

echo "== web-shared-mirror"

# 1. A hand-made HEAD is refused, and --adopt takes it over.
M="$WORK/mirror"; new_repo "$M"
echo "snapshot" >"$M/stale.txt"; git -C "$M" add -A; git -C "$M" commit -q -m "chore: import"
if out="$("$UNDER_TEST" sync --mirror "$M" 2>&1)"; then ko "a hand-made HEAD is refused (it synced: $out)"
else grep -q "edited by hand" <<<"$out" && ok "a hand-made HEAD is refused and says why" || ko "refusal names the hand edit ($out)"; fi
[ "$(git -C "$M" log -1 --format=%s)" = "chore: import" ] && ok "a refusal commits nothing" || ko "a refusal commits nothing"

"$UNDER_TEST" sync --mirror "$M" --adopt >/dev/null 2>&1 && ok "--adopt syncs over a hand-made HEAD" || ko "--adopt syncs over a hand-made HEAD"
sha="$(git -C "$REPO_ROOT" rev-parse HEAD)"
git -C "$M" log -1 --format=%B | grep -q "^Synced-From: dravr-platform@$sha$" && ok "the sync commit names the platform commit" || ko "the sync commit names the platform commit"

# 2. The mirror holds exactly packages/ at the commit, plus the root templates.
expected="$( (git -C "$REPO_ROOT" ls-tree -r --name-only HEAD packages | sed 's#^packages/##'; echo README.md; echo package.json; echo .gitignore) | sort -u)"
actual="$(git -C "$M" ls-files | sort)"
[ "$expected" = "$actual" ] && ok "the mirror's file list is packages/ plus the root templates" || ko "file list matches ($(diff <(echo "$expected") <(echo "$actual") | head -5 | tr '\n' ' '))"
[ ! -e "$M/stale.txt" ] && ok "a file the platform does not have is removed" || ko "a stale file is removed"
cmp -s "$M/README.md" <(git -C "$REPO_ROOT" show HEAD:scripts/web-shared/README.md) && ok "README.md comes from scripts/web-shared" || ko "README.md comes from scripts/web-shared"
f="$(git -C "$REPO_ROOT" ls-tree -r --name-only HEAD packages | head -1)"
cmp -s "$M/${f#packages/}" <(git -C "$REPO_ROOT" show "HEAD:$f") && ok "a package file is byte-identical" || ko "a package file is byte-identical"

# 3. A second run is a no-op.
before="$(head_sha "$M")"
"$UNDER_TEST" sync --mirror "$M" >/dev/null 2>&1 && [ "$(head_sha "$M")" = "$before" ] && ok "a mirror in sync gets no commit" || ko "a mirror in sync gets no commit"
"$UNDER_TEST" sync --mirror "$M" --check >/dev/null 2>&1 && ok "--check passes a mirror in sync" || ko "--check passes a mirror in sync"

# 4. Drift made by a sync-looking commit is reported by --check and repaired by a run.
echo "drift" >>"$M/README.md"; git -C "$M" commit -q -am "x" -m "Synced-From: dravr-platform@$sha"
if "$UNDER_TEST" sync --mirror "$M" --check >/dev/null 2>&1; then ko "--check reports drift"; else ok "--check reports drift"; fi
git -C "$M" diff --quiet && git -C "$M" diff --cached --quiet && ok "--check leaves the checkout clean" || ko "--check leaves the checkout clean"
"$UNDER_TEST" sync --mirror "$M" >/dev/null 2>&1 && cmp -s "$M/README.md" <(git -C "$REPO_ROOT" show HEAD:scripts/web-shared/README.md) && ok "a run repairs drift" || ko "a run repairs drift"

# 5. Untracked files in the platform never reach the mirror.
echo "local only" >"$probe"
"$UNDER_TEST" sync --mirror "$M" >/dev/null 2>&1 && synced=true || synced=false
rm -f "$probe"
[ "$synced" = true ] && [ ! -e "$M/$(basename "$probe")" ] && ok "an untracked platform file is not mirrored" || ko "an untracked platform file is not mirrored"

# 6. bump-pitoune moves the gitlink, once.
P="$WORK/pitoune"; new_repo "$P"
printf '[submodule "packages"]\n\tpath = packages\n\turl = https://github.com/dravr-ai/dravr-web-shared\n' >"$P/.gitmodules"
git -C "$P" add .gitmodules
old="$(git -C "$M" rev-parse HEAD~1)"
git -C "$P" update-index --add --cacheinfo "160000,$old,packages"
git -C "$P" commit -q -m "mount"
new="$(head_sha "$M")"
"$UNDER_TEST" bump-pitoune --pitoune "$P" --sha "$new" >/dev/null 2>&1
[ "$(git -C "$P" ls-tree HEAD packages | awk '{print $3}')" = "$new" ] && ok "bump-pitoune points packages/ at the mirror commit" || ko "bump-pitoune points packages/ at the mirror commit"
before="$(head_sha "$P")"
"$UNDER_TEST" bump-pitoune --pitoune "$P" --sha "$new" >/dev/null 2>&1 && [ "$(head_sha "$P")" = "$before" ] && ok "bump-pitoune at the same commit is a no-op" || ko "bump-pitoune at the same commit is a no-op"

# 7. bump-pitoune refuses a packages/ that is not the web-shared submodule, and a short sha.
Q="$WORK/other"; new_repo "$Q"
printf '[submodule "packages"]\n\tpath = packages\n\turl = https://github.com/someone/else\n' >"$Q/.gitmodules"
git -C "$Q" add .gitmodules; git -C "$Q" update-index --add --cacheinfo "160000,$old,packages"; git -C "$Q" commit -q -m m
if out="$("$UNDER_TEST" bump-pitoune --pitoune "$Q" --sha "$new" 2>&1)"; then ko "a foreign submodule is refused"
else grep -q "not the dravr-web-shared submodule" <<<"$out" && ok "a foreign submodule is refused and says why" || ko "foreign-submodule refusal names the cause ($out)"; fi
if out="$("$UNDER_TEST" bump-pitoune --pitoune "$P" --sha "${new:0:12}" 2>&1)"; then ko "a short sha is refused"
else grep -q "full 40-character" <<<"$out" && ok "a short sha is refused and says why" || ko "short-sha refusal names the cause ($out)"; fi

# 8. A sync that fails after staging (here the mirror's commit is refused by a hook)
#    fails the run and leaves no staging directory behind.
tmpbase="$WORK/tmp"; mkdir -p "$tmpbase"
H="$WORK/hooked"; new_repo "$H"; echo x >"$H/x"; git -C "$H" add -A; git -C "$H" commit -q -m "hand"
printf '#!/bin/sh\nexit 1\n' >"$H/.git/hooks/pre-commit"; chmod +x "$H/.git/hooks/pre-commit"
if TMPDIR="$tmpbase" "$UNDER_TEST" sync --mirror "$H" --adopt >/dev/null 2>&1; then ko "a refused mirror commit fails the sync"
else ok "a refused mirror commit fails the sync"; fi
[ -z "$(ls -A "$tmpbase")" ] && ok "a failed sync leaves no staging directory" || ko "a failed sync leaves no staging directory ($(ls -A "$tmpbase"))"
[ -z "$(git -C "$H" status --porcelain --untracked-files=no)" ] && ok "a failed sync puts the mirror checkout back at HEAD" || ko "a failed sync puts the mirror checkout back at HEAD"

# 9. A mirror checkout with local changes is refused, and left as it was.
echo "local edit" >>"$M/README.md"
if out="$("$UNDER_TEST" sync --mirror "$M" 2>&1)"; then ko "a dirty mirror checkout is refused"
else grep -q "local changes" <<<"$out" && ok "a dirty mirror checkout is refused and says why" || ko "dirty-checkout refusal names the cause ($out)"; fi
grep -q "local edit" "$M/README.md" && ok "a refused sync keeps the local edit" || ko "a refused sync keeps the local edit"
git -C "$M" checkout -q -- README.md

# Synthetic platform commits: HEAD plus the given "path<TAB>content" lines.
synthetic() {
    local idx="$WORK/index.$RANDOM" line path blob tree
    GIT_INDEX_FILE="$idx" git -C "$REPO_ROOT" read-tree HEAD
    for line in "$@"; do
        path="${line%%$'\t'*}"
        blob="$(printf '%s\n' "${line#*$'\t'}" | git -C "$REPO_ROOT" hash-object -w --stdin)"
        GIT_INDEX_FILE="$idx" git -C "$REPO_ROOT" update-index --add --cacheinfo "100644,$blob,$path"
    done
    tree="$(GIT_INDEX_FILE="$idx" git -C "$REPO_ROOT" write-tree)"
    git -C "$REPO_ROOT" commit-tree "$tree" -p HEAD -m "synthetic"
}
first_pkg="$(git -C "$REPO_ROOT" ls-tree --name-only HEAD packages/ | sed -n 's#^packages/##p' | while read -r d; do git -C "$REPO_ROOT" cat-file -e "HEAD:packages/$d/package.json" 2>/dev/null && { echo "$d"; break; }; done)"

# 10. export-ignore and .gitignore rules inside packages/ cannot drop a tracked file.
rules="$(synthetic "packages/.gitattributes	$first_pkg/kept.ts export-ignore" \
    "packages/$first_pkg/.gitignore	dist" \
    "packages/$first_pkg/kept.ts	export const kept = 1;" \
    "packages/$first_pkg/dist/built.js	export const built = 1;")"
"$UNDER_TEST" sync --mirror "$M" --rev "$rules" >/dev/null 2>&1
[ -f "$M/$first_pkg/kept.ts" ] && git -C "$M" ls-files --error-unmatch "$first_pkg/kept.ts" >/dev/null 2>&1 \
    && ok "an export-ignored file is still mirrored" || ko "an export-ignored file is still mirrored"
git -C "$M" ls-files --error-unmatch "$first_pkg/dist/built.js" >/dev/null 2>&1 \
    && ok "a tracked file under a .gitignore rule is still mirrored" || ko "a tracked file under a .gitignore rule is still mirrored"
"$UNDER_TEST" sync --mirror "$M" --rev "$rules" --check >/dev/null 2>&1 && ok "--check agrees the rule-laden tree is in sync" || ko "--check agrees the rule-laden tree is in sync"
"$UNDER_TEST" sync --mirror "$M" >/dev/null 2>&1

# 11. The mirror's workspaces are written from the packages that exist: a new
#     package is listed without touching the template, and a template that lists
#     workspaces itself is refused.
newpkg="$(synthetic "packages/brand-new/package.json	{\"name\": \"brand-new\"}")"
"$UNDER_TEST" sync --mirror "$M" --rev "$newpkg" >/dev/null 2>&1
python3 -c 'import json,sys; w=json.load(open(sys.argv[1]))["workspaces"]; sys.exit(0 if "brand-new" in w and w==sorted(w) else 1)' "$M/package.json" \
    && ok "a new package appears in the mirror's workspaces" || ko "a new package appears in the mirror's workspaces"
listed="$(synthetic "scripts/web-shared/package.json	{\"name\": \"x\", \"workspaces\": [\"a\"]}")"
if out="$("$UNDER_TEST" sync --mirror "$M" --rev "$listed" 2>&1)"; then ko "a template listing workspaces is refused"
else grep -q "must not list workspaces" <<<"$out" && ok "a template listing workspaces is refused and says why" || ko "template refusal names the cause ($out)"; fi
"$UNDER_TEST" sync --mirror "$M" >/dev/null 2>&1

# 12. A packages/ file that would collide with a root template is refused.
collide="$(synthetic "packages/README.md	hand readme")"
if out="$("$UNDER_TEST" sync --mirror "$M" --rev "$collide" 2>&1)"; then ko "a template collision is refused"
else grep -q "collide" <<<"$out" && ok "a template collision is refused and says why" || ko "collision refusal names the cause ($out)"; fi

# 13. Untracked files in a hand-run mirror checkout are never swept into a sync commit.
echo "junk" >"$M/untracked-junk.txt"
drift="$(synthetic "packages/$first_pkg/drift.ts	export const drift = 1;")"
"$UNDER_TEST" sync --mirror "$M" --rev "$drift" >/dev/null 2>&1
git -C "$M" ls-files --error-unmatch untracked-junk.txt >/dev/null 2>&1 && ko "untracked mirror files stay out of the commit" || ok "untracked mirror files stay out of the commit"

echo ""
echo "$pass passed · $fail failed"
[ "$fail" -eq 0 ]
