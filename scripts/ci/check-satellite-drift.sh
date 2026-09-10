#!/usr/bin/env bash
# ABOUTME: Fails when a dravr-* satellite pin trails a real release nobody moved it onto
# ABOUTME: Enumerates pins from the manifests, resolves each repo's latest released tag, judges diamonds against their upstream

# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists. dravr-cageux shipped v0.12.0 through v0.18.0 on 8-9 September
# 2026 while this repo stayed pinned at v0.13.0 — five real releases behind, in one
# week, with nothing red. `dependency-check.yml` could not see it: it runs
# `cargo upgrade --dry-run`, which reads crates.io versions and says nothing about a
# `git = "...", tag = "vX.Y.Z"` dependency. Every other satellite is watched by its
# own bump lane (embacle, enforme, contremaitre, photograveur, tronc); cageux, canot,
# commere and meteo have no lane at all, so nothing was ever going to notice.
# The cageux pin this first caught is carnet#422.
#
# What a "release" is here, and why it is not "the newest tag". A satellite's release
# job pushes its TAG before its BRANCH, so a failed run leaves an orphaned tag that
# still resolves and still builds — dravr-cageux's own CHANGELOG carries
# "v0.8.3 — NEVER RELEASED, DO NOT PIN" for exactly that reason. And a tag can exist
# with no GitHub release behind it when the publish half fails. So a version counts
# as released only when BOTH hold: a GitHub release exists for the tag, and the tag
# is an ancestor of that repo's default branch. Checking either alone has already
# misfired in this fleet.
#
# Diamonds. Three of the pins here are not this repo's to move. dravr-enforme
# v0.1.56 pins dravr-equilibre v0.2.8, dravr-riviere v0.2.4 and dravr-sciotte
# v0.12.0, and the platform pins all three at those same versions on purpose:
# moving one alone puts two copies of that crate in Cargo.lock and hard-fails the
# duplicate gate. carnet#323 was that exact failure with dravr-equilibre at 0.2.4
# beside 0.2.5, found twenty minutes into a gate as E0053 in health_sync.rs. So a
# diamond pin is judged against what the PINNED enforme pins, never against that
# repo's own latest release — a naive "the pin is behind" check on sciotte is what
# sciotte-pin-check.yml used to file, for a bump that could not be done alone, and
# it was removed for being noise.
#
# The pins are read from the manifests rather than a list in this file, for the
# reason contremaitre-bump.yml records: a hardcoded list left one crate behind, put
# two revs in Cargo.lock and hard-failed Security Audit on main. Both manifest
# shapes are searched — the root [workspace.dependencies] table and per-crate
# declarations — so this works before and after that migration.
#
# Exit 0 clean, 1 on drift, 2 when the scan could not verify anything. A scan that
# resolved no pins at all is a failure, never a pass: absence of a finding is
# indistinguishable from absence of a run.

set -uo pipefail

# --offline runs only the half that needs no network: reconciling satellites.toml
# against the manifests, and refusing a satellite pin in a file the bump chain does
# not scan. That half is deterministic and costs nothing, so it gates every push;
# the release comparison below needs ~9 GitHub API calls per pin and is a daily job.
#
# The split exists because the drift half stopped being a useful push gate the day
# every satellite got a lane. A laned pin is reported and never failed on — a lane's
# lag is transient by construction — so with eleven lanes the only things that can
# fail here are a diamond disagreeing with its upstream, a declaration out of sync
# with the tree, and a GitHub API hiccup. Gating pushes on the last of those reds
# main for the network.
OFFLINE=false
[ "${1:-}" = "--offline" ] && OFFLINE=true

REPO_OWNER="dravr-ai"
FAILURES=0
CHECKED=0

# The diamond table and the lane table used to live here as two `declare -A` blocks.
# They are the same per-satellite knowledge a bump lane needs, and keeping two copies
# is how they drift: this file said dravr-stripe moves by "tronc-bump.yml (lockstep)"
# while no lane moved it at all. Both now come out of satellites.toml, which is the
# single declaration the bump spine also reads.
PIN_SH="$(dirname "${BASH_SOURCE[0]}")/satellite-pin.sh"
[ -x "$PIN_SH" ] || { echo "❌ ${PIN_SH} missing — cannot read satellites.toml"; exit 2; }

