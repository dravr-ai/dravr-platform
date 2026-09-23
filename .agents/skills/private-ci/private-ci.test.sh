#!/usr/bin/env bash
# ABOUTME: Drives private-ci.sh end to end against a stubbed gh and a fixture repo
# ABOUTME: Asserts the one thing that matters on every path — the repo ends private

# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The first version of private-ci.sh left a repo PUBLIC after every successful
# run: its EXIT trap read cmd_run's locals after the function had returned, and
# under `set -u` errored out before flipping anything (dravr-meteo, 2026-09-23,
# ~13 minutes public). Only the red-CI path — which exits inside the function —
# worked, and that was the path reasoned about. So this runs the real script on
# every path, with `gh` replaced by a stand-in that keeps each repo's visibility
# in a file, and checks that file at the end.
#
# Usage: .agents/skills/private-ci/private-ci.test.sh   (bash, git; no network)

set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="${HERE}/private-ci.sh"
WORK=$(mktemp -d); trap 'rm -rf "${WORK}"' EXIT
FAILURES=0
pass() { printf '  ok   %s\n' "$1"; }
fail() { printf '  FAIL %s\n' "$1"; FAILURES=$((FAILURES + 1)); }

STATE="${WORK}/state"; mkdir -p "${STATE}/vis"
BIN="${WORK}/bin"; mkdir -p "${BIN}"
cat > "${BIN}/gh" <<'SH'
#!/usr/bin/env bash
# Visibility per repo lives in $STATE/vis/<repo>. CI_CONCL / REL_CONCL decide
# how the dispatched CI (run 111) and release (run 222) conclude.
echo "gh $*" >> "${STATE}/calls"
repo_of() { local r="$1"; r="${r#repos/}"; r="${r#dravr-ai/}"; echo "${r%%/*}"; }
case "$1 $2" in
  "repo edit")
    r="${3#dravr-ai/}"; v=""; prev=""
    for a in "$@"; do [ "${prev}" = "--visibility" ] && v="$a"; prev="$a"; done
    echo "${v}" > "${STATE}/vis/${r}"; exit 0 ;;
  "workflow run")
    r=""; prev=""; for a in "$@"; do [ "${prev}" = "-R" ] && r="${a#dravr-ai/}"; prev="$a"; done
    case "$3" in ci.yml) id=111 ;; *) id=222 ;; esac
    echo "https://github.com/dravr-ai/${r}/actions/runs/${id}"; exit 0 ;;
  "run cancel") exit 0 ;;
  "run view")
    id="$3"
    case "$*" in
      *"--json status"*) echo completed ;;
      *"--json conclusion"*) [ "${id}" = 111 ] && echo "${CI_CONCL}" || echo "${REL_CONCL}" ;;
      *"--json headSha"*) echo "abcdef0123456789" ;;
      *"--json jobs"*) [ "${id}" = 111 ] && [ "${CI_CONCL}" != success ] && echo "   ✗ Tests: failure" || echo "   Bump Version & Tag: ${REL_CONCL}" ;;
      *"--log"*) echo "test result: ok. 5 passed; 0 failed" ;;
    esac
    exit 0 ;;
esac
if [ "$1" = api ]; then
  path="$2"; r=$(repo_of "${path}")
  case "${path}" in
    */tags) echo "v9.9.9" ;;
    *"/actions/runs?status=queued"*) : ;;
    *"/actions/runs?"*) echo 0 ;;
    *)
      [ -f "${STATE}/vis/${r}" ] || exit 1
      case "$*" in *".visibility"*) cat "${STATE}/vis/${r}" ;; *) echo "${r}" ;; esac ;;
  esac
  exit 0
fi
exit 0
SH
chmod +x "${BIN}/gh"

make_checkout() {  # make_checkout <repo> [secret-line]
  local dir="${WORK}/co-$1"; rm -rf "${dir}"; mkdir -p "${dir}"
  ( cd "${dir}" && git init -q && git config user.email t@t && git config user.name t
    printf 'fn main() {}\n' > main.rs; [ -n "${2:-}" ] && printf '%s\n' "$2" >> main.rs
    git add -A && git commit -qm init
    git remote add origin "https://github.com/dravr-ai/$1.git"
    git update-ref refs/remotes/origin/main HEAD )
  echo "${dir}"
}

run() {  # run <repo> <start-visibility> <ci> <release-concl> [args...]
  local repo="$1" vis="$2"; CI_CONCL="$3"; REL_CONCL="$4"; shift 4
  echo "${vis}" > "${STATE}/vis/${repo}"; : > "${STATE}/calls"
  set +e
  OUT=$(PATH="${BIN}:${PATH}" STATE="${STATE}" CI_CONCL="${CI_CONCL}" REL_CONCL="${REL_CONCL}" \
        TMPDIR="${WORK}/tmp" bash "${SCRIPT}" run "${repo}" "$@" 2>&1)
  RC=$?
  set -e 2>/dev/null; set +e
  FINAL=$(cat "${STATE}/vis/${repo}")
}
mkdir -p "${WORK}/tmp"
expect() {  # expect <name> <rc> <final visibility> <output fragment>
  if [ "${RC}" = "$2" ] && [ "${FINAL}" = "$3" ] && grep -qF -- "$4" <<<"${OUT}"; then pass "$1"
  else fail "$1 (rc=${RC} want $2, ends ${FINAL} want $3, output lacks '$4')"; printf '%s\n' "${OUT}" | tail -4 | sed 's/^/       /'; fi
}

CO=$(make_checkout dravr-demo)
run dravr-demo private success success "${CO}"
expect "green CI ends private — the path the first version got wrong" 0 private "→ private"

run dravr-demo private failure success "${CO}"
expect "red CI ends private and says it stays private until a new push" 1 private "STAYS private until a new push"

run dravr-demo private success success "${CO}" --release minor
expect "green CI then a green release ends private" 0 private "release: success"

run dravr-demo private success failure "${CO}" --release minor
expect "a failed release ends private" 1 private "release: failure"

run dravr-demo public success success "${CO}"
expect "an already-public repo is refused and left as it was" 1 public "already public"
grep -q "repo edit" "${STATE}/calls" && fail "  ...but it flipped visibility" || pass "  ...without touching visibility"

mkdir -p "${WORK}/tmp/dravr-private-ci-locks/dravr-demo" && echo "$$ other-session" > "${WORK}/tmp/dravr-private-ci-locks/dravr-demo/holder"
run dravr-demo private success success "${CO}"
expect "a live lock holder refuses a second window" 1 private "is locked by pid"
rm -rf "${WORK}/tmp/dravr-private-ci-locks/dravr-demo"

run dravr-demo private success success "${CO}"
[ ! -d "${WORK}/tmp/dravr-private-ci-locks/dravr-demo" ] && pass "the lock is released after a run" || fail "the lock outlived the run"

SECRET=$(make_checkout dravr-leaky 'const KEY: &str = "AKIAABCDEFGHIJKLMNOP";')
echo private > "${STATE}/vis/dravr-leaky"
run dravr-leaky private success success "${SECRET}"
expect "a secret-shaped line in history stops the run before any flip" 1 private "scan found secret-shaped content"
grep -q "repo edit" "${STATE}/calls" && fail "  ...but it flipped visibility" || pass "  ...without touching visibility"

echo private > "${STATE}/vis/dravr-carnet"
run dravr-carnet private success success "${CO}"
expect "dravr-carnet is refused" 1 private "never made public"

echo
[ "${FAILURES}" -eq 0 ] && { echo "all private-ci scenarios pass"; exit 0; }
echo "${FAILURES} scenario(s) failed"; exit 1
