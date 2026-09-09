#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Compile-free detection of twinned server-surface divergence — a wire
# ABOUTME: field one auth surface accepts and its paired twin surface does not.

# WHY THIS EXISTS
# ---------------
# Every other phantom-surface scan asks "is this declared thing unreachable?".
# This defect is the inverse and passes all of them: NOTHING is unreachable.
# Two structurally parallel server surfaces — a token/bearer admin route table
# in crates/pierre-routes-admin and a cookie/session one in
# crates/pierre-routes-web-admin — each declare their own request DTO, each
# deserialize live traffic, each are served, each have a real client, and both
# call the same pierre-services function. A capability added to ONE of them is a
# half-finished capability, and every counter in the tree is non-zero.
#
#   Recurrence (feature/invite-email, registre#405): AllowEmailRequest gained
#   `#[serde(default)] pub send_invite: bool` in pierre-routes-admin, read at
#   handlers/users.rs to mail the sign-up link. The identically-named struct in
#   pierre-routes-web-admin — the surface the browser admin console posts to —
#   was never touched, so `pierre-cli user allow --send-invite` can mail the
#   invite and the human operator clicking the button in the console cannot.
#   serde ignores unknown fields by default and neither struct sets
#   deny_unknown_fields, so the console's request succeeds with the field
#   silently dropped: the failure mode is a 200.
#
#   Standing recurrence found by the same pairing: ApproveUserRequest carries
#   create_default_tenant/tenant_name/tenant_slug on the token surface and
#   `reason` alone on the cookie surface, so a console approval cannot provision
#   the user's default tenant.
#
# check-phantom-surfaces.sh cannot see any of this, and no scope change to it
# would help. Its scan 3 predicate is count(read sites)==0 and the added field
# HAS a read site; its scan 4 predicate is "does a client mention this path" and
# both clients do; its one binary parity predicate has its operand pools
# hardcoded to frontend/src vs frontend-mobile/src. Detection needs a BINARY
# predicate over a PAIR of live peer declarations, plus a pairing relation that
# says which two declarations are peers. That relation is what this script
# derives.
#
# HOW A PAIR IS ESTABLISHED (two relations, unioned — see the header of the
# PAIRING section for the adversarial reason both are needed)
#   R1  two handlers in DIFFERENT crates call the same `pierre_services::<mod>::<fn>`
#   R2  two handlers in DIFFERENT crates take an axum body/query extractor whose
#       type has the SAME NAME and resolves to two different declarations
#
# MODES
#   check-twin-surfaces.sh              — report the standing stock, exit 0
#   check-twin-surfaces.sh <BASE_REF>   — additionally FAIL when the diff against
#                                         BASE_REF introduces a NEW divergence
#
# Exit 0  no new divergence (standing stock reported, not blessed)
# Exit 1  this change introduced a divergence, OR the scan verified nothing
#         (zero handlers, zero structs, zero pierre-services functions, or zero
#         comparable twin pairs) — a scan that resolved nothing must not report
#         success, the check-lockfile-duplicates.sh rule.
#
# Portable on purpose: the runners' awk is mawk, so no gawk-only builtins.

set -uo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
if [[ -d "$SCRIPT_DIR/../../crates" ]]; then
    PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
else
    # Running from outside the repo tree (a scratch copy): anchor on the repo
    # the working directory belongs to rather than guessing.
    PROJECT_ROOT="$( git rev-parse --show-toplevel 2>/dev/null )"
fi
if [[ -z "${PROJECT_ROOT:-}" || ! -d "$PROJECT_ROOT/crates" ]]; then
    echo "cannot locate the workspace root (no crates/ directory)" >&2
    exit 1
fi
cd "$PROJECT_ROOT" || exit 1

BASE_REF="${1:-}"
FAILED=false