# Value is the repo whose pinned manifest decides what this pin must equal.
upstream_of() { "$PIN_SH" upstream "$1" 2>/dev/null; }

# A lane's lag is transient by construction, so a satellite with a lane is reported,
# never failed on — failing here would red main for a bot's schedule.
lane_of() { "$PIN_SH" lane "$1" 2>/dev/null; }

# GITHUB_TOKEN in CI, gh auth locally. Three attempts: a single `dial tcp ... i/o
# timeout` is common enough that without a retry this gate reports "could not verify"
# and reds main for the network rather than for a pin. Still returns empty on real
# failure — the callers fail closed on that, deliberately.
api() { # $1=path
  local attempt out
  for attempt in 1 2 3; do
    if [ -n "${GITHUB_TOKEN:-}" ]; then
      out=$(curl -sf --max-time 20 -H "Authorization: Bearer ${GITHUB_TOKEN}" \
                 -H "Accept: application/vnd.github+json" "https://api.github.com/$1" 2>/dev/null)
    else
      out=$(gh api "$1" 2>/dev/null)
    fi
    if [ -n "$out" ]; then printf '%s' "$out"; return 0; fi
    sleep $(( attempt * 2 ))
  done
  return 1
}

# The newest release whose tag is an ancestor of the repo's default branch.
latest_released_tag() { # $1=repo
  local repo="$1" rels default_branch tag
  rels=$(api "repos/${REPO_OWNER}/${repo}/releases?per_page=20") || return 1
  [ -z "$rels" ] && return 1
  default_branch=$(api "repos/${REPO_OWNER}/${repo}" | jq -r '.default_branch // empty')
  [ -z "$default_branch" ] && return 1
  while read -r tag; do
    [ -z "$tag" ] && continue
    # compare/<branch>...<tag> reports "behind"/"identical" when the tag is an
    # ancestor; "diverged"/"ahead" means it is not on the branch.
    local status
    status=$(api "repos/${REPO_OWNER}/${repo}/compare/${default_branch}...${tag}" | jq -r '.status // empty')
    case "$status" in
      behind|identical) printf '%s\n' "$tag"; return 0 ;;
      "") return 1 ;;                       # could not compare — refuse to guess
    esac
  done < <(printf '%s' "$rels" | jq -r '.[] | select(.draft==false) | .tag_name')
  return 1
}

# Some repos tag without ever cutting a GitHub release — dravr-meteo has two tags and
# zero releases. Their pins still deserve judging, so fall back to the newest vX.Y.Z
# tag that is an ancestor of the default branch. The caller says which evidence it
# used, because "a release exists" is the stronger claim and the two must not blur.
latest_ancestor_tag() { # $1=repo
  local repo="$1" default_branch tag status
  default_branch=$(api "repos/${REPO_OWNER}/${repo}" | jq -r '.default_branch // empty') || return 1
  [ -z "$default_branch" ] && return 1
  while read -r tag; do
    [ -z "$tag" ] && continue
    status=$(api "repos/${REPO_OWNER}/${repo}/compare/${default_branch}...${tag}" | jq -r '.status // empty')
    case "$status" in
      behind|identical) printf '%s\n' "$tag"; return 0 ;;
      "") return 1 ;;
    esac
  done < <(api "repos/${REPO_OWNER}/${repo}/tags?per_page=100" \
           | jq -r '.[].name | select(test("^v[0-9]+\\.[0-9]+\\.[0-9]+$"))' \
           | sort -Vr)
  return 1
}

echo "==== Satellite pin drift ===="

