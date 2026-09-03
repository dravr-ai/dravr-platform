#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Compile-free detection of phantom surfaces — declared capabilities
# ABOUTME: with no implementor (Rust traits) and no caller (api-client methods).

# WHY THIS EXISTS
# ---------------
# The dual rule in AGENTS.md: a capability predicate, enum variant, or trait
# method whose only callers are tests is a phantom surface. CI already enforces
# that for the canot messaging surface (supports_*/max_* predicates,
# MessageContent variants). It did not cover the two cheapest cases:
#
#   1. A Rust trait declared with ZERO implementors anywhere in the workspace.
#      Recurrence: pierre-chat-pipeline's QuotaGate and UsageRecorder — both
#      have live call sites in the pipeline (`if let Some(gate) = ...`) and
#      doc comments describing behaviour in the present tense, but nothing has
#      ever implemented them, so the branches are unreachable and the real
#      quota enforcement lives elsewhere (pierre-services/src/usage_counter.rs).
#   2. A @pierre/api-client domain method with ZERO production call sites, or
#      with call sites on only ONE of the two in-app clients.
#      Recurrence: authApi.refreshToken(), which posts grant_type=refresh_token
#      to an endpoint whose handler rejects that grant — dormant because no
#      caller has ever exercised it.
#      Recurrence: the whole 2026-08 parity survey. This scan used to pool web,
#      mobile, the shared packages and the SDK into ONE caller list, so a method
#      called from web alone read as "consumed" and every client-side parity gap
#      passed green. The pools are separate now: web-only and mobile-only are
#      reported as the gaps they are.
#
# Both were found by a manual cold read months after they were written. Neither
# is visible to a regression test: nothing broke, because nothing ran.
#
# MODES
#   check-phantom-surfaces.sh              — report the standing stock, exit 0
#   check-phantom-surfaces.sh <BASE_REF>   — additionally FAIL when the diff
#                                            against BASE_REF introduces a NEW
#                                            phantom surface
#
# The diff-scoped mode is the gate: it stops the stock from growing at the
# moment of authoring, which is the only moment the author has the context to
# fix it. Clearing the existing stock is a deletion decision per surface (some
# are superseded scaffolding, some are unfinished features) and is tracked in
# the dravr-ai/carnet register (carnet#17), not blessed by a list in this script.

set -uo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
cd "$PROJECT_ROOT" || exit 1

BASE_REF="${1:-}"
FAILED=false

echo -e "${BLUE}==== Phantom Surface Detection (static) ====${NC}"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ---------------------------------------------------------------------------
# Scan 1: Rust traits with zero implementors
# ---------------------------------------------------------------------------
# Declared traits. `^\s*(pub…)?trait Name` only matches a declaration site —
# `-> impl Trait`, `dyn Trait` and `where T: Trait` carry no `trait` keyword,
# and doc comments are excluded by anchoring the optional visibility modifier.
grep -rhoE '^[[:space:]]*(pub([[:space:]]*\([^)]*\))?[[:space:]]+)?(unsafe[[:space:]]+)?trait[[:space:]]+[A-Za-z0-9_]+' \
    crates/*/src --include='*.rs' 2>/dev/null \
    | grep -oE '[A-Za-z0-9_]+$' | sort -u > "$TMP/declared.txt"

# Implemented traits: the trait path in `impl [<generics>] Path[<args>] for`.
# Take the last `::` segment so `impl pierre_x::Foo for Bar` counts for `Foo`.
grep -rhoE '^[[:space:]]*impl([[:space:]]*<[^>]*>)?[[:space:]]+[A-Za-z0-9_:]+([[:space:]]*<[^>]*>)?[[:space:]]+for[[:space:]]' \
    crates --include='*.rs' 2>/dev/null \
    | sed -E 's/[[:space:]]*for[[:space:]]*$//' \
    | grep -oE '[A-Za-z0-9_:]+([[:space:]]*<[^>]*>)?$' \
    | sed -E 's/[[:space:]]*<.*//; s/.*:://' \
    | sort -u > "$TMP/implemented.txt"

comm -23 "$TMP/declared.txt" "$TMP/implemented.txt" > "$TMP/orphan_traits.txt"