echo -e "${BLUE}==== Twinned Surface Divergence (static) ====${NC}"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ---------------------------------------------------------------------------
# awk pass 1 — handler signatures and pierre-services call sites
# ---------------------------------------------------------------------------
# Emits, tab separated:
#   FN    file line fnname extractor-types(comma separated, may be empty)
#   CALL  file line fnname mod::fn
# A signature is accumulated until its parenthesis depth returns to zero, because
# every axum handler in this tree spells its extractors one per line. `Path<..>`
# is deliberately excluded: a path parameter is part of the route string, not of
# the deserialized body, so a difference there is a route-shape question.
cat > "$TMP/fns.awk" <<'AWK'
FNR == 1 { insig = 0; curfn = ""; sig = "" }

{
  line = $0
  sub(/\/\/.*$/, "", line)
}

insig == 1 {
  sig = sig " " line
  paren += depth(line)
  if (paren <= 0) { insig = 0; emit_fn() }
  next
}

/^[ \t]*(pub([ \t]*\([^)]*\))?[ \t]+)?(async[ \t]+)?fn[ \t]+[A-Za-z0-9_]+/ {
  n = line
  sub(/^[ \t]*/, "", n)
  sub(/^pub([ \t]*\([^)]*\))?[ \t]+/, "", n)
  sub(/^async[ \t]+/, "", n)
  sub(/^fn[ \t]+/, "", n)
  sub(/[^A-Za-z0-9_].*$/, "", n)
  curfn = n; fnline = FNR
  sig = line
  paren = depth(line)
  if (paren > 0) { insig = 1 } else { emit_fn() }
  next
}

{
  s = line
  while (match(s, /[a-z][a-z0-9_]*::[a-z][a-z0-9_]*[ \t]*\(/)) {
    tok = substr(s, RSTART, RLENGTH)
    s = substr(s, RSTART + RLENGTH)
    sub(/[ \t]*\($/, "", tok)
    if (curfn != "") printf "CALL\t%s\t%d\t%s\t%s\n", FILENAME, FNR, curfn, tok
  }
}

function depth(l,   i, c, n) {
  n = 0
  for (i = 1; i <= length(l); i++) {
    c = substr(l, i, 1)
    if (c == "(") n++
    else if (c == ")") n--
  }
  return n
}

function emit_fn(   s, types, t) {
  types = ""
  s = sig
  while (match(s, /(Json|Query|Form)[ \t]*<[^<>]*>/)) {
    t = substr(s, RSTART, RLENGTH)
    s = substr(s, RSTART + RLENGTH)
    sub(/^(Json|Query|Form)[ \t]*</, "", t)
    sub(/>$/, "", t)
    gsub(/[ \t]/, "", t)
    sub(/^.*::/, "", t)
    if (t != "") types = (types == "" ? t : types "," t)
  }
  printf "FN\t%s\t%d\t%s\t%s\n", FILENAME, fnline, curfn, types
  sig = ""
}
AWK

# ---------------------------------------------------------------------------
# awk pass 2 — struct declarations and their field-name sets
# ---------------------------------------------------------------------------
# Emits: file line StructName field1,field2,...
cat > "$TMP/structs.awk" <<'AWK'
FNR == 1 { instruct = 0; name = ""; fields = "" }

instruct == 1 {
  l = $0
  sub(/\/\/.*$/, "", l)
  brace += gsub(/\{/, "{", l) - gsub(/\}/, "}", l)
  if (brace <= 0) {
    printf "%s\t%d\t%s\t%s\n", FILENAME, sline, name, fields
    instruct = 0; next
  }
  if (l ~ /^[ \t]*(pub([ \t]*\([^)]*\))?[ \t]+)?[a-z_][A-Za-z0-9_]*[ \t]*:/) {
    f = l
    sub(/^[ \t]*/, "", f)
    sub(/^pub([ \t]*\([^)]*\))?[ \t]+/, "", f)
    sub(/[ \t]*:.*$/, "", f)
    if (f != "") fields = (fields == "" ? f : fields "," f)
  }
  next
}

