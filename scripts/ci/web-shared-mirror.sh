#!/usr/bin/env bash
# ABOUTME: Publishes this repo's packages/ as the read-only dravr-web-shared mirror, and moves
# ABOUTME: dravr-pitoune's packages/ submodule to the mirror commit it just made
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# packages/ here is the one source of the TypeScript shared by the web app and the
# React Native app. Three of its packages are generated from the Rust server in this
# repo (scene-types, shared-types, shared-constants/src/*.generated.ts) and the web
# app that consumes all of them lives here too, so it cannot live anywhere else.
#
# The app lives in the private dravr-ai/dravr-pitoune, which mounts
# dravr-ai/dravr-web-shared at its packages/. That repo is therefore a mirror: every
# file in it is written by `sync` from a commit of this repo, and its root files
# (README.md, package.json, .gitignore) come from scripts/web-shared/. Nobody edits
# it by hand; a change to a shared package lands here and reaches the app through
# this script, run by .github/workflows/web-shared-mirror.yml.
#
#   sync --mirror <dir> [--rev <rev>] [--check] [--adopt]
#       Make the checkout at <dir> hold exactly the files of packages/ at <rev>
#       (default HEAD) plus the scripts/web-shared/ root files, and commit it with a
#       `Synced-From: dravr-platform@<sha>` trailer. A mirror already in sync makes
#       no commit. --check reports drift (exit 1) and changes nothing.
#       Files are written from the commit's blobs, byte for byte with their modes:
#       never from the working tree (no node_modules or uncommitted edits), never
#       through `git archive` (export-ignore and export-subst would alter the copy),
#       and staged by name with --force (a .gitignore inside packages/ cannot drop one).
#       It refuses a mirror whose HEAD carries no Synced-From trailer, since that
#       commit was made by hand and syncing over it would silently discard it: port
#       the change into packages/ here first. --adopt overrides the refusal, for a
#       person taking a mirror over on purpose, by hand. It also refuses a mirror
#       checkout with local changes, which a sync would destroy.
#
#   bump-pitoune --pitoune <dir> --sha <mirror sha>
#       Point dravr-pitoune's packages/ gitlink at <sha> and commit it. No-op when
#       it already points there. Refuses a checkout whose packages/ submodule is not
#       dravr-web-shared.
#
# Neither subcommand pushes: the workflow does, so a run here can be tested on
# throwaway repositories. Both print commit ids and counts only — the two target
# repositories are private and the workflow's log is public.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEMPLATES="scripts/web-shared"
ROOT_FILES=(README.md package.json .gitignore)
TRAILER="Synced-From: dravr-platform@"
SUBMODULE_URL_PATTERN='[/:]dravr-ai/dravr-web-shared(\.git)?$'

# The staging directory of a sync, removed on every exit: die and errexit leave
# through exit, which never runs a function's RETURN trap.
STAGE=""
# A sync that fails after it began rewriting the mirror also puts the checkout back
# at HEAD; it was clean when the sync started, so nothing of anyone's is lost.
MIRROR_DIRTIED=""
trap '[ -z "$STAGE" ] || rm -rf "$STAGE"; [ -z "$MIRROR_DIRTIED" ] || git -C "$MIRROR_DIRTIED" reset -q --hard' EXIT

die() { echo "❌ web-shared-mirror: $*" >&2; exit 1; }

