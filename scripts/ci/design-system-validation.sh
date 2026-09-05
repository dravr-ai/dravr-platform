#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Ratcheting design-system conformance gate for web + mobile
# ABOUTME: Enforces DESIGN.md token discipline that ESLint and tsc cannot check

# Two classes of rule live here:
#
#   1. HARD rules — a violation fails the build outright. These are for drift
#      that is fully migrated, so the correct count is zero forever.
#
#   2. RATCHETED rules — a pre-existing backlog too large to clear in one
#      change. The build fails when the count RISES, and fails with a
#      "lower the baseline" instruction when it FALLS. The number can only
#      travel one direction. It is a counter, not an allowlist: no file is
#      ever named or exempted, so nothing can hide behind it permanently.
#
# Element-level rules (raw <select>/<textarea>) live in frontend/eslint.config.js
# instead, where the JSX AST makes them precise. This script covers what only a
# text scan can see.

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"

# ============================================================================
# BASELINES — these may only ever go DOWN.
#
# Lowering one is the whole point: clear some violations, run this script, and
# it tells you the new number to write here. Raising one requires deleting this
# comment and explaining why in the commit, because it means the design system
# lost ground.
# ============================================================================
BASELINE_WEB_RAW_INPUT=30
BASELINE_WEB_RAW_PALETTE=2
BASELINE_MOBILE_RAW_PALETTE=26
BASELINE_MOBILE_RAW_TEXTINPUT=14
BASELINE_WEB_LEGACY_PIERRE=0
# Boreal v2 (DESIGN.md §4, §5): backdrop blur belongs to overlays over
# photography and `boreal-hero-gradient` to nothing at all any more. Both are
# ratcheted from the count the refresh left, so they can only fall.
BASELINE_WEB_BACKDROP_BLUR=16
BASELINE_WEB_HERO_GRADIENT=7

# Tailwind's stock palette. Project tokens (primary, surface, on-surface,
# outline, and the pillar tokens) never match: the colour name must follow the
# utility prefix directly, so "text-activity" is a token and "text-red-500" is not.
PALETTE_RE='(text|bg|border|ring|from|to|via|divide|placeholder|decoration|outline|shadow|accent|caret|fill|stroke)-(red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose|slate|gray|zinc|neutral|stone)-(50|[1-9]00|950)'

FAILED=0

echo -e "${BLUE}==== Dravr - Design System Validation ====${NC}"
echo "Project root: $PROJECT_ROOT"
echo ""

# ----------------------------------------------------------------------------
# Helper: ratcheted count check
#   $1 human label   $2 actual count   $3 baseline   $4 remediation hint
# ----------------------------------------------------------------------------
check_ratchet() {
    local label="$1" actual="$2" baseline="$3" hint="$4"

    if [ "$actual" -gt "$baseline" ]; then
        echo -e "${RED}FAIL${NC} $label: $actual (baseline $baseline)"
        echo "     $((actual - baseline)) new violation(s). $hint"
        FAILED=1
    elif [ "$actual" -lt "$baseline" ]; then
        echo -e "${YELLOW}RATCHET${NC} $label: $actual (baseline $baseline)"
        echo "     You removed $((baseline - actual)). Lower the baseline in"
        echo "     scripts/ci/design-system-validation.sh to $actual to lock the win in."
        FAILED=1
    else
        echo -e "${GREEN}OK${NC}   $label: $actual (at baseline)"
    fi
}

# ----------------------------------------------------------------------------
# HARD: every design token is defined in exactly one place per platform.
# DESIGN.md claims to be mirrored across these files; verify the files exist
# rather than trusting the claim.
# ----------------------------------------------------------------------------
echo "-- Token source files --"
for f in \
    "frontend/DESIGN.md" \
    "frontend/src/index.css" \
    "frontend/tailwind.config.cjs" \
    "frontend-mobile/global.css" \
    "frontend-mobile/tailwind.config.js" \
    "packages/shared-constants/src/design-system.ts"
do
    if [ ! -f "$PROJECT_ROOT/$f" ]; then
        echo -e "${RED}FAIL${NC} DESIGN.md names $f as a token mirror, but it does not exist."
        FAILED=1
    fi
done
[ "$FAILED" -eq 0 ] && echo -e "${GREEN}OK${NC}   all six token mirrors present"
echo ""

