#!/usr/bin/env bash
# ABOUTME: Runs CI or a release on a private dravr repo while private-repo Actions minutes are exhausted
# ABOUTME: Scan history, flip public, run, flip private on exit — a red CI leaves it private until a new push
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# ChefFamille's procedure (carnet#534, 2026-09-23): when the org's private-repo
# Actions minutes are exhausted, a private repo's CI or release runs by making the
# repo public for the run and private again as soon as it is terminal. If CI fails,
# the repo goes back to private and stays private until a new push — fix, push,
# then run this again.
#
#   private-ci.sh check                          has GitHub refused private-repo jobs in the last 24h?
#   private-ci.sh scan  <repo> <local-checkout>  full-history secret scan, fails closed
#   private-ci.sh run   <repo> <local-checkout> [--workflow ci.yml] [--release patch|minor|major]
#                                                [--scan-reviewed] [--dry-run]
#
# <repo> is the bare name (dravr-enforme). dravr-carnet and dravr-vault are refused:
# carnet issue bodies name security residuals, the vault is the team's notes.

set -uo pipefail

ORG="dravr-ai"
REFUSED="dravr-carnet dravr-vault"
BILLING_MARK="recent account payments have failed or your spending limit"

die() { echo "❌ $*" >&2; exit 1; }
say() { echo "   $*"; }

guard_repo() {
  local repo="$1"
  [ -n "${repo}" ] || die "name a repo (dravr-<x>)"
  for r in ${REFUSED}; do [ "${repo}" = "${r}" ] && die "${repo} is never made public"; done
  gh api "repos/${ORG}/${repo}" --jq .name >/dev/null 2>&1 || die "${ORG}/${repo} not reachable with this gh auth"
}

# A run's failed jobs that never executed a step — GitHub's shape for a job it
# refused to start. Printed as names, empty when none.
unstarted_jobs() {
  gh api "repos/${ORG}/$1/actions/runs/$2/jobs?per_page=100" \
    --jq '[.jobs[] | select(.conclusion == "failure" and ((.steps // []) | length) == 0) | .name] | join(", ")' 2>/dev/null
}

# Is the private-repo minutes limit reached? GitHub exposes no "remaining minutes"
# for this (the billing usage API reports monthly totals, not a cap), so the only
# direct evidence is a refusal: a failed job with no step run and GitHub's billing
# annotation. Looks across every private repo in the org over the last 24 hours —
# one repo that happened not to run anything proves nothing, so a quiet org is
# reported as "not seen", never as "fine".
cmd_check() {
  local since; since=$(date -u -v-24H +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -d '24 hours ago' +%Y-%m-%dT%H:%M:%SZ)
  local repos; repos=$(gh api "orgs/${ORG}/repos?type=private&per_page=100" --jq '.[] | select(.archived|not) | .name' 2>/dev/null)
  [ -n "${repos}" ] || die "could not list ${ORG}'s private repos with this gh auth"
  local checked=0 latest="" where=""
  for repo in ${repos}; do
    checked=$((checked + 1))
    while IFS=$'\t' read -r id created; do
      [ -n "${id}" ] || continue
      [ -n "$(unstarted_jobs "${repo}" "${id}")" ] || continue
      gh run view "${id}" -R "${ORG}/${repo}" 2>/dev/null | grep -q "${BILLING_MARK}" || continue
      if [ -z "${latest}" ] || [[ "${created}" > "${latest}" ]]; then latest="${created}"; where="${repo} run ${id}"; fi
      break
    done < <(gh api "repos/${ORG}/${repo}/actions/runs?status=failure&created=>=${since}&per_page=10" \
               --jq '.workflow_runs[] | "\(.id)\t\(.created_at)"' 2>/dev/null)
  done
  if [ -n "${latest}" ]; then
    echo "⛔ private-repo Actions minutes are exhausted — GitHub refused to start jobs, latest ${latest} (${where})."
    echo "   Private repos run CI with: private-ci.sh run <repo> <local-checkout>"
    return 3
  fi
  echo "✅ no billing refusal in the last 24h across ${checked} private repos."
  echo "   Not proof the limit is clear — an org that ran nothing looks the same. A refusal names itself:"
  echo "   \"${BILLING_MARK}\"."
}