/^[ \t]*(pub([ \t]*\([^)]*\))?[ \t]+)?struct[ \t]+[A-Za-z0-9_]+/ {
  l = $0
  sub(/\/\/.*$/, "", l)
  n = l
  sub(/^[ \t]*/, "", n)
  sub(/^pub([ \t]*\([^)]*\))?[ \t]+/, "", n)
  sub(/^struct[ \t]+/, "", n)
  sub(/[^A-Za-z0-9_].*$/, "", n)
  if (l ~ /\{/) {
    name = n; sline = FNR; fields = ""
    brace = gsub(/\{/, "{", l) - gsub(/\}/, "}", l)
    if (brace > 0) { instruct = 1 }
    else { printf "%s\t%d\t%s\t%s\n", FILENAME, FNR, n, "" }
  } else {
    printf "%s\t%d\t%s\t%s\n", FILENAME, FNR, n, ""
  }
}
AWK

# ---------------------------------------------------------------------------
# awk pass 3 — pair the twins, resolve each side's DTO, compare field sets
# ---------------------------------------------------------------------------
# Inputs, in this order: structs.tsv  fns.tsv  svcfns.txt
# Emits one record per comparable pair:
#   VERDICT relation label Afile Aline Afields Bfile Bline Bfields
# with VERDICT in SAME | DIVERGENT | OPAQUE | AMBIGUOUS | NOBODY.
cat > "$TMP/pair.awk" <<'AWK'
function crate_of(path,   p) { split(path, p, "/"); return p[2] }

# Resolve a type name to a declaration index, preferring the handler's own file,
# then its own crate, then the workspace. Returns an index, "OPAQUE" when no
# struct of that name exists anywhere (serde_json::Value, a primitive, a type
# alias), or "AMBIG" when several equally-close declarations disagree on fields.
function resolve(t, hfile, hcrate,   n, i, arr, cand, cn, uniqf) {
  if (!(t in byname)) return "OPAQUE"
  n = split(byname[t], arr, " ")
  for (i = 1; i <= n; i++) if (sfile[arr[i]] == hfile) return arr[i]
  cn = 0
  for (i = 1; i <= n; i++) if (crate_of(sfile[arr[i]]) == hcrate) { cn++; cand[cn] = arr[i] }
  if (cn == 0) { cn = n; for (i = 1; i <= n; i++) cand[i] = arr[i] }
  if (cn == 1) return cand[1]
  uniqf = ""
  for (i = 1; i <= cn; i++) {
    if (uniqf == "") uniqf = sfields[cand[i]]
    else if (sfields[cand[i]] != uniqf) return "AMBIG"
  }
  return cand[1]
}

BEGIN {
  # ROUTECRATES is derived by the caller: every crate whose src declares an axum
  # `.route(`. It is what makes a "surface" a surface. Without it, pierre-services'
  # own internal calls count as a third crate and a service function reached from
  # two handlers in ONE crate reads as a cross-crate twin — the first false
  # positive this scan produced (admin_ops::transition_user_status paired
  # ApproveUserRequest against SuspendUserRequest, both in pierre-routes-admin).
  n = split(ROUTECRATES, rc, " ")
  for (i = 1; i <= n; i++) if (rc[i] != "") routecrate[rc[i]] = 1
  nroutecrates = n
}

FILENAME ~ /structs\.tsv$/ {
  ns++
  sfile[ns] = $1; sline[ns] = $2; sname[ns] = $3; sfields[ns] = $4
  byname[$3] = ($3 in byname) ? byname[$3] " " ns : ns
  next
}

FILENAME ~ /fns\.tsv$/ {
  if (!(crate_of($2) in routecrate)) next
  if ($1 == "FN") {
    fndto[$2 "\t" $4] = $5
    nfn++
    if ($5 != "") {
      n = split($5, tarr, ",")
      for (i = 1; i <= n; i++) r2site[++nr2] = tarr[i] "\t" $2 "\t" $4
    }
  } else if ($1 == "CALL") {
    callsite[++ncs] = $5 "\t" $2 "\t" $4
  }
  next
}

# svcfns.txt
{ svcfn[$0] = 1; nsvc++ }