# Write every file under packages/ at <sha> into <dest>, from its blob, with its mode,
# and append each path (NUL-terminated, relative to packages/) to <list>.
stage_packages() {
    local sha="$1" dest="$2" list="$3" meta path mode type obj rel
    while IFS= read -r -d '' entry; do
        meta="${entry%%$'\t'*}"
        path="${entry#*$'\t'}"
        read -r mode type obj <<<"$meta"
        rel="${path#packages/}"
        [ "$type" = blob ] || die "sync: $path is a $type; the mirror holds files only"
        mkdir -p "$dest/$(dirname "$rel")"
        case "$mode" in
            120000) ln -s "$(git -C "$REPO_ROOT" cat-file blob "$obj")" "$dest/$rel" ;;
            100755) git -C "$REPO_ROOT" cat-file blob "$obj" >"$dest/$rel"; chmod 755 "$dest/$rel" ;;
            100644) git -C "$REPO_ROOT" cat-file blob "$obj" >"$dest/$rel"; chmod 644 "$dest/$rel" ;;
            *) die "sync: $path has mode $mode, which the mirror cannot hold" ;;
        esac
        printf '%s\0' "$rel" >>"$list"
    done < <(git -C "$REPO_ROOT" ls-tree -r -z --full-tree "$sha" -- packages/)
}

# The mirror's package.json: the template with its workspaces written from the
# packages that exist at <sha>, so a package added here is in the app's install
# without a second list to keep.
render_package_json() {
    local sha="$1" out="$2"
    git -C "$REPO_ROOT" ls-tree -r --name-only "$sha" -- packages/ \
        | sed -n 's#^packages/\([^/]*\)/package\.json$#\1#p' | sort >"$out.workspaces"
    [ -s "$out.workspaces" ] || die "sync: no package under packages/ at $sha"
    git -C "$REPO_ROOT" show "$sha:$TEMPLATES/package.json" | python3 -c '
import json, sys
doc = json.load(sys.stdin)
if "workspaces" in doc:
    sys.exit("the template must not list workspaces; sync writes them")
doc["workspaces"] = [line.strip() for line in open(sys.argv[1]) if line.strip()]
print(json.dumps(doc, indent=2))
' "$out.workspaces" >"$out" || die "sync: $TEMPLATES/package.json at $sha cannot be rendered"
    rm -f "$out.workspaces"
}

sync_mirror() {
    local mirror="" rev="HEAD" check=false adopt=false
    while [ $# -gt 0 ]; do
        case "$1" in
            --mirror) mirror="${2:-}"; shift 2 ;;
            --rev) rev="${2:-}"; shift 2 ;;
            --check) check=true; shift ;;
            --adopt) adopt=true; shift ;;
            *) die "sync: unknown argument $1" ;;
        esac
    done
    [ -n "$mirror" ] || die "sync: --mirror <dir> is required"
    git -C "$mirror" rev-parse --git-dir >/dev/null 2>&1 || die "sync: $mirror is not a git checkout"
    [ -z "$(git -C "$mirror" status --porcelain --untracked-files=no)" ] \
        || die "sync: $mirror has local changes, which a sync would destroy"

    local sha short
    sha="$(git -C "$REPO_ROOT" rev-parse --verify "$rev^{commit}")" || die "sync: $rev is not a commit here"
    short="$(git -C "$REPO_ROOT" rev-parse --short=12 "$sha")"
    for f in "${ROOT_FILES[@]}"; do
        git -C "$REPO_ROOT" cat-file -e "$sha:$TEMPLATES/$f" 2>/dev/null \
            || die "sync: $TEMPLATES/$f is missing at $short"
        if git -C "$REPO_ROOT" cat-file -e "$sha:packages/$f" 2>/dev/null; then
            die "sync: packages/$f exists at $short and would collide with $TEMPLATES/$f"
        fi
    done

    if git -C "$mirror" rev-parse --verify -q HEAD >/dev/null \
        && ! git -C "$mirror" log -1 --format=%B | grep -q "^$TRAILER" \
        && [ "$adopt" != true ]; then
        die "sync: the mirror's HEAD $(git -C "$mirror" rev-parse --short HEAD) has no '$TRAILER' trailer, so it was edited by hand. Port that change into packages/ here, then re-run with --adopt to take the mirror over."
    fi

    local stage list
    STAGE="$(mktemp -d "${TMPDIR:-/tmp}/web-shared-mirror.XXXXXX")"
    stage="$STAGE"
    list="$stage/.paths"
    mkdir "$stage/tree"
    stage_packages "$sha" "$stage/tree" "$list"
    for f in "${ROOT_FILES[@]}"; do
        if [ "$f" = package.json ]; then
            render_package_json "$sha" "$stage/tree/$f"
        else
            git -C "$REPO_ROOT" show "$sha:$TEMPLATES/$f" >"$stage/tree/$f"
        fi
        printf '%s\0' "$f" >>"$list"
    done

    # Replace the mirror's tracked files with the staged tree, and stage exactly those
    # paths: untracked files in a hand-run mirror checkout are never swept in.
    MIRROR_DIRTIED="$mirror"
    git -C "$mirror" ls-files -z | (cd "$mirror" && xargs -0 rm -f --)
    git -C "$mirror" rm -r -q --cached --ignore-unmatch -- . >/dev/null
    (cd "$stage/tree" && tar -cf - .) | (cd "$mirror" && tar -xf -)
    git -C "$mirror" add --force --pathspec-from-file="$list" --pathspec-file-nul

    if git -C "$mirror" diff --cached --quiet; then
        MIRROR_DIRTIED=""
        echo "✅ dravr-web-shared already matches dravr-platform@$short"
        return 0
    fi

    local changed
    changed="$(git -C "$mirror" diff --cached --name-only | wc -l | tr -d ' ')"
    if [ "$check" = true ]; then
        git -C "$mirror" reset -q --hard
        echo "❌ dravr-web-shared differs from dravr-platform@$short in $changed file(s)"
        return 1
    fi

    git -C "$mirror" commit -q -m "sync: dravr-platform $short" \
        -m "Generated from dravr-platform packages/ by scripts/ci/web-shared-mirror.sh. Edit packages/ there, never here." \
        -m "$TRAILER$sha"
    MIRROR_DIRTIED=""
    echo "✅ dravr-web-shared $(git -C "$mirror" rev-parse --short=12 HEAD): $changed file(s) from dravr-platform@$short"
}