cmd_scan() {
  local repo="$1" dir="$2"; guard_repo "${repo}"
  [ -d "${dir}/.git" ] || [ -f "${dir}/.git" ] || die "${dir} is not a git checkout of ${repo}"
  git -C "${dir}" fetch -q --all 2>/dev/null
  local origin; origin=$(git -C "${dir}" remote get-url origin 2>/dev/null)
  case "${origin}" in *"${ORG}/${repo}"*) ;; *) die "${dir}'s origin is ${origin}, not ${ORG}/${repo}";; esac
  local commits; commits=$(git -C "${dir}" rev-list --all | wc -l | tr -d ' ')
  local files strings
  files=$(git -C "${dir}" log --all --name-only --format= | sort -u \
          | grep -iE '(^|/)\.env($|\.)|\.pem$|\.key$|id_rsa|\.p12$|credentials\.json|service-account.*\.json|cookies?\.json' || true)
  strings=$(git -C "${dir}" log --all -p --format= | grep '^+' \
          | grep -oE 'sk_(live|test)_[A-Za-z0-9]{10,}|whsec_[A-Za-z0-9]{10,}|AKIA[0-9A-Z]{16}|ghp_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}|xox[baprs]-[A-Za-z0-9-]{10,}|-----BEGIN [A-Z ]*PRIVATE KEY-----|AIza[0-9A-Za-z_-]{35}|(password|passwd|api[_-]?key|access_token|refresh_token|client_secret|secret|token)["'"'"' ]*[:=]["'"'"' ]*"[^"$]{6,}"' \
          | sort -u || true)
  echo "scanned ${commits} commits of ${repo}"
  if [ -z "${files}${strings}" ]; then echo "✅ nothing secret-shaped in the history"; return 0; fi
  echo "⚠️  secret-shaped content — review every line (test fixtures are fine; a real credential means STOP and tell ChefFamille):"
  [ -n "${files}" ] && printf '%s\n' "${files}" | sed 's/^/   file: /'
  [ -n "${strings}" ] && printf '%s\n' "${strings}" | sed -E 's/^(.{0,60}).*/   text: \1…/'
  return 2
}

flip() { gh repo edit "${ORG}/$1" --visibility "$2" --accept-visibility-change-consequences >/dev/null 2>&1; gh api "repos/${ORG}/$1" --jq .visibility; }

wait_run() {
  until [ "$(gh run view "$1" -R "${ORG}/$2" --json status --jq .status 2>/dev/null)" = completed ]; do sleep 30; done
  gh run view "$1" -R "${ORG}/$2" --json conclusion --jq .conclusion
}

wait_idle() {
  until [ "$(gh api "repos/${ORG}/$1/actions/runs?per_page=30" \
            --jq '[.workflow_runs[] | select(.status=="queued" or .status=="in_progress" or .status=="waiting" or .status=="pending")] | length' 2>/dev/null || echo 1)" = 0 ]; do
    sleep 20
  done
}