END {
  if (nroutecrates == 0) { print "STALE\troute-crates"; exit }
  if (nfn == 0)          { print "STALE\thandlers";     exit }
  if (ns == 0)           { print "STALE\tstructs";      exit }
  if (nsvc == 0)         { print "STALE\tservices";     exit }

  # ---- relation R1: a shared pierre_services callee reached from 2+ crates ----
  for (i = 1; i <= ncs; i++) {
    split(callsite[i], a, "\t")
    if (!(a[1] in svcfn)) continue
    c = crate_of(a[2])
    if (!((a[1] SUBSEP c) in r1seen)) { r1seen[a[1] SUBSEP c] = 1; r1n[a[1]]++ }
    r1sites[a[1]] = (a[1] in r1sites) ? r1sites[a[1]] "\n" a[2] "\t" a[3] : a[2] "\t" a[3]
  }
  for (g in r1n) if (r1n[g] >= 2) collect("R1", g, r1sites[g])

  # ---- relation R2: a same-named extractor type used in 2+ crates ------------
  for (i = 1; i <= nr2; i++) {
    split(r2site[i], a, "\t")
    c = crate_of(a[2])
    if (!((a[1] SUBSEP c) in r2seen)) { r2seen[a[1] SUBSEP c] = 1; r2n[a[1]]++ }
    r2sites[a[1]] = (a[1] in r2sites) ? r2sites[a[1]] "\n" a[2] "\t" a[3] : a[2] "\t" a[3]
  }
  for (g in r2n) if (r2n[g] >= 2) collect("R2", g, r2sites[g])

  for (i = 1; i <= nout; i++) {
    k = outkey[i]
    printf "%s\t%s\t%s\t%s\n", outv[k], outrel[k], outlabel[k], outsites[k]
  }
}

function note(verdict, rel, label,   k) {
  k = verdict "\t" label
  if (k in outv) return
  outkey[++nout] = k
  outv[k] = verdict; outrel[k] = rel; outlabel[k] = label
  outsites[k] = "-\t0\t-\t-\t0\t-"
}

# One comparison per unordered pair of DIFFERENT crates, each contributing
# exactly one declaration. A crate contributing two declarations for the same
# label is not a 1:1 twin correspondence and is reported as unresolvable rather
# than guessed at.
function collect(rel, label, siteblob,   lines, n, i, j, a, t, m, tt, d, cr,
                                          cn, cd, cdseen, crates, ncr,
                                          x, y, dx, dy, opaque, nobody, multi, withd, k, v) {
  n = split(siteblob, lines, "\n")
  ncr = 0; opaque = 0; nobody = 0
  for (i = 1; i <= n; i++) {
    split(lines[i], a, "\t")
    cr = crate_of(a[1])
    if (!(cr in cn)) { cn[cr] = 0; crates[++ncr] = cr }
    t = fndto[a[1] "\t" a[2]]
    if (t == "") { nobody++; continue }
    m = split(t, tt, ",")
    for (j = 1; j <= m; j++) {
      if (rel == "R2" && tt[j] != label) continue
      d = resolve(tt[j], a[1], cr)
      if (d == "OPAQUE") { opaque++; continue }
      if (d == "AMBIG")  { note("AMBIGUOUS", rel, label); return }
      if (!((cr SUBSEP d) in cdseen)) {
        cdseen[cr SUBSEP d] = 1
        cn[cr]++
        cd[cr SUBSEP cn[cr]] = d
      }
    }
  }
  multi = 0; withd = 0
  for (x = 1; x <= ncr; x++) {
    if (cn[crates[x]] > 1) multi = 1
    if (cn[crates[x]] == 1) withd++
  }
  if (multi)      { note("AMBIGUOUS", rel, label); return }
  if (withd == 0) { note(opaque > 0 ? "OPAQUE" : "NOBODY", rel, label); return }
  if (withd < ncr) { note("PARTIAL", rel, label) }
  if (withd < 2)  { return }
  for (x = 1; x < ncr; x++) {
    if (cn[crates[x]] != 1) continue
    for (y = x + 1; y <= ncr; y++) {
      if (cn[crates[y]] != 1) continue
      dx = cd[crates[x] SUBSEP 1]; dy = cd[crates[y] SUBSEP 1]
      if (dx == dy) { v = "SAME" } else { v = (sfields[dx] == sfields[dy]) ? "SAME" : "DIVERGENT" }
      k = sfile[dx] ":" sline[dx] "|" sfile[dy] ":" sline[dy]
      if (k in outv) {
        # The same declaration pair found by the other relation: record both,
        # report once. Two independent relations agreeing is a stronger pairing,
        # not a second finding.
        if (index("," outrel[k] ",", "," rel ",") == 0) outrel[k] = outrel[k] "," rel
        continue
      }
      outkey[++nout] = k
      outv[k] = v; outrel[k] = rel; outlabel[k] = label
      outsites[k] = sfile[dx] "\t" sline[dx] "\t" (sfields[dx] == "" ? "-" : sfields[dx]) "\t" \
                    sfile[dy] "\t" sline[dy] "\t" (sfields[dy] == "" ? "-" : sfields[dy])
    }
  }
}
AWK