# name<TAB>tag, from both manifest shapes, deduped.
PINS=$(grep -hoE '^(dravr-[a-z-]+|photograveur) = \{[^}]*git = "[^"]*"[^}]*tag = "v[0-9]+\.[0-9]+\.[0-9]+"' \
         Cargo.toml crates/*/Cargo.toml 2>/dev/null \
       | sed -E 's/^([a-z-]+) = \{(.*)$/\1\t\2/' \
       | awk -F'\t' '{
           name = $1; rest = $2;
           # A renamed dependency carries the real crate in `package = "..."`; the key
           # is only a local alias. dravr-equilibre-sync is dravr-equilibre.
           if (match(rest, /package = "[^"]+"/))
             { pkg = substr(rest, RSTART, RLENGTH); gsub(/package = "|"/, "", pkg); name = pkg }
           if (match(rest, /tag = "v[0-9]+\.[0-9]+\.[0-9]+"/))
             { tag = substr(rest, RSTART, RLENGTH); gsub(/tag = "|"/, "", tag); print name "\t" tag }
         }' | sort -u)

if [ -z "$PINS" ]; then
  echo "❌ resolved no dravr-* tag pins at all — the manifest shape changed and this scan verified nothing"
  exit 2
fi

# Reconcile the manifests against satellites.toml, both directions, before judging
# anything. A pin with no stanza is invisible to the bump spine — it gets no lane and
# no diamond rule, which is how dravr-cageux sat five releases behind. A stanza with
# no pin is a lane pointed at a dependency this repo dropped, which will resolve a
# version forever and report success. Neither is drift, so neither is exit 1: the
# declaration is out of sync with the tree and the scan cannot be trusted, which is
# what exit 2 means everywhere else in this repo.
RECONCILE=0
while IFS=$'\t' read -r name _; do
  [ -z "$name" ] && continue
  if [ -z "$("$PIN_SH" repo "$name" 2>/dev/null)" ]; then
    echo "❌ ${name} is pinned in a manifest but has no stanza in satellites.toml"
    RECONCILE=$(( RECONCILE + 1 ))
  fi
done <<< "$PINS"

while read -r name; do
  [ -z "$name" ] && continue
  # crates.io and git-rev satellites carry no tag, so they are absent from PINS by
  # construction. Everything that DOES carry one must be found.
  case "$("$PIN_SH" source "$name" 2>/dev/null)" in
    git-tag|git-tag+version) ;;
    *) continue ;;
  esac
  if ! printf '%s' "$PINS" | awk -F'\t' -v n="$name" '$1==n{f=1} END{exit !f}'; then
    echo "❌ satellites.toml declares ${name} but no manifest pins it"
    RECONCILE=$(( RECONCILE + 1 ))
  fi
done < <("$PIN_SH" names)

# A pin in a manifest nothing scans is the one failure with no safety net. The
# rewriter would move every site it can see, report success, and leave that one
# behind; and `check-lockfile-duplicates.sh` cannot catch it either, because a
# manifest outside `members = ["crates/*"]` resolves against its own lockfile and
# never reaches the workspace one. So convert a silent miss into a loud one: any
# satellite-shaped pin outside the scanned scope fails the scan.
# -prune, not -not -path: a filter still DESCENDS into the directory it excludes, so
# `find . -not -path "./target/*"` walks every build artefact in the repo before
# discarding it — 14 seconds against a warm target/, which is not a push gate.
# Pruning never enters them at all.
OUT_OF_SCOPE=$(find . \
                 \( -name target -o -name node_modules -o -name .git \) -prune -o \
                 -name Cargo.toml -print 2>/dev/null \
               | grep -vE '^\./Cargo\.toml$|^\./crates/[^/]+/Cargo\.toml$' \
               | while read -r m; do
                   if grep -qE '^(dravr-[a-z-]+|photograveur|embacle(-[a-z-]+)?) = ' "$m" 2>/dev/null; then
                     echo "$m"
                   fi
                 done)
if [ -n "$OUT_OF_SCOPE" ]; then
  echo "❌ a satellite pin lives in a manifest the bump chain does not scan:"
  printf '   %s\n' $OUT_OF_SCOPE
  echo "   satellite-pin.sh scans Cargo.toml and crates/*/Cargo.toml. A pin outside that"
  echo "   would be left behind by every bump while the run reported success."
  RECONCILE=$(( RECONCILE + 1 ))
fi

if [ "$RECONCILE" -gt 0 ]; then
  echo
  echo "❌ satellites.toml and the manifests disagree on ${RECONCILE} satellite(s) — scan unverified"
  exit 2
fi

if [ "$OFFLINE" = "true" ]; then
  echo "✅ satellites.toml reconciles with the manifests ($(printf '%s' "$PINS" | grep -c . ) tag pin(s), every stanza accounted for)"
  exit 0
fi

while IFS=$'\t' read -r name tag; do
  [ -z "$name" ] && continue
  CHECKED=$(( CHECKED + 1 ))
  repo="$name"; [ "$name" = "photograveur" ] && repo="dravr-photograveur"

  upstream=$(upstream_of "$name")
  if [ -n "$upstream" ]; then
    up_tag=$(printf '%s' "$PINS" | awk -F'\t' -v u="$upstream" '$1==u{print $2}')
    if [ -z "$up_tag" ]; then
      echo "❌ ${name} ${tag}: pinned by ${upstream}, but this repo pins no ${upstream} to read it from"
      FAILURES=$(( FAILURES + 1 )); continue
    fi
    want=$(api "repos/${REPO_OWNER}/${upstream}/contents/Cargo.toml?ref=${up_tag}" \
           | jq -r '.content // empty' | base64 -d 2>/dev/null \
           | grep -oE "^${name} = \{[^}]*tag = \"v[0-9]+\.[0-9]+\.[0-9]+\"" \
           | grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+' | head -1)
    if [ -z "$want" ]; then
      echo "❌ ${name} ${tag}: could not read what ${upstream} ${up_tag} pins — refusing to report this as in sync"
      FAILURES=$(( FAILURES + 1 )); continue
    fi
    if [ "$want" = "$tag" ]; then
      echo "✅ ${name} ${tag} — matches ${upstream} ${up_tag} (diamond; moves only with it)"
    else
      echo "❌ ${name} ${tag} — ${upstream} ${up_tag} pins ${want}. Two copies of ${name} would resolve."
      echo "   Move both in one commit; $(lane_of "$upstream") is the lane that does it."
      FAILURES=$(( FAILURES + 1 ))
    fi
    continue
  fi

  # `release_evidence = "tag"` in satellites.toml says this repo cuts no GitHub
  # releases, so the ancestor-of-default-branch check is the whole existence proof.
  # It is strictly the weaker claim, which is why it is declared per satellite and
  # never inferred: silently falling back would let a repo that USED to cut releases
  # and stopped look the same as one that never did.
  if [ "$("$PIN_SH" evidence "$name" 2>/dev/null)" = "tag" ]; then
    evidence="tagged (repo cuts no GitHub releases)"
    latest=$(latest_ancestor_tag "$repo")
  else
    evidence="released"
    latest=$(latest_released_tag "$repo")
    if [ -z "$latest" ]; then
      latest=$(latest_ancestor_tag "$repo")
      evidence="tagged (no GitHub release resolved)"
    fi
  fi
  if [ -z "$latest" ]; then
    # Distinguish "this repo has no versions" from "this token cannot see it".
    # Every satellite is private, so a repo-scoped token 404s here and every pin
    # reports the same thing — which reads as total drift rather than as the
    # credential problem it is.
    if ! api "repos/${REPO_OWNER}/${repo}" >/dev/null; then
      echo "❌ ${name} ${tag}: cannot read ${REPO_OWNER}/${repo} at all — the repo is private, so this is almost certainly the token's scope, not drift"
    else
      echo "❌ ${name} ${tag}: could not resolve a released or tagged version for ${repo} — scan unverified, not clean"
    fi
    FAILURES=$(( FAILURES + 1 )); continue
  fi

  if [ "$latest" = "$tag" ]; then
    echo "✅ ${name} ${tag} — current (${evidence})"
  elif [ -n "$(lane_of "$name")" ]; then
    echo "⚠️  ${name} ${tag} — ${latest} is out; $(lane_of "$name") moves this, not reported as drift"
  else
    behind=$(api "repos/${REPO_OWNER}/${repo}/releases?per_page=100" \
             | jq -r --arg t "$tag" '[.[] | select(.draft==false) | .tag_name] | index($t) // empty')
    echo "❌ ${name} ${tag} — ${latest} is out (${evidence}) and nothing moves this pin (${behind:-?} release(s) behind)"
    FAILURES=$(( FAILURES + 1 ))
  fi
done <<< "$PINS"

echo
if [ "$CHECKED" -lt 1 ]; then
  echo "❌ checked no pins — refusing to report a pass"
  exit 2
fi
if [ "$FAILURES" -gt 0 ]; then
  echo "❌ ${FAILURES} of ${CHECKED} pin(s) drifted"
  exit 1
fi
echo "✅ satellite pins in sync (${CHECKED} checked)"
