#!/usr/bin/env bash
# ABOUTME: Pins carry-forward-frontend-assets.sh, which lends the deployed frontend image's chunks to the next build
# ABOUTME: Runs it against stub gcloud and docker over fixture images, then checks publish-images.yml calls it before the build
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Pinned here:
#   - a first deploy (no deployed-dev) carries nothing, records the build's own
#     asset list, and exits 0;
#   - the previous image's own assets land in the new dist, a file the new
#     build made is never overwritten, and index.html is never carried;
#   - one generation only: what the previous image itself carried stays behind;
#   - an image that predates the asset list has all of its assets carried;
#   - a failed listing, pull, a non-digest tag, a dist or previous image with no
#     assets of its own and an asset list naming a path outside assets/ each
#     exit 1;
#   - publish-images.yml runs the script after the dist is built and before the
#     image build reads it.
#
# Usage: scripts/ci/carry-forward-frontend-assets.test.sh — bash and awk, no network, no docker.

set -euo pipefail
cd "$(dirname "$0")/../.."
SCRIPT="./scripts/ci/carry-forward-frontend-assets.sh"
WORKFLOW=".github/workflows/publish-images.yml"
WORK=$(mktemp -d); trap 'rm -rf "${WORK}"' EXIT
FAILURES=0
pass() { printf '  ok   %s\n' "$1"; }
fail() { printf '  FAIL %s\n' "$1"; FAILURES=$(( FAILURES + 1 )); }

# ---------------------------------------------------------------------------
# Stubs. TAGS holds the `tags list` output; IMAGES/<digest>/ is that image's
# /usr/share/nginx/html. FAIL_LIST and FAIL_PULL inject refusals.
# ---------------------------------------------------------------------------
BIN="${WORK}/bin"; mkdir -p "${BIN}"
cat > "${BIN}/gcloud" <<'SH'
#!/usr/bin/env bash
[ "$1 $2 $3 $4" = "artifacts docker tags list" ] || { echo "unexpected gcloud call: $*" >&2; exit 2; }
[ "${FAIL_LIST:-}" != "1" ] || { echo "ERROR: PERMISSION_DENIED" >&2; exit 1; }
cat "${TAGS}"
SH
cat > "${BIN}/docker" <<'SH'
#!/usr/bin/env bash
case "$1" in
  pull) [ "${FAIL_PULL:-}" != "1" ] || { echo "manifest unknown" >&2; exit 1; }
        [ -d "${IMAGES}/${3##*@}" ] || { echo "not found: $3" >&2; exit 1; } ;;
  create) printf 'ctr-%s\n' "${2##*@sha256:}" ;;
  cp) id="${2%%:*}"; digest="sha256:${id#ctr-}"
      [ "${2#*:}" = "/usr/share/nginx/html/." ] || { echo "unexpected cp source: $2" >&2; exit 2; }
      cp -R "${IMAGES}/${digest}" "$3" ;;
  rm) : ;;
  *) echo "unexpected docker call: $*" >&2; exit 2 ;;
esac
SH
chmod +x "${BIN}/gcloud" "${BIN}/docker"

PKG="northamerica-northeast1-docker.pkg.dev/dravr-artifacts/dravr-images/frontend"
D1="sha256:1111111111111111111111111111111111111111111111111111111111111111"
D2="sha256:2222222222222222222222222222222222222222222222222222222222222222"
IMAGES="${WORK}/images"; TAGS="${WORK}/tags"

new_dist() {  # new_dist <dir> <asset...> — a built dist holding those assets
  local dir="$1"; shift
  rm -rf "${dir}"; mkdir -p "${dir}/assets"
  echo "NEW-INDEX" > "${dir}/index.html"
  for a in "$@"; do echo "new $a" > "${dir}/assets/$a"; done
}

