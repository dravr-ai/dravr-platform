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
BASELINE_WEB_RAW_INPUT=29
BASELINE_WEB_RAW_PALETTE=2
BASELINE_MOBILE_RAW_PALETTE=0
BASELINE_MOBILE_RAW_TEXTINPUT=12
BASELINE_WEB_LEGACY_PIERRE=0
# Boreal v2 (DESIGN.md §4, §5): backdrop blur belongs to overlays over
# photography and `boreal-hero-gradient` to nothing at all any more. Both are
# ratcheted from the count the refresh left, so they can only fall.
BASELINE_WEB_BACKDROP_BLUR=15
BASELINE_WEB_HERO_GRADIENT=5
# Boreal v2.1 (DESIGN.md §5, §9): a group is a Section, not a Card; the
# legacy card-dark/card-admin wrappers are flat sections by CSS until their
# consumers move; display sizes above 18px belong to auth and hero numbers.
# All three are ratcheted from the count the density pass left.
BASELINE_WEB_CARD_SITES=127
BASELINE_WEB_LEGACY_CARD_CLASSES=16
BASELINE_WEB_LARGE_TEXT=67

# Boreal v2.2 "Mobile Less" Phase 6 (DESIGN.md §10): the density and token
# rules Phases 1-5 applied to the phone, ratcheted from what that migration
# left rather than zeroed — a `useCardStyle`/`<Card` site the design note
# names as deliberate (StoreCoachDetailScreen's action bar, a floating menu)
# is not debt, and BillingScreen/SciotteLoginModal/BrandIcons keep their
# literals by the same out-of-scope call Phase 2 made.
BASELINE_MOBILE_HEX_LITERALS=0
BASELINE_MOBILE_CARD_SITES=9
BASELINE_MOBILE_ROUNDED_FULL=26
BASELINE_MOBILE_INLINE_BORDER_RADIUS=32

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
# JSX only. A bare `<TextInput` also matches the GENERIC TYPE PARAMETER in
# `useRef<TextInput>(null)` and `RefObject<TextInput | null>`, which are not raw
# fields — they are how a screen holds a ref to the primitive, which is the
# supported way to chain focus. Counting them made the ratchet punish adding a
# ref and it had already banked two of them (ChatInputBar, ChatScreen) into the
# baseline. Requiring a non-identifier character before the `<` separates the
# element from the type argument; the trailing class keeps `<TextInputFoo` out.
MOBILE_RAW_TEXTINPUT=$(grep -rnE '(^|[^[:alnum:]_])<TextInput([[:space:]/>]|$)' "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' 2>/dev/null \
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

WEB_TINY_TEXT=$(grep -rEo 'text-\[(9|10|11)px\]' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
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

WEB_BLUR=$(grep -rEo 'backdrop-blur(-[a-z]+)?' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "web backdrop-blur outside the overlay pattern" "$WEB_BLUR" "$BASELINE_WEB_BACKDROP_BLUR" \
    "Blur is for .card-boreal-overlay over photography; a scrim is bg-scrim/60 with no blur (DESIGN.md §4)."

WEB_HERO_GRADIENT=$(grep -rEoh 'boreal-hero-gradient|bg-boreal-hero' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "web hero-gradient decoration" "$WEB_HERO_GRADIENT" "$BASELINE_WEB_HERO_GRADIENT" \
    "A filled surface is bg-primary; a tint is bg-primary-container; nothing is a gradient (DESIGN.md §2)."
echo ""

# ----------------------------------------------------------------------------
# Boreal v2.1 "Less" (DESIGN.md §5, §8, §9): the density pass took the athlete
# surfaces out of their boxes and down one type step. What is left is counted
# so it can only fall; what was retired is a hard zero.
#
#   <Card> sites — a group is a Section (title, line, rows); a Card is for what
#   floats and for a data object inside a message. 128 sites remain, in
#   drawers, modals and the operator config tabs.
#   card-dark / card-admin — legacy wrappers, flat by CSS; each consumer that
#   moves to Section takes one off the count.
#   text-2xl / text-3xl — 22px and 26px belong to auth headlines and hero
#   numbers; a page or section title is text-xl or text-sm.
#   min-h-[44px] — targets follow the pointer through the touch-target rule;
#   a hard-coded 44 re-inflates the desktop scale.
#   .chat-bubble-ai — the agent speaks as prose; the class is gone.
# ----------------------------------------------------------------------------
echo "-- Boreal v2.1 density (DESIGN.md §5, §8, §9) --"
WEB_CARD_SITES=$(grep -rEo '<Card\b' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "web Card sites" "$WEB_CARD_SITES" "$BASELINE_WEB_CARD_SITES" \
    "Group content with ui/Section (title, line, rows); keep Card for what floats (DESIGN.md §5)."

WEB_LEGACY_CARD_CLASSES=$(grep -rEoh 'className="(card-dark|card-admin)' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "web legacy card-dark/card-admin wrappers" "$WEB_LEGACY_CARD_CLASSES" "$BASELINE_WEB_LEGACY_CARD_CLASSES" \
    "Move the group to ui/Section; the class is a flat shim, not a design (DESIGN.md §5)."

WEB_LARGE_TEXT=$(grep -rEo '\btext-(2xl|3xl)\b' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "web text above 18px" "$WEB_LARGE_TEXT" "$BASELINE_WEB_LARGE_TEXT" \
    "Titles are text-xl (18px), section titles text-sm 600; text-2xl/3xl are for auth headlines and hero numbers (DESIGN.md §3)."

WEB_HARD_44=$(grep -rEo 'min-[hw]-\[44px\]' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "web hard-coded 44px targets" "$WEB_HARD_44" 0 \
    "Use the touch-target class: 44px on a coarse pointer and under lg, the 32px scale on a fine pointer (DESIGN.md §8)."

WEB_AI_BUBBLE=$(grep -rEo 'chat-bubble-ai' "$PROJECT_ROOT/frontend/src" 2>/dev/null | wc -l | tr -d ' ')
check_ratchet "web agent bubble class" "$WEB_AI_BUBBLE" 0 \
    "The agent's turn is prose on the canvas; only .chat-bubble-user remains (DESIGN.md §5)."
echo ""

# ----------------------------------------------------------------------------
# Boreal v2.2 "Mobile Less" Phase 6 (DESIGN.md §10): HARD, phone-specific
# retirements. Each of these is fully migrated per Phases 1-2 — a copy-paste
# from a pre-Boreal screen is the only way one comes back, same reasoning as
# the web v2 retirements above.
# ----------------------------------------------------------------------------
echo "-- Boreal v2.2 mobile retirements (DESIGN.md §10) --"
MOBILE_EMOJI=$(find "$PROJECT_ROOT/frontend-mobile/src" -name '*.tsx' -not -path '*/__tests__/*' -print0 2>/dev/null \
    | xargs -0 perl -CSD -ne 'print "$ARGV:$.\n" if /[\x{1F300}-\x{1FAFF}\x{2600}-\x{27BF}]/' \
    | wc -l | tr -d ' ')
check_ratchet "mobile emoji used as icons" "$MOBILE_EMOJI" 0 \
    "Draw the Feather/lucide glyph the screen already uses elsewhere; emoji are not part of the brand."

MOBILE_TINY_TEXT=$(grep -rEo 'text-\[1' "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "mobile arbitrary text size (text-[1…)" "$MOBILE_TINY_TEXT" 0 \
    "The ladder covers 12-26px (xs…3xl, tailwind.config.js); use a step instead of an arbitrary value."

MOBILE_CAPS_LABELS=$(grep -rEoh 'uppercase tracking-wider?|tracking-wider? uppercase' "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "mobile caps-tracked labels" "$MOBILE_CAPS_LABELS" 0 \
    "Labels are sentence case; only the wordmark carries tracking-brand (DESIGN.md §10)."

MOBILE_BLUR=$(grep -rEoh 'BlurView|expo-blur|expo-glass-effect' "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' --include='*.ts' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "mobile BlurView/expo-blur/expo-glass-effect" "$MOBILE_BLUR" 0 \
    "Phase 1 deleted the floating glass chrome; the system bar and header need no blur."

# Two files keep it by Phase 2's own acceptance criteria: SciotteLoginModal's
# brand sweep and ScrollFadeContainer's edge fade, neither a card or button.
# -E on the exclusion, not just the rest of the file's convention: BSD grep's
# basic-regex mode does not treat \| as alternation, so without it this
# silently matched nothing and let both permitted files count as violations.
MOBILE_GRADIENT=$(grep -rln 'LinearGradient' "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' 2>/dev/null \
    | grep -vE 'SciotteLoginModal\.tsx$|ScrollFadeContainer\.tsx$' | wc -l | tr -d ' ')
check_ratchet "mobile LinearGradient outside the two permitted files" "$MOBILE_GRADIENT" 0 \
    "A filled surface is bg-primary, a tint is bg-primary-container; nothing else is a gradient (DESIGN.md §10)."

MOBILE_SPACE_GROTESK=$(grep -rEoh 'Space Grotesk' "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' --include='*.ts' "$PROJECT_ROOT/frontend-mobile/tailwind.config.js" 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "mobile Space Grotesk" "$MOBILE_SPACE_GROTESK" 0 \
    "Schibsted Grotesk is the only display face loaded (app/_layout.tsx, DESIGN.md §10)."

MOBILE_BG_BLACK=$(grep -rEoh 'bg-black/[0-9]+' "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "mobile bg-black/* overlays" "$MOBILE_BG_BLACK" 0 \
    "Use bg-scrim/60 — one veil token for every sheet and dialog (DESIGN.md §10)."

MOBILE_HARD_44=$(grep -rEoh 'min-[hw]-\[44px\]' "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "mobile hard-coded 44px targets" "$MOBILE_HARD_44" 0 \
    "The phone has one pointer, not two: express 44 through the spacing scale (min-h-11), not an arbitrary bracket value."
echo ""

# ----------------------------------------------------------------------------
# Boreal v2.2 "Mobile Less" Phase 6 (DESIGN.md §10): RATCHETED, phone-specific
# backlog too large to clear in this phase — counted so it can only fall.
# ----------------------------------------------------------------------------
echo "-- Boreal v2.2 mobile density backlog (DESIGN.md §10) --"

# Anchored on a preceding quote or bracket character so a real string literal
# ('#00241a', bg-[#fff]) counts and a bare comment reference does not:
# carnet#215 is three hex digits to a naive regex, and DESIGN.md's own status
# section calls this out by name ("carnet#207 is a 'hex' to a naive regex").
# A backtick-quoted mention inside a /** doc comment */ (`#8f6a2e`) is the same
# trap in a different font and is excluded the same way.
# No -h: the two filters below match on the file path, so the path has to
# survive into the piped text (dropping it would silently match nothing, the
# same trap the gradient check above hit).
MOBILE_HEX_LITERALS=$(grep -rEo "['\"\[]#[0-9a-fA-F]{3,8}\b|['\"]rgba?\([0-9]" "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' \
    | grep -vE 'BrandIcons\.tsx|SciotteLoginModal\.tsx|BillingScreen\.tsx' \
    | wc -l | tr -d ' ')
check_ratchet "mobile hex/rgba literals outside BrandIcons/SciotteLoginModal/BillingScreen" \
    "$MOBILE_HEX_LITERALS" "$BASELINE_MOBILE_HEX_LITERALS" \
    "Read the value from useThemeColors()/tokens instead of a frozen literal (DESIGN.md §2, §10)."

# useCardStyle's own definition (components/ui/Card.tsx) is excluded — the
# primitive necessarily calls itself to exist, which is not a consumer still
# reaching for card visuals. Everywhere else it is a Card-shaped surface a
# Section hasn't replaced yet: some deliberately (StoreCoachDetailScreen's
# action bar, ConversationsScreen's floating overflow menu — Card is for what
# floats, DESIGN.md §5), some not yet reached.
MOBILE_CARD_STYLE_ALL=$(grep -rEo '\buseCardStyle\b' "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
MOBILE_CARD_STYLE_OWN=$(grep -c '\buseCardStyle\b' "$PROJECT_ROOT/frontend-mobile/src/components/ui/Card.tsx" 2>/dev/null || echo 0)
MOBILE_CARD_JSX=$(grep -rEo '<Card\b' "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
MOBILE_CARD_SITES=$(( MOBILE_CARD_STYLE_ALL - MOBILE_CARD_STYLE_OWN + MOBILE_CARD_JSX ))
check_ratchet "mobile useCardStyle/<Card sites" "$MOBILE_CARD_SITES" "$BASELINE_MOBILE_CARD_SITES" \
    "Group content with ui/Section; keep Card for what floats (DESIGN.md §5, §10)."

# Generously ratcheted rather than triaged per-site: the spot check found
# typing dots, circular send/icon buttons and pill progress-bar tracks, every
# one a legitimate use of the radius ladder's own `full` step (avatars and
# badges, tailwind.config.js) rather than a card or button that skipped the
# scale. No file is excluded — a real regression still moves the count.
# No -h: the __tests__ filter matches on the file path (4 sites live in test
# fixtures and are excluded the same way every other check here excludes them).
MOBILE_ROUNDED_FULL=$(grep -rEo '\brounded-full\b' "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "mobile rounded-full sites" "$MOBILE_ROUNDED_FULL" "$BASELINE_MOBILE_ROUNDED_FULL" \
    "Confirm it is an avatar/dot/badge; a card or button belongs on the radius ladder instead (DESIGN.md §10)."

# 999/9999 is the circular-dot idiom (NotificationRow's unread dot) and is as
# legitimate as rounded-full above; every other inline value, including 0
# (Input's deliberately flat editorial field), is a pixel radius that bypassed
# the ladder (4/8/12/20) by being written into a style object instead of a
# className. 0 is not carved out a second time beyond what the row already
# exempts: it is still an inline literal, just one that happens not to need
# the scale to be unambiguous.
MOBILE_BORDER_RADIUS=$(grep -rEo 'borderRadius:[[:space:]]*[0-9]+' "$PROJECT_ROOT/frontend-mobile/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | grep -vE ':[[:space:]]*(999|9999)$' | wc -l | tr -d ' ')
check_ratchet "mobile inline borderRadius: values" "$MOBILE_BORDER_RADIUS" "$BASELINE_MOBILE_INLINE_BORDER_RADIUS" \
    "Use the className radius scale (rounded, rounded-lg, rounded-xl, rounded-3xl) instead of a style-object literal (DESIGN.md §10)."
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