# ----------------------------------------------------------------------------
# HARD: the mirrors must agree, not merely exist.
#
# "Mirrored in these files" was a claim nothing checked, and the platforms drifted
# behind it: mobile carried the Editorial-tier warning (#8f6a2e) for months while
# DESIGN.md §2 specified the Product-tier value (#b08326). Same class of bug as a
# capability advertised without a backing impl — so it is checked the same way.
# ----------------------------------------------------------------------------
echo "-- Feedback palette agreement (DESIGN.md §2) --"

# DESIGN.md §2 is the source of truth. Values as "r g b" per CSS-var convention.
declare -a FEEDBACK_LIGHT=(
    "success:46 125 91"
    "warning:176 131 38"
    "error:186 26 26"
    "info:62 114 131"
)
declare -a FEEDBACK_DARK=(
    "success:121 166 148"
    "warning:214 184 122"
    "error:255 180 171"
    "info:155 182 189"
)

check_token() {
    local file="$1" token="$2" expect="$3" theme="$4"
    local found
    found=$(grep -oE -- "--color-$token:[[:space:]]*[0-9]+ [0-9]+ [0-9]+" "$PROJECT_ROOT/$file" \
        | sed -E "s/.*--color-$token:[[:space:]]*//" | sed -n "${5}p")
    if [ -z "$found" ]; then
        echo -e "${RED}FAIL${NC} $file is missing --color-$token ($theme)"
        FAILED=1
    elif [ "$found" != "$expect" ]; then
        echo -e "${RED}FAIL${NC} $file --color-$token ($theme) = '$found', DESIGN.md §2 says '$expect'"
        FAILED=1
    fi
}

# Both stylesheets declare light first, then dark — occurrence 1 is light, 2 is dark.
for f in "frontend/src/index.css" "frontend-mobile/global.css"; do
    for pair in "${FEEDBACK_LIGHT[@]}"; do
        check_token "$f" "${pair%%:*}" "${pair#*:}" "light" 1
    done
    for pair in "${FEEDBACK_DARK[@]}"; do
        check_token "$f" "${pair%%:*}" "${pair#*:}" "dark" 2
    done
done

# Tokens are useless to Tailwind unless the config composes alpha onto them;
# a bare var() silently drops every /NN modifier at build time.
for cfg in "frontend/tailwind.config.cjs" "frontend-mobile/tailwind.config.js"; do
    if grep -qE "'var\(--color-" "$PROJECT_ROOT/$cfg"; then
        echo -e "${RED}FAIL${NC} $cfg maps a token as bare 'var(--color-…)'."
        echo "     Tailwind drops opacity modifiers on those, so bg-token/20 emits nothing."
        echo "     Use 'rgb(var(--color-…) / <alpha-value>)'."
        grep -nE "'var\(--color-" "$PROJECT_ROOT/$cfg" | head -5
        FAILED=1
    fi
done

grep -q "SEMANTIC_COLORS_DARK" "$PROJECT_ROOT/packages/shared-constants/src/design-system.ts" || {
    echo -e "${RED}FAIL${NC} shared-constants exports no SEMANTIC_COLORS_DARK; DESIGN.md §2 defines both halves."
    FAILED=1
}

[ "$FAILED" -eq 0 ] && echo -e "${GREEN}OK${NC}   feedback palette agrees across web, mobile and shared-constants"
echo ""