cmd_run() {
  local repo="$1" dir="$2"; shift 2
  local workflow="ci.yml" release="" reviewed="" dry=""
  while [ $# -gt 0 ]; do case "$1" in
    --workflow) workflow="$2"; shift ;;
    --release) release="$2"; shift ;;
    --scan-reviewed) reviewed=1 ;;
    --dry-run) dry=1 ;;
    *) die "unknown option $1" ;;
  esac; shift; done
  guard_repo "${repo}"
  [ -z "${release}" ] || case "${release}" in patch|minor|major) ;; *) die "--release is patch, minor or major";; esac

  # Checked first, before the scan: a repo that is already public either always
  # is (it needs none of this) or has another session's window open.
  if [ "$(gh api "repos/${ORG}/${repo}" --jq .visibility)" = "public" ]; then
    die "${repo} is already public — either it always is (CI runs normally; nothing to do) or another session's window is open (wait for it to flip back)"
  fi
  cmd_scan "${repo}" "${dir}"; local rc=$?
  if [ ${rc} -eq 2 ] && [ -z "${reviewed}" ]; then
    die "scan found secret-shaped content — review it; re-run with --scan-reviewed only if every line is a fixture or placeholder"
  fi
  [ ${rc} -le 2 ] || exit 1
  local sha; sha=$(git -C "${dir}" rev-parse "origin/main")
  echo "plan: ${repo} public → ${workflow} on main (${sha:0:9})${release:+ → Release bump=${release}} → private"
  [ -n "${dry}" ] && { echo "(dry run — nothing changed)"; return 0; }

  # One window per repo. Two sessions running this on one repo would each flip
  # it back to private from their own trap — the first to finish would cut the
  # other's run off mid-flight. Guarded twice:
  #   - on GitHub: a private repo that is already public has a window open, from
  #     this machine or another; wait for it rather than share it.
  #   - on this machine: an atomic mkdir lock naming its holder, stale when the
  #     holder's pid is gone.
  local lockroot="${TMPDIR:-/tmp}/dravr-private-ci-locks" lock
  lock="${lockroot}/${repo}"
  mkdir -p "${lockroot}"
  if ! mkdir "${lock}" 2>/dev/null; then
    local holder; holder=$(cat "${lock}/holder" 2>/dev/null)
    if [ -n "${holder}" ] && kill -0 "${holder%% *}" 2>/dev/null; then
      die "${repo} is locked by pid ${holder} — another session is running its window"
    fi
    rm -rf "${lock}" && mkdir "${lock}" || die "could not take the lock on ${repo}"
    say "took over a stale lock (${holder:-no holder recorded})"
  fi
  echo "$$ ${CLAUDE_SESSION_ID:-session} $(date -u +%H:%M:%SZ)" > "${lock}/holder"

  # Private again on ANY exit: success, red CI, a failed step, Ctrl-C. Only a
  # repo this run made public is flipped back, and the lock goes with it.
  local flipped=""
  trap '[ -n "${flipped}" ] && echo "   ${repo} → $(flip "${repo}" private) at $(date -u +%H:%M:%SZ)"; rm -rf "${lock}"' EXIT
  echo "   ${repo} → $(flip "${repo}" public) at $(date -u +%H:%M:%SZ)"; flipped=1

  # Runs queued before the flip can never start — billing was decided at queue time.
  for id in $(gh api "repos/${ORG}/${repo}/actions/runs?status=queued&per_page=30" --jq '.workflow_runs[].id' 2>/dev/null); do
    gh run cancel "${id}" -R "${ORG}/${repo}" >/dev/null 2>&1 && say "cancelled run ${id}, queued before the flip"
  done

  local url id concl head
  url=$(gh workflow run "${workflow}" -R "${ORG}/${repo}" --ref main 2>&1 | tail -1); id="${url##*/}"
  [[ "${id}" =~ ^[0-9]+$ ]] || die "could not dispatch ${workflow}: ${url}"
  say "${workflow} → run ${id}"
  concl=$(wait_run "${id}" "${repo}")
  head=$(gh run view "${id}" -R "${ORG}/${repo}" --json headSha --jq .headSha)
  local tests; tests=$(gh run view "${id}" -R "${ORG}/${repo}" --log 2>/dev/null | grep -oE 'test result: ok\. [0-9]+ passed' | awk '{s+=$4} END {print s+0}')
  echo "${workflow}: ${concl} on ${head:0:9} — ${tests} tests passed"
  if [ "${concl}" != "success" ]; then
    gh run view "${id}" -R "${ORG}/${repo}" --json jobs --jq '.jobs[] | select(.conclusion != "success" and .conclusion != "skipped") | "   ✗ \(.name): \(.conclusion)"'
    echo "⛔ CI is not green — ${repo} goes back to private and STAYS private until a new push. Fix, push, run this again."
    exit 1
  fi

  if [ -n "${release}" ]; then
    # Satellites name the workflow "Release"; dispatching by name covers them all.
    url=$(gh workflow run Release -R "${ORG}/${repo}" -f "bump=${release}" 2>&1 | tail -1)
    id="${url##*/}"
    [[ "${id}" =~ ^[0-9]+$ ]] || die "could not dispatch the release: ${url}"
    say "release → run ${id}"
    concl=$(wait_run "${id}" "${repo}")
    gh run view "${id}" -R "${ORG}/${repo}" --json jobs --jq '.jobs[] | "   \(.name): \(.conclusion)"'
    echo "release: ${concl} — latest tag $(gh api "repos/${ORG}/${repo}/tags" --jq '.[0].name')"
    [ "${concl}" = "success" ] || exit 1
  fi

  # Anything the run set off in this repo (an announce, a lane) finishes while public.
  wait_idle "${repo}"
}

case "${1:-}" in
  check) cmd_check ;;
  scan)  shift; cmd_scan "${1:-}" "${2:-}" ;;
  run)   shift; [ $# -ge 2 ] || die "run <repo> <local-checkout> [options]"; cmd_run "$@" ;;
  *) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 1 ;;
esac