# ---------------------------------------------------------------------------
# Run the passes. Each is checked on its EXIT STATUS, separately from any grep
# of its output: a crashed extractor must not read as "found nothing".
# ---------------------------------------------------------------------------
if ! find crates/*/src -name '*.rs' -type f > "$TMP/files.txt" 2>"$TMP/err"; then
    echo -e "${RED}❌ cannot enumerate Rust sources: $(cat "$TMP/err")${NC}"
    exit 1
fi
FILE_N=$(grep -c . < "$TMP/files.txt" || true)
if [[ "$FILE_N" -eq 0 ]]; then
    echo -e "${RED}❌ Found zero Rust sources under crates/*/src — this scan is stale.${NC}"
    exit 1
fi

if ! ERR="$(xargs awk -f "$TMP/fns.awk" < "$TMP/files.txt" 2>&1 >"$TMP/fns.tsv")"; then
    echo -e "${RED}❌ handler extraction failed: ${ERR}${NC}"
    exit 1
fi
if ! ERR="$(xargs awk -f "$TMP/structs.awk" < "$TMP/files.txt" 2>&1 >"$TMP/structs.tsv")"; then
    echo -e "${RED}❌ struct extraction failed: ${ERR}${NC}"
    exit 1
fi

# The pierre-services function registry: the third artifact R1 pairs against.
# Neither twin is compared to the other directly — both are compared to the
# callee they share, which is what keeps the relation from being a name guess.
SERVICES_SRC="crates/pierre-services/src"
if [[ ! -d "$SERVICES_SRC" ]]; then
    echo -e "${RED}❌ ${SERVICES_SRC} not found — this scan is stale.${NC}"
    exit 1
fi
: > "$TMP/svcfns.txt"
for f in "$SERVICES_SRC"/*.rs; do
    m="$(basename "$f" .rs)"
    [[ "$m" == "lib" ]] && continue
    grep -hoE '^[[:space:]]*pub (async )?fn [a-z_0-9]+' "$f" 2>/dev/null \
        | sed -E 's/.*fn //' | sort -u \
        | sed -E "s#^#${m}::#" >> "$TMP/svcfns.txt"
done

# The surface registry: a crate is a server surface when it declares an axum
# route. Derived, not listed, so a new route crate is in scope the day it lands.
if ! ROUTE_CRATES="$(grep -rl '\.route(' crates/*/src --include='*.rs' 2>/dev/null \
        | cut -d/ -f2 | sort -u | tr '\n' ' ')"; then
    echo -e "${RED}❌ cannot derive the route-crate registry${NC}"
    exit 1
fi
if [[ -z "${ROUTE_CRATES// /}" ]]; then
    echo -e "${RED}❌ Derived zero route-serving crates — this scan is stale.${NC}"
    exit 1
fi

FN_N=$(grep -c '^FN' "$TMP/fns.tsv" || true)
STRUCT_N=$(grep -c . < "$TMP/structs.tsv" || true)
SVC_N=$(grep -c . < "$TMP/svcfns.txt" || true)