# ----------------------------------------------------------------------------
# HARD: the editorial underline is the only form-field language.
# ESLint enforces the JSX side; this catches a bypass via the CSS classes the
# pre-Boreal system used, which ESLint cannot see.
# ----------------------------------------------------------------------------
echo "-- Retired pre-Boreal field classes --"
LEGACY_FIELD=$(grep -rn 'input-dark\|select-dark' \
    "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null | wc -l | tr -d ' ')
if [ "$LEGACY_FIELD" -gt 0 ]; then
    echo -e "${RED}FAIL${NC} $LEGACY_FIELD use(s) of the retired .input-dark/.select-dark classes:"
    grep -rn 'input-dark\|select-dark' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null | head -10
    echo "     Use <Input>, <Textarea> or <Select> from components/ui instead."
    FAILED=1
else
    echo -e "${GREEN}OK${NC}   no retired field classes in use"
fi
echo ""

# ----------------------------------------------------------------------------
# RATCHETED: raw <input> outside the ui/ primitives.
# Unlike <select>/<textarea> this is not fully migrated — <Input> covers text
# fields, while checkbox/radio/number/file/range have no primitive yet, so a
# hard zero is not reachable until those exist.
# ----------------------------------------------------------------------------
# ----------------------------------------------------------------------------
# HARD: DESIGN.md §5 "Forbidden" — a dark brand surface paired with a hardcoded
# text colour.
#
# Neither half is stock Tailwind, so the palette ratchet cannot see this, and it
# type-checks and lints cleanly. It is only visible when rendered: a filled CTA
# built this way is theme-invariant, so in dark mode it sits next to a real
# .btn-primary showing the inverse contrast. A visual sweep found exactly one
# (ConnectProviderBanner) after it had shipped.
# ----------------------------------------------------------------------------
echo "-- Forbidden CTA pattern (DESIGN.md §5) --"
FORBIDDEN_CTA=$(grep -rn --include='*.tsx' -E \
    '(bg-pierre-violet|bg-pierre-cyan|from-pierre-violet|from-pierre-cyan)[^"'"'"']*\btext-(white|black)\b' \
    "$PROJECT_ROOT/frontend/src" 2>/dev/null | wc -l | tr -d ' ')
if [ "$FORBIDDEN_CTA" -gt 0 ]; then
    echo -e "${RED}FAIL${NC} $FORBIDDEN_CTA element(s) pair a dark pierre surface with hardcoded text:"
    grep -rn --include='*.tsx' -E \
        '(bg-pierre-violet|bg-pierre-cyan|from-pierre-violet|from-pierre-cyan)[^"'"'"']*\btext-(white|black)\b' \
        "$PROJECT_ROOT/frontend/src" 2>/dev/null | cut -c1-140 | head -5
    echo "     Use .btn-primary, or pair the surface explicitly with text-on-primary."
    FAILED=1
else
    echo -e "${GREEN}OK${NC}   no dark-surface + hardcoded-text pairings"
fi
echo ""

echo "-- Raw form controls --"
WEB_RAW_INPUT=$(grep -rn '<input' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | grep -v 'src/components/ui/' | wc -l | tr -d ' ')
check_ratchet "web raw <input> outside components/ui" \
    "$WEB_RAW_INPUT" "$BASELINE_WEB_RAW_INPUT" \
    "Use <Input> from components/ui, or add the missing primitive."

# Mobile had no equivalent rule, which is how a modal kept hand-rolling boxed
# fields and a hardcoded brand-blue CTA long after the primitive existed. The
# bypass is the bug, not the platform.
MOBILE_RAW_TEXTINPUT=$(grep -rn '<TextInput' "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' 2>/dev/null \
    | grep -v 'src/components/ui/' | wc -l | tr -d ' ')
check_ratchet "mobile raw <TextInput> outside components/ui" \
    "$MOBILE_RAW_TEXTINPUT" "$BASELINE_MOBILE_RAW_TEXTINPUT" \
    "Use <Input> from components/ui."
echo ""

# ----------------------------------------------------------------------------
# RATCHETED: Tailwind stock palette instead of Boreal tokens.
# DESIGN.md §2 defines the full palette; text-amber-400 (#fbbf24) next to the
# Boreal warning (#d6b87a) reads as a different product.
# ----------------------------------------------------------------------------
# ----------------------------------------------------------------------------
# RATCHETED: the legacy `pierre.*` colour namespace.
#
# This is a second colour system living beside Boreal. Every value in it is a
# LIGHT-theme hex frozen as a literal — pierre-violet is #00241a, which is what
# `primary` resolves to in light and nothing like what it resolves to in dark.
# So each of these call sites is theme-invariant by construction: the same bug
# as a hardcoded brand CTA, just wearing a project-looking name.
#
# That name is exactly why it hid. The stock-palette check above matches
# Tailwind's own colour names, and `pierre-violet` is not one, so ~841 frozen
# colours were invisible to every other rule in this file. A gate that only
# looks where you expect drift is a gate that certifies the parts you already
# trusted.
#
# tailwind.config.cjs says of this namespace: "Migrate call sites to canonical
# MD3 names and this block can be deleted." That is the exit condition — when
# this count reaches zero, delete the `pierre:` block from the config.
# ----------------------------------------------------------------------------
echo "-- Legacy pierre.* namespace (frozen light-theme colours) --"
WEB_LEGACY=$(grep -rEoh '\b[a-z]+-pierre-[a-z0-9-]+' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "web legacy pierre-* classnames" \
    "$WEB_LEGACY" "$BASELINE_WEB_LEGACY_PIERRE" \
    "Use the canonical token (primary, error, on-surface, the pillar tokens…) — they track the theme."
echo ""

echo "-- Stock Tailwind palette vs Boreal tokens --"
WEB_PALETTE=$(grep -rEoh "$PALETTE_RE" "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "web stock-palette classnames" \
    "$WEB_PALETTE" "$BASELINE_WEB_RAW_PALETTE" \
    "Use the DESIGN.md §2 tokens (primary, surface, on-surface, outline, error, warning, success)."

MOBILE_PALETTE=$(grep -rEoh "$PALETTE_RE" "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "mobile stock-palette classnames" \
    "$MOBILE_PALETTE" "$BASELINE_MOBILE_RAW_PALETTE" \
    "Use the DESIGN.md §2 tokens via NativeWind."
echo ""

# ----------------------------------------------------------------------------
# HARD, Boreal v2 (DESIGN.md §2–§5): four things the refresh removed and that a
# copy-paste from an old screen brings back without any other gate noticing.
#
#   emoji as icons — the audit found fifteen (a wrench, a lock, a robot, a
#   party popper); every icon is inline SVG or a lucide glyph now.
#   text under 12px — `text-[9px|10px|11px]` sat in 34 places below the §3 floor.
#   caps-tracked labels — `uppercase tracking-wide(r)` was the Inter 11px label
#   idiom; the only tracked text in the product is the wordmark.
#   retired faces and scale — Inter, Space Grotesk, `font-label`,
#   `tracking-label`, and the stock `shadow-sm/md/lg/xl` classes (the Tailwind
#   scale now holds only `floating`).
# ----------------------------------------------------------------------------
echo "-- Boreal v2 retirements (DESIGN.md §2–§5) --"
# Code points through perl — U+1F300–U+1FAFF (pictographs) and U+2600–U+27BF
# (dingbats) — because BSD grep on a Mac and GNU grep in CI do not agree on
# byte-range classes, and a gate that silently reads 0 is worse than none.
WEB_EMOJI=$(find "$PROJECT_ROOT/frontend/src" -name '*.tsx' -not -path '*/__tests__/*' -print0 2>/dev/null \
    | xargs -0 perl -CSD -ne 'print "$ARGV:$.\n" if /[\x{1F300}-\x{1FAFF}\x{2600}-\x{27BF}]/' \
    | wc -l | tr -d ' ')
check_ratchet "web emoji used as icons" "$WEB_EMOJI" 0 \
    "Draw an inline SVG or use a lucide glyph; emoji are not part of the brand."

WEB_TINY_TEXT=$(grep -rEoh 'text-\[(9|10|11)px\]' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "web text below the 12px floor" "$WEB_TINY_TEXT" 0 \
    "The smallest step is text-xs (DESIGN.md §3)."

WEB_CAPS_LABELS=$(grep -rEoh 'uppercase tracking-wider?|tracking-wider? uppercase' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "web caps-tracked labels" "$WEB_CAPS_LABELS" 0 \
    "Labels are sentence case with no tracking; only the wordmark is tracked (DESIGN.md §3)."

WEB_RETIRED_FACES=$(grep -rEo "'Inter'|Space Grotesk|\bfont-label\b|\btracking-label\b|\bshadow-(sm|md|lg|xl|2xl)\b" \
    "$PROJECT_ROOT/frontend/src" "$PROJECT_ROOT/frontend/tailwind.config.cjs" --include='*.tsx' --include='*.ts' --include='*.css' --include='*.cjs' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "web retired faces and shadow classes" "$WEB_RETIRED_FACES" 0 \
    "Schibsted Grotesk for headings, Plus Jakarta Sans for everything else, and shadow-floating is the only shadow (DESIGN.md §3, §4)."

WEB_BLUR=$(grep -rEoh 'backdrop-blur(-[a-z]+)?' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "web backdrop-blur outside the overlay pattern" "$WEB_BLUR" "$BASELINE_WEB_BACKDROP_BLUR" \
    "Blur is for .card-boreal-overlay over photography; a scrim is bg-scrim/60 with no blur (DESIGN.md §4)."

WEB_HERO_GRADIENT=$(grep -rEoh 'boreal-hero-gradient|bg-boreal-hero' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "web hero-gradient decoration" "$WEB_HERO_GRADIENT" "$BASELINE_WEB_HERO_GRADIENT" \
    "A filled surface is bg-primary; a tint is bg-primary-container; nothing is a gradient (DESIGN.md §2)."
echo ""

# ----------------------------------------------------------------------------
echo "========================================="
if [ "$FAILED" -eq 0 ]; then
    echo -e "${GREEN}Design system validation passed${NC}"
    exit 0
else
    echo -e "${RED}Design system validation failed${NC}"
    echo ""
    echo "The design system is enforced the same way the Rust architecture is."
    echo "See frontend/DESIGN.md for the rules these checks encode."
    exit 1
fi