bump_pitoune() {
    local pitoune="" sha=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --pitoune) pitoune="${2:-}"; shift 2 ;;
            --sha) sha="${2:-}"; shift 2 ;;
            *) die "bump-pitoune: unknown argument $1" ;;
        esac
    done
    [ -n "$pitoune" ] || die "bump-pitoune: --pitoune <dir> is required"
    [[ "$sha" =~ ^[0-9a-f]{40}$ ]] || die "bump-pitoune: --sha must be a full 40-character commit id"
    git -C "$pitoune" rev-parse --git-dir >/dev/null 2>&1 || die "bump-pitoune: $pitoune is not a git checkout"

    local url mode current
    url="$(git -C "$pitoune" config -f .gitmodules --get submodule.packages.url 2>/dev/null || true)"
    [[ "$url" =~ $SUBMODULE_URL_PATTERN ]] || die "bump-pitoune: packages/ is not the dravr-web-shared submodule (url: ${url:-none})"
    read -r mode current _ < <(git -C "$pitoune" ls-files -s -- packages) || true
    [ "${mode:-}" = "160000" ] || die "bump-pitoune: packages/ is not a gitlink in the index"

    if [ "$current" = "$sha" ]; then
        echo "✅ dravr-pitoune packages/ already at ${sha:0:12}"
        return 0
    fi
    git -C "$pitoune" update-index --cacheinfo "160000,$sha,packages"
    git -C "$pitoune" commit -q -m "chore(packages): dravr-web-shared ${sha:0:12}" \
        -m "Moved by dravr-platform scripts/ci/web-shared-mirror.sh after the mirror sync."
    echo "✅ dravr-pitoune $(git -C "$pitoune" rev-parse --short=12 HEAD): packages/ ${current:0:12} -> ${sha:0:12}"
}

case "${1:-}" in
    sync) shift; sync_mirror "$@" ;;
    bump-pitoune) shift; bump_pitoune "$@" ;;
    *) die "usage: web-shared-mirror.sh sync --mirror <dir> [--rev <rev>] [--check] [--adopt] | bump-pitoune --pitoune <dir> --sha <sha>" ;;
esac