if ! ERR="$(awk -v ROUTECRATES="$ROUTE_CRATES" -f "$TMP/pair.awk" "$TMP/structs.tsv" "$TMP/fns.tsv" "$TMP/svcfns.txt" 2>&1 >"$TMP/pairs.tsv")"; then
    echo -e "${RED}❌ pair resolution failed: ${ERR}${NC}"
    exit 1
fi

if grep -q '^STALE' "$TMP/pairs.tsv"; then
    echo -e "${RED}❌ Scan verified nothing ($(cut -f2 < "$TMP/pairs.tsv" | tr '\n' ' ')) — this scan is stale.${NC}"
    exit 1
fi

SAME_N=$(grep -c '^SAME' "$TMP/pairs.tsv" || true)
DIV_N=$(grep -c '^DIVERGENT' "$TMP/pairs.tsv" || true)
OPAQUE_N=$(grep -c '^OPAQUE' "$TMP/pairs.tsv" || true)
AMBIG_N=$(grep -c '^AMBIGUOUS' "$TMP/pairs.tsv" || true)
NOBODY_N=$(grep -c '^NOBODY' "$TMP/pairs.tsv" || true)
PARTIAL_N=$(grep -c '^PARTIAL' "$TMP/pairs.tsv" || true)
COMPARED_N=$(( SAME_N + DIV_N ))

echo "   ${FILE_N} sources, ${FN_N} functions, ${STRUCT_N} structs, ${SVC_N} pierre-services fns"
echo "   route-serving crates: ${ROUTE_CRATES}"
echo "   twin pairs: ${COMPARED_N} comparable, ${NOBODY_N} with no request body,"
echo "               ${PARTIAL_N} body on one side only, ${OPAQUE_N} opaque, ${AMBIG_N} unresolvable"

# FAIL CLOSED. A twin scan that resolved zero comparable pairs is
# indistinguishable from a tree with no twins, so it must not report success.
if [[ "$COMPARED_N" -eq 0 ]]; then
    echo -e "${RED}❌ Resolved zero comparable twin pairs — this scan is stale.${NC}"
    echo -e "${RED}   Either the pairing relations no longer match this tree's shape, or an${NC}"
    echo -e "${RED}   extractor stopped parsing. Fix the scan; do not treat this as a pass.${NC}"
    exit 1
fi

# ---------------------------------------------------------------------------
# marker_for  — is the lacking side's gap registered?
# ---------------------------------------------------------------------------
# A LIMITATION(registre#n) marker must sit in the 15 lines immediately above the
# LACKING struct's own declaration AND name the missing field somewhere in that
# same window. Both conditions, so one marker at the top of a file cannot
# silence every struct in it, and a marker on the right struct cannot silence a
# second field added later. Prints "registre#n" on a hit, nothing otherwise.
marker_for() {
    local file="$1" line="$2" field="$3" from win
    from=$(( line - 15 )); [[ "$from" -lt 1 ]] && from=1
    win="$(sed -n "${from},${line}p" "$file" 2>/dev/null)" || return 0
    grep -qE 'LIMITATION\(registre#[0-9]+\)' <<< "$win" || return 0
    grep -qF -- "$field" <<< "$win" || return 0
    grep -oE 'registre#[0-9]+' <<< "$win" | head -1
}

# fields_only_in A B  — set difference of two comma-separated field lists
fields_only_in() {
    local a="$1" b="$2"
    tr ',' '\n' <<< "$a" | sort -u > "$TMP/fa.txt"
    tr ',' '\n' <<< "$b" | sort -u > "$TMP/fb.txt"
    comm -23 "$TMP/fa.txt" "$TMP/fb.txt" | grep -v '^$' || true
}

# ---------------------------------------------------------------------------
# Standing stock — reported, never blessed
# ---------------------------------------------------------------------------
if [[ "$DIV_N" -eq 0 ]]; then
    echo -e "${GREEN}✅ Twin surfaces: all ${COMPARED_N} comparable pairs accept the same field set.${NC}"