DECLARED_N=$(grep -c . < "$TMP/declared.txt" || true)
ORPHAN_TRAIT_N=$(grep -c . < "$TMP/orphan_traits.txt" || true)

if [[ "$DECLARED_N" -eq 0 ]]; then
    echo -e "${RED}❌ Parsed zero trait declarations — this scan is stale.${NC}"
    FAILED=true
elif [[ "$ORPHAN_TRAIT_N" -eq 0 ]]; then
    echo -e "${GREEN}✅ Rust traits: all ${DECLARED_N} declared traits have an implementor.${NC}"
else
    echo -e "${YELLOW}⚠️  Rust traits with zero implementors (${ORPHAN_TRAIT_N} of ${DECLARED_N}):${NC}"
    while read -r t; do
        [[ -z "$t" ]] && continue
        SITE=$(grep -rnE "trait[[:space:]]+${t}\b" crates/*/src --include='*.rs' 2>/dev/null | head -1 | cut -d: -f1,2)
        echo "   $t  ($SITE)"
    done < "$TMP/orphan_traits.txt"
fi

# ---------------------------------------------------------------------------
# Scan 2: @pierre/api-client domain methods with zero production call sites
# ---------------------------------------------------------------------------
DOMAINS="packages/api-client/src/domains"

if [[ ! -d "$DOMAINS" ]]; then
    echo -e "${RED}❌ ${DOMAINS} not found — this scan is stale.${NC}"
    FAILED=true
else
    grep -rhoE '^[[:space:]]{4}async [a-zA-Z0-9_]+\(' "$DOMAINS"/*.ts 2>/dev/null \
        | sed -E 's/^[[:space:]]*async //; s/\($//' | sort -u > "$TMP/methods.txt"

    METHOD_N=$(grep -c . < "$TMP/methods.txt" || true)

    # Production consumers, per surface. `web` and `mobile` are the two in-app
    # clients; `shared` is everything that serves both (the api-client's own
    # core, the shared packages, the SDK). Test files and the definition files
    # themselves are excluded — a method whose only callers are tests is
    # precisely what the dual rule calls phantom.
    surface_files() {
        find "$@" \( -name '*.ts' -o -name '*.tsx' \) 2>/dev/null \
            | grep -vE '__tests__|\.test\.|\.spec\.' \
            | grep -v "^${DOMAINS}/"
    }
    surface_files frontend/src > "$TMP/files_web.txt"
    # `frontend-mobile/app` is expo-router's route tree: layouts and screens
    # that ship. Scanning only `src` read a call that moved into a layout as a
    # call that had disappeared — the i18n bundle fetch did exactly that when
    # its wrapper was deleted, and this pool said the method reached web alone.
    surface_files frontend-mobile/src frontend-mobile/app > "$TMP/files_mobile.txt"
    surface_files packages/api-client/src packages/shared-constants/src \
        packages/shared-types/src packages/mcp-types/src sdk/src > "$TMP/files_shared.txt"

    PROD_N=$(cat "$TMP"/files_*.txt 2>/dev/null | grep -c . || true)

    if [[ "$PROD_N" -eq 0 || "$METHOD_N" -eq 0 ]]; then
        echo -e "${RED}❌ Parsed zero api-client methods or zero production files — this scan is stale.${NC}"
        FAILED=true
    else
        # Matching `.name` rather than the bare identifier keeps by-reference
        # uses in — `queryFn: api.user.getProfile` still carries the dot —
        # while not counting an unrelated local variable of the same name as a
        # caller. `authApi.refreshToken` is exactly that case: the bare
        # identifier appears all over the auth code as a local, so an
        # identifier-level scan called it used when nothing calls it.
        # The trailing (non-word|end-of-line) group stops `.getVersion` from
        # matching `.getVersionDiff`.
        sed -E 's/^/\\./; s/$/([^a-zA-Z0-9_]|$)/' "$TMP/methods.txt" > "$TMP/patterns.txt"

        for surface in web mobile shared; do
            # NUL-delimited xargs from stdin: `xargs -a` is GNU-only and BSD
            # xargs rejects it, which silently produced an empty match set on
            # macOS and reported every method as an orphan.
            tr '\n' '\0' < "$TMP/files_${surface}.txt" \
                | xargs -0 grep -ohEf "$TMP/patterns.txt" 2>/dev/null \
                | sed -E 's/^\.//; s/[^a-zA-Z0-9_]$//' \
                | sort -u > "$TMP/used_${surface}.txt"
        done
        sort -u "$TMP/used_web.txt" "$TMP/used_mobile.txt" "$TMP/used_shared.txt" > "$TMP/used.txt"

        comm -23 "$TMP/methods.txt" "$TMP/used.txt" > "$TMP/orphan_methods.txt"
        # A parity gap: reached from one client, from neither the other client
        # nor any shared consumer that would serve both.
        comm -23 "$TMP/used_web.txt" "$TMP/used_mobile.txt" \
            | comm -23 - "$TMP/used_shared.txt" > "$TMP/web_only.txt"
        comm -23 "$TMP/used_mobile.txt" "$TMP/used_web.txt" \
            | comm -23 - "$TMP/used_shared.txt" > "$TMP/mobile_only.txt"

        USED_N=$(grep -c . < "$TMP/used.txt" || true)
        ORPHAN_METHOD_N=$(grep -c . < "$TMP/orphan_methods.txt" || true)
        WEB_ONLY_N=$(grep -c . < "$TMP/web_only.txt" || true)
        MOBILE_ONLY_N=$(grep -c . < "$TMP/mobile_only.txt" || true)

        # Completeness tripwire: both apps call login/logout/getStatus and many
        # more, so a near-empty match set means the scan itself broke, not that
        # the codebase calls nothing. Fail loudly rather than emit a wall of
        # false orphans that reads like a real finding.
        if [[ "$USED_N" -lt $(( METHOD_N / 4 )) ]]; then
            echo -e "${RED}❌ Usage scan implausible: only ${USED_N} of ${METHOD_N} methods matched in ${PROD_N} files.${NC}"
            echo -e "${YELLOW}   The scan machinery is broken (grep/xargs behaviour), not the codebase.${NC}"
            FAILED=true
            ORPHAN_METHOD_N=-1
        fi
        if [[ "$ORPHAN_METHOD_N" -eq -1 ]]; then
            : # scan broken; already reported
        elif [[ "$ORPHAN_METHOD_N" -eq 0 ]]; then
            echo -e "${GREEN}✅ api-client: all ${METHOD_N} domain methods have a production caller.${NC}"
        else
            echo -e "${YELLOW}⚠️  api-client methods with zero production callers (${ORPHAN_METHOD_N} of ${METHOD_N}):${NC}"
            while read -r m; do
                [[ -z "$m" ]] && continue
                SITE=$(grep -rnE "async ${m}\(" "$DOMAINS"/*.ts 2>/dev/null | head -1 | cut -d: -f1,2)
                echo "   $m  ($SITE)"
            done < "$TMP/orphan_methods.txt"
        fi

        if [[ "$ORPHAN_METHOD_N" -ne -1 ]]; then
            if [[ "$WEB_ONLY_N" -eq 0 && "$MOBILE_ONLY_N" -eq 0 ]]; then
                echo -e "${GREEN}✅ api-client parity: every called method is reached from both clients.${NC}"
            else
                echo -e "${YELLOW}⚠️  api-client methods called from one client only (${WEB_ONLY_N} web, ${MOBILE_ONLY_N} mobile):${NC}"
                while read -r m; do
                    [[ -z "$m" ]] && continue
                    echo "   web only:    $m"
                done < "$TMP/web_only.txt"
                while read -r m; do
                    [[ -z "$m" ]] && continue
                    echo "   mobile only: $m"
                done < "$TMP/mobile_only.txt"
            fi
        fi
    fi
fi

# ---------------------------------------------------------------------------
# Scan 3: deserialized config fields with zero read sites
# ---------------------------------------------------------------------------
# The quietest phantom of the three. An operator sets the key, the file parses,
# validation passes — and the value is discarded, so the config surface lies
# about what it controls. The 2026-06-03 due-diligence review found eight on
# PersonaContract alone; every one had survived two months of green CI.
#
# A read site is a `.field` access OUTSIDE the declaring file. Both exclusions
# are load-bearing: the declaration `pub field:` carries no dot, and the
# parent/child merge function sits beside the struct and touches every field —
# counting it would mark the whole struct consumed.
#
# The trade-off that buys: a field consumed only by a same-file method reads as
# orphaned (`inherits` is the standing example — the merge resolves it in
# place). Reporting those is deliberate, because the alternative misses real
# gaps: NotificationPolicy's tier_floor/digest are merged in-file and consumed
# nowhere, which is exactly the registered limitation this scan should surface.
# The gate below only fails on fields ADDED by the current diff, so a
# same-file-only consumer is a one-line explanation at review time, never a
# silent block.
#
# Scoped to structs parsed from YAML/TOML — operator-editable config. Provider
# wire DTOs also derive Deserialize, but an unread field there mirrors an
# upstream payload we don't control and is not a lie about our own config
# surface; including them buried the real signal under ~500 entries.
grep -rl 'serde_yaml\|serde_yml\|toml::from_str' crates/*/src --include='*.rs' 2>/dev/null \
    | while IFS= read -r file; do
        awk -v f="$file" '
            /^#\[derive\(/            { derive_line = $0; derive_at = NR }
            /^pub struct [A-Za-z0-9_]+[[:space:]]*\{/ {
                inside = (derive_at && NR - derive_at <= 4 && derive_line ~ /Deserialize/)
                next
            }
            inside && /^\}/           { inside = 0 }
            inside && /^[[:space:]]+pub [a-z_0-9]+:/ {
                match($0, /pub [a-z_0-9]+:/)
                print substr($0, RSTART + 4, RLENGTH - 5) "\t" f
            }
        ' "$file"
    done | sort -u > "$TMP/config_fields.txt"

# Every `.field` access in the tree, as `FILE:field`, collected in one pass.
grep -roE '\.[a-z_0-9]+' crates/*/src --include='*.rs' 2>/dev/null \
    | sed 's/:\./:/' | sort -u > "$TMP/field_reads.txt"

awk -F'\t' '
    NR == FNR {
        p = index($0, ":")
        if (p == 0) next
        file = substr($0, 1, p - 1); fld = substr($0, p + 1)
        if (!((fld, file) in seen)) { seen[fld, file] = 1; nfiles[fld]++ }
        next
    }
    {
        n = nfiles[$1] + 0
        if (n == 0 || (n == 1 && (($1, $2) in seen))) print $1 "\t" $2
    }
' "$TMP/field_reads.txt" "$TMP/config_fields.txt" > "$TMP/orphan_config_fields.txt"

CONFIG_N=$(grep -c . < "$TMP/config_fields.txt" || true)
ORPHAN_CONFIG_N=$(grep -c . < "$TMP/orphan_config_fields.txt" || true)

if [[ "$CONFIG_N" -eq 0 ]]; then
    echo -e "${RED}❌ Parsed zero deserialized config fields — this scan is stale.${NC}"
    FAILED=true
elif [[ "$ORPHAN_CONFIG_N" -eq 0 ]]; then
    echo -e "${GREEN}✅ Config fields: all ${CONFIG_N} deserialized fields have a read site.${NC}"
else
    echo -e "${YELLOW}⚠️  Deserialized config fields with zero read sites (${ORPHAN_CONFIG_N} of ${CONFIG_N}):${NC}"
    while IFS=$'\t' read -r fld file; do
        [[ -z "$fld" ]] && continue
        echo "   $fld  ($file)"
    done < "$TMP/orphan_config_fields.txt"
fi

# ---------------------------------------------------------------------------
# Gate: fail when THIS change introduces a new phantom surface
# ---------------------------------------------------------------------------
if [[ -n "$BASE_REF" ]]; then
    echo ""
    echo -e "${BLUE}---- New phantom surfaces in this change (vs ${BASE_REF}) ----${NC}"

    ADDED="$(git diff -U0 "${BASE_REF}...HEAD" -- 'crates/*/src/*.rs' "$DOMAINS" 2>/dev/null \
        | grep '^+' | grep -v '^+++' || true)"

    # Per-client removals, attributed by tree. A method loses parity two ways:
    # it arrives with a caller on one client only, or its caller on one client
    # is taken away while the other keeps using it. The second is invisible to
    # every other gate — nothing breaks, the endpoint still exists, and the
    # client that dropped it simply stops offering the capability.
    #
    # `--diff-filter=d` excludes whole-file deletions: a call site that goes
    # away because the component around it was deleted is a visible, reviewed
    # act, and the file was often unrendered scaffolding whose "call" nothing
    # ever executed. What this catches is the quiet one — a live file losing a
    # call while the other client keeps making it.
    REMOVED_WEB="$(git diff -U0 --diff-filter=d "${BASE_REF}...HEAD" -- 'frontend/src' 2>/dev/null \
        | grep '^-' | grep -v '^---' || true)"
    REMOVED_MOBILE="$(git diff -U0 --diff-filter=d "${BASE_REF}...HEAD" -- 'frontend-mobile/src' 'frontend-mobile/app' 2>/dev/null \
        | grep '^-' | grep -v '^---' || true)"

    NEW_FOUND=false
    while read -r t; do
        [[ -z "$t" ]] && continue
        if printf '%s\n' "$ADDED" | grep -qE "trait[[:space:]]+${t}\b"; then
            echo -e "${RED}❌ New trait with no implementor: ${t}${NC}"
            NEW_FOUND=true
        fi
    done < "$TMP/orphan_traits.txt"

    if [[ -f "$TMP/orphan_methods.txt" ]]; then
        while read -r m; do
            [[ -z "$m" ]] && continue
            if printf '%s\n' "$ADDED" | grep -qE "async ${m}\("; then
                echo -e "${RED}❌ New api-client method with no production caller: ${m}${NC}"
                NEW_FOUND=true
            fi
        done < "$TMP/orphan_methods.txt"
    fi

    # Parity gaps this change is responsible for.
    check_parity() {
        local list="$1" have="$2" missing="$3" removed="$4"
        [[ -f "$list" ]] || return 0
        while read -r m; do
            [[ -z "$m" ]] && continue
            # Here-strings rather than a pipe into `grep -q`: grep exits at the
            # first match and the writer takes a SIGPIPE, which prints a
            # "write error: Broken pipe" line per hit over a diff this size.
            if grep -qE "async ${m}\(" <<< "$ADDED"; then
                echo -e "${RED}❌ New api-client method called from ${have} only: ${m}${NC}"
                NEW_FOUND=true
            elif grep -qE "\.${m}([^a-zA-Z0-9_]|$)" <<< "$removed"; then
                echo -e "${RED}❌ ${missing} dropped its last caller of ${m}; ${have} still calls it${NC}"
                NEW_FOUND=true
            fi
        done < "$list"
    }
    check_parity "$TMP/web_only.txt" "web" "mobile" "$REMOVED_MOBILE"
    check_parity "$TMP/mobile_only.txt" "mobile" "web" "$REMOVED_WEB"

    if [[ -f "$TMP/orphan_config_fields.txt" ]]; then
        while IFS=$'\t' read -r fld _file; do
            [[ -z "$fld" ]] && continue
            if printf '%s\n' "$ADDED" | grep -qE "pub ${fld}:"; then
                echo -e "${RED}❌ New deserialized config field with no read site: ${fld}${NC}"
                NEW_FOUND=true
            fi
        done < "$TMP/orphan_config_fields.txt"
    fi

    if [[ "$NEW_FOUND" == "true" ]]; then
        echo ""
        echo -e "${RED}A capability that nothing implements or calls is a phantom surface.${NC}"
        echo -e "${RED}A capability only one client calls is the same thing, on one surface.${NC}"
        echo -e "${YELLOW}Wire a production consumer on every surface that should have it in this${NC}"
        echo -e "${YELLOW}same change, or register the gap with a LIMITATION(registre#issue):${NC}"
        echo -e "${YELLOW}marker naming the item. Do not ship it bare — nothing breaks when a${NC}"
        echo -e "${YELLOW}phantom is wrong, so no test will ever find it.${NC}"
        FAILED=true
    else
        echo -e "${GREEN}✅ This change adds no unimplemented trait, no uncalled api-client method,${NC}"
        echo -e "${GREEN}   and no method that reaches one client but not the other.${NC}"
    fi
fi

if [[ "$FAILED" == "true" ]]; then
    echo -e "${RED}❌ PHANTOM SURFACE CHECK FAILED${NC}"
    exit 1
fi
exit 0