run_cmd() {  # run_cmd [VAR=value ...] -- <command...>
  local vars=()
  while [ "$1" != "--" ]; do vars+=("$1"); shift; done
  shift
  set +e
  OUT=$(env PATH="${BIN}:${PATH}" TAGS="${TAGS}" IMAGES="${IMAGES}" ${vars[@]+"${vars[@]}"} "$@" 2>&1)
  RC=$?
  set -e
}
check() {  # check <name> <expected rc> <expected output fragment>
  if [ "${RC}" = "$2" ] && grep -qF -- "$3" <<<"${OUT}"; then pass "$1"
  else fail "$1 (rc=${RC}, want $2; output did not contain '$3')"; printf '%s\n' "${OUT}" | tail -3 | sed 's/^/       /'; fi
}
has() { if [ -f "$2" ]; then pass "$1"; else fail "$1 ($2 missing)"; fi; }
lacks() { if [ ! -e "$2" ]; then pass "$1"; else fail "$1 ($2 present)"; fi; }
content() { if [ "$(cat "$2")" = "$3" ]; then pass "$1"; else fail "$1 ($2 holds '$(cat "$2")', want '$3')"; fi; }

DIST="${WORK}/dist"

echo "carry-forward-frontend-assets: first deploy"
printf '1a2b3c4\t%s\n' "${D1}" > "${TAGS}"
mkdir -p "${IMAGES}"
new_dist "${DIST}" index-AAA.js index-AAA.css
run_cmd -- "${SCRIPT}" --package "${PKG}" --tag deployed-dev --dist "${DIST}"
check "no deployed-dev is a first deploy, not an error" 0 "its first deploy"
content "  ...and the build's own assets are recorded" "${DIST}/release-assets.txt" "$(printf 'assets/index-AAA.css\nassets/index-AAA.js')"

echo "carry-forward-frontend-assets: one generation back"
# The previous image built verdicts-OLD.js and shared-SAME.js, and itself
# carried ancient-GEN2.js from the release before it.
P="${IMAGES}/${D1}"; mkdir -p "${P}/assets/nested"
echo "OLD-INDEX" > "${P}/index.html"
echo "old verdicts" > "${P}/assets/verdicts-OLD.js"
echo "same" > "${P}/assets/shared-SAME.js"
echo "old nested" > "${P}/assets/nested/font-OLD.woff2"
echo "gen2" > "${P}/assets/ancient-GEN2.js"
printf 'assets/nested/font-OLD.woff2\nassets/shared-SAME.js\nassets/verdicts-OLD.js\n' > "${P}/release-assets.txt"
printf 'deployed-dev\t%s\n' "${D1}" > "${TAGS}"
new_dist "${DIST}" index-BBB.js shared-SAME.js
echo "same" > "${DIST}/assets/shared-SAME.js"
run_cmd -- "${SCRIPT}" --package "${PKG}" --tag deployed-dev --dist "${DIST}"
check "the previous release's own assets are carried" 0 "Carried 2 assets forward"
has "  ...verdicts-OLD.js resolves in the new image" "${DIST}/assets/verdicts-OLD.js"
has "  ...nested paths too" "${DIST}/assets/nested/font-OLD.woff2"
lacks "  ...what the previous image carried itself stays behind" "${DIST}/assets/ancient-GEN2.js"
content "  ...index.html is this build's" "${DIST}/index.html" "NEW-INDEX"
content "  ...the list names only this build's assets" "${DIST}/release-assets.txt" "$(printf 'assets/index-BBB.js\nassets/shared-SAME.js')"

new_dist "${DIST}" verdicts-OLD.js
run_cmd -- "${SCRIPT}" --package "${PKG}" --tag deployed-dev --dist "${DIST}"
check "a name this build made is never overwritten" 0 "differs between"
content "  ...this build's copy wins" "${DIST}/assets/verdicts-OLD.js" "new verdicts-OLD.js"

echo "carry-forward-frontend-assets: an image from before the list"
L="${IMAGES}/${D2}"; mkdir -p "${L}/assets"
echo "OLD-INDEX" > "${L}/index.html"
echo "legacy a" > "${L}/assets/a-LEG.js"; echo "legacy b" > "${L}/assets/b-LEG.css"
printf 'deployed-dev\t%s\n' "${D2}" > "${TAGS}"
new_dist "${DIST}" index-CCC.js
run_cmd -- "${SCRIPT}" --package "${PKG}" --tag deployed-dev --dist "${DIST}"
check "every asset of a pre-list image is its own" 0 "Carried 2 assets forward"
has "  ...a-LEG.js carried" "${DIST}/assets/a-LEG.js"