else
    echo -e "${YELLOW}⚠️  Twin surfaces whose request field sets differ (${DIV_N} of ${COMPARED_N}):${NC}"
    while IFS=$'\t' read -r _v rel label af al afl bf bl bfl; do
        echo "   ${label}  (paired by ${rel})"
        echo "      ${af}:${al}  [${afl}]"
        echo "      ${bf}:${bl}  [${bfl}]"
        while read -r f; do
            [[ -z "$f" ]] && continue
            issue="$(marker_for "$bf" "$bl" "$f")"
            if [[ -n "$issue" ]]; then
                echo -e "      ${YELLOW}⚠️  ${f}: registered gap (${issue}) at ${bf}:${bl}${NC}"
            else
                echo "      only $(cut -d/ -f2 <<< "$af") accepts: ${f}"
            fi
        done < <(fields_only_in "$afl" "$bfl")
        while read -r f; do
            [[ -z "$f" ]] && continue
            issue="$(marker_for "$af" "$al" "$f")"
            if [[ -n "$issue" ]]; then
                echo -e "      ${YELLOW}⚠️  ${f}: registered gap (${issue}) at ${af}:${al}${NC}"
            else
                echo "      only $(cut -d/ -f2 <<< "$bf") accepts: ${f}"
            fi
        done < <(fields_only_in "$bfl" "$afl")
    done < <(grep '^DIVERGENT' "$TMP/pairs.tsv")
fi

if [[ "$AMBIG_N" -gt 0 ]]; then
    echo -e "${YELLOW}⚠️  ${AMBIG_N} twin pair(s) whose DTO could not be resolved to one declaration:${NC}"
    cut -f2,3 < <(grep '^AMBIGUOUS' "$TMP/pairs.tsv") | sed 's/^/   /'
fi

# ---------------------------------------------------------------------------
# Gate: fail when THIS change introduces a divergence
# ---------------------------------------------------------------------------
if [[ -n "$BASE_REF" ]]; then
    echo ""
    echo -e "${BLUE}---- New twin divergence in this change (vs ${BASE_REF}) ----${NC}"

    # A caller may pass either a bare base ("origin/main") or a complete range
    # ("origin/main...origin/feature/x"). Appending ...HEAD to the latter yields
    # a three-ended revision that git rejects, which failed closed but made the
    # range form unusable — so only supply the missing end.
    if [[ "$BASE_REF" == *..* ]]; then
        DIFF_RANGE="$BASE_REF"
    else
        DIFF_RANGE="${BASE_REF}...HEAD"
    fi

    if ! DIFF="$(git diff -U0 "$DIFF_RANGE" -- 'crates/*/src/*.rs' 2>&1)"; then
        echo -e "${RED}❌ git diff against ${DIFF_RANGE} failed: ${DIFF}${NC}"
        exit 1
    fi
    # Per-file added and removed field declarations, so a field added on one side
    # is attributed to the file that gained it rather than to the diff as a whole.
    if ! ERR="$(awk '
        /^\+\+\+ b\// { f = substr($0, 7); next }
        /^--- a\//    { g = substr($0, 7); next }
        /^\+[ \t]*(pub([ \t]*\([^)]*\))?[ \t]+)?[a-z_][A-Za-z0-9_]*[ \t]*:/ {
            l = $0; sub(/^\+[ \t]*/, "", l)
            sub(/^pub([ \t]*\([^)]*\))?[ \t]+/, "", l); sub(/[ \t]*:.*$/, "", l)
            if (l != "") printf "ADD\t%s\t%s\n", f, l
            next
        }
        /^-[ \t]*(pub([ \t]*\([^)]*\))?[ \t]+)?[a-z_][A-Za-z0-9_]*[ \t]*:/ {
            l = $0; sub(/^-[ \t]*/, "", l)
            sub(/^pub([ \t]*\([^)]*\))?[ \t]+/, "", l); sub(/[ \t]*:.*$/, "", l)
            if (l != "") printf "DEL\t%s\t%s\n", g, l
            next
        }
    ' <<< "$DIFF" 2>&1 >"$TMP/diff_fields.tsv")"; then
        echo -e "${RED}❌ diff field extraction failed: ${ERR}${NC}"
        exit 1
    fi

    touched() {  # touched ADD|DEL <file> <field>
        grep -qF -- "$1	$2	$3" "$TMP/diff_fields.tsv"
    }

    NEW_FOUND=false
    while IFS=$'\t' read -r _v rel label af al afl bf bl bfl; do
        # Both directions of the asymmetry, twice over: a field this change ADDED
        # to one twin, and a field this change REMOVED from the other while the
        # first kept it. The second is the case no other gate can see — nothing
        # breaks, the surface simply stops offering the capability.
        for dir in ab ba; do
            if [[ "$dir" == "ab" ]]; then
                hav_f="$af"; hav_l="$al"; hav_x="$afl"; lac_f="$bf"; lac_l="$bl"; lac_x="$bfl"
            else
                hav_f="$bf"; hav_l="$bl"; hav_x="$bfl"; lac_f="$af"; lac_l="$al"; lac_x="$afl"
            fi
            while read -r f; do
                [[ -z "$f" ]] && continue
                hav_c="$(cut -d/ -f2 <<< "$hav_f")"; lac_c="$(cut -d/ -f2 <<< "$lac_f")"
                why=""
                if touched ADD "$hav_f" "$f"; then
                    why="'${f}' was added to ${hav_c} and not to its twin ${lac_c}"
                elif touched DEL "$lac_f" "$f"; then
                    why="'${f}' was removed from ${lac_c} while ${hav_c} still accepts it"
                fi
                [[ -z "$why" ]] && continue
                issue="$(marker_for "$lac_f" "$lac_l" "$f")"
                if [[ -n "$issue" ]]; then
                    echo -e "${YELLOW}⚠️  Registered gap (${issue}): ${label}.${f} is absent from ${lac_f}:${lac_l}${NC}"
                else
                    echo -e "${RED}❌ ${label}: ${why}${NC}"
                    echo -e "${RED}     has it:      ${hav_f}:${hav_l}  [${hav_x}]${NC}"
                    echo -e "${RED}     lacks it:    ${lac_f}:${lac_l}  [${lac_x}]${NC}"
                    echo -e "${RED}     both reach:  ${label}  (paired by ${rel})${NC}"
                    NEW_FOUND=true
                fi
            done < <(fields_only_in "$hav_x" "$lac_x")
        done
    done < <(grep '^DIVERGENT' "$TMP/pairs.tsv")

    if [[ "$NEW_FOUND" == "true" ]]; then
        echo ""
        echo -e "${RED}Two server surfaces reach the same service and accept different requests.${NC}"
        echo -e "${RED}One auth surface can do the thing and its twin cannot, and nothing breaks:${NC}"
        echo -e "${RED}serde drops the unknown field and the twin answers 200, so no test fails${NC}"
        echo -e "${RED}and no phantom-surface scan sees it — both surfaces are fully live.${NC}"
        echo -e "${YELLOW}Add the field to both twins in this same change, or move the capability${NC}"
        echo -e "${YELLOW}down into the pierre-services function they already share so the${NC}"
        echo -e "${YELLOW}asymmetry becomes unrepresentable. If the gap is deliberate, register it:${NC}"
        echo -e "${YELLOW}a LIMITATION(registre#issue): line in the 15 lines above the LACKING${NC}"
        echo -e "${YELLOW}struct's declaration, naming that field.${NC}"
        FAILED=true
    else
        echo -e "${GREEN}✅ This change adds no wire field to one server surface without its twin,${NC}"
        echo -e "${GREEN}   and takes none away from one twin while the other keeps it.${NC}"
    fi
fi

echo -e "${GREEN}TWIN-SURFACE-SCAN: COMPLETED ${COMPARED_N} pair comparison(s)${NC}"

if [[ "$FAILED" == "true" ]]; then
    echo -e "${RED}❌ TWIN SURFACE CHECK FAILED${NC}"
    exit 1
fi
exit 0