echo "carry-forward-frontend-assets: failures are loud"
new_dist "${DIST}" index-DDD.js
run_cmd FAIL_LIST=1 -- "${SCRIPT}" --package "${PKG}" --tag deployed-dev --dist "${DIST}"
check "a failed tag listing fails" 1 "could not list the tags"
run_cmd FAIL_PULL=1 -- "${SCRIPT}" --package "${PKG}" --tag deployed-dev --dist "${DIST}"
check "a failed pull fails" 1 "could not pull"
printf 'deployed-dev\tlatest\n' > "${TAGS}"
run_cmd -- "${SCRIPT}" --package "${PKG}" --tag deployed-dev --dist "${DIST}"
check "a tag that is not a digest fails" 1 "not one digest"
printf 'assets/../index.html\n' > "${P}/release-assets.txt"
printf 'deployed-dev\t%s\n' "${D1}" > "${TAGS}"
run_cmd -- "${SCRIPT}" --package "${PKG}" --tag deployed-dev --dist "${DIST}"
check "a listed path outside assets/ fails" 1 "not a path under assets/"
content "  ...and index.html is untouched" "${DIST}/index.html" "NEW-INDEX"
printf 'assets/gone-XXX.js\n' > "${P}/release-assets.txt"
run_cmd -- "${SCRIPT}" --package "${PKG}" --tag deployed-dev --dist "${DIST}"
check "a listed file the image lacks fails" 1 "holds no regular file"
new_dist "${DIST}" "odd name.js"
run_cmd -- "${SCRIPT}" --package "${PKG}" --tag deployed-dev --dist "${DIST}"
check "an own asset the next release could not carry fails this build" 1 "not a name the next release can carry"
new_dist "${DIST}"
run_cmd -- "${SCRIPT}" --package "${PKG}" --tag deployed-dev --dist "${DIST}"
check "a dist whose assets/ is empty fails" 1 "holds no files"
new_dist "${DIST}" index-EEE.js
: > "${P}/release-assets.txt"
run_cmd -- "${SCRIPT}" --package "${PKG}" --tag deployed-dev --dist "${DIST}"
check "a previous image naming no assets of its own fails" 1 "names no assets of its own"
rm -rf "${DIST}/assets"
run_cmd -- "${SCRIPT}" --package "${PKG}" --tag deployed-dev --dist "${DIST}"
check "a dist with no assets/ fails" 1 "no assets/ directory"

echo "carry-forward-frontend-assets: workflow wiring"
# Reads the workflow as data: nothing runs publish-images.yml before a real
# deploy, and the step order (dist built, then carried into, then imaged) is a
# parity between two artefacts no runtime joins — the script and its caller.
line_of() { awk -v pat="$1" 'index($0, pat) { print NR; exit }' "${WORKFLOW}"; }
DIST_LINE=$(line_of "- name: Build frontend dist")
CARRY_LINE=$(line_of "./scripts/ci/carry-forward-frontend-assets.sh --package")
BUILD_LINE=$(line_of "- name: Build and push frontend image")
if [ -n "${DIST_LINE}" ] && [ -n "${CARRY_LINE}" ] && [ -n "${BUILD_LINE}" ] \
   && [ "${DIST_LINE}" -lt "${CARRY_LINE}" ] && [ "${CARRY_LINE}" -lt "${BUILD_LINE}" ]; then
  pass "publish-frontend carries assets after the dist build and before the image build"
else
  fail "publish-frontend does not run the carry-forward between the dist and image builds (dist=${DIST_LINE:-?} carry=${CARRY_LINE:-?} build=${BUILD_LINE:-?})"
fi
# shellcheck disable=SC2016 # the literal workflow text, not an expansion
if grep -qF -- '--tag deployed-dev --dist "${DIST_DIR}"' "${WORKFLOW}"; then
  pass "  ...from deployed-dev into the built dist"
else
  fail "  ...the call does not read deployed-dev into the built dist"
fi

if [ "${FAILURES}" -gt 0 ]; then
  echo "❌ ${FAILURES} carry-forward check(s) failed"
  exit 1
fi
echo "✅ carry-forward-frontend-assets checks passed"
