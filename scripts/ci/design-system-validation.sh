#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Ratcheting design-system conformance gate for web
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
BASELINE_WEB_RAW_INPUT=28
BASELINE_WEB_RAW_PALETTE=2
# White literals 9 -> 2: the seven hand-written switches' `bg-white` thumbs
# went with them onto ui/Switch, whose thumb is a token (carnet#1584).
BASELINE_WEB_WHITE_LITERAL=2
BASELINE_WEB_LEGACY_PIERRE=0
# Boreal v2 (DESIGN.md "Do's and Don'ts"): backdrop blur belongs to overlays over
# photography and `boreal-hero-gradient` to nothing at all any more. Both are
# ratcheted from the count the refresh left, so they can only fall.
BASELINE_WEB_BACKDROP_BLUR=14
BASELINE_WEB_HERO_GRADIENT=4
# Boreal v2.1 (DESIGN.md "Cards / Containers"): a group is a Section, not a Card; the
# legacy card-dark/card-admin wrappers are flat sections by CSS until their
# consumers move; display sizes above 18px belong to auth and hero numbers.
# All three are ratcheted from the count the density pass left.
# Cards 85 -> 82: three error banners that borrowed a Card for its shape
# (ToolAvailability's globally-disabled notice, CoachRejectionModal's warning,
# BillingPage's payment problem) draw through the alert tone table instead
# (carnet#1584).
BASELINE_WEB_CARD_SITES=82
BASELINE_WEB_LEGACY_CARD_CLASSES=12
BASELINE_WEB_LARGE_TEXT=46

# Feedback tints written by hand instead of read from a tone table
# (DESIGN.md "Alerts", carnet#1584). A banner, notice, inline error or
# tinted chip takes ALERT_TONE or BADGE_TONE, whose inks are measured on their
# tints; what is left draws a neutral or bound ink on a wash, never the bare
# hue on its own tint. Web 0: the four washes left (the version history's
# before/after cells, the private lane, the current plan card) are WASH_TONE,
# a third table alertTone.test.tsx reads and measures (5 -> 0).
BASELINE_WEB_HAND_TINT=0

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
# HARD: every design token is defined in exactly one place.
# DESIGN.md names design-system.ts as the source, the generated block the web
# stylesheet imports, and the Tailwind config that maps it; verify the files
# exist rather than trusting the claim. That the block matches the source is
# scripts/ci/check-hosted-css.sh's job: it regenerates and diffs it.
# ----------------------------------------------------------------------------
echo "-- Token source files --"
for f in \
    "frontend/DESIGN.md" \
    "packages/shared-constants/src/design-system.ts" \
    "frontend/src/boreal-tokens.generated.css" \
    "frontend/tailwind.config.cjs" \
    "frontend/src/index.css"
do
    if [ ! -f "$PROJECT_ROOT/$f" ]; then
        echo -e "${RED}FAIL${NC} DESIGN.md names $f as part of the token system, but it does not exist."
        FAILED=1
    fi
done
[ "$FAILED" -eq 0 ] && echo -e "${GREEN}OK${NC}   the token source, the generated block, its stylesheet and the Tailwind config are present"
echo ""

# ----------------------------------------------------------------------------
# HARD: every token reaches Tailwind with its alpha slot.
# ----------------------------------------------------------------------------
echo "-- Tailwind token mapping (DESIGN.md \"The Alpha Rule\") --"

# Tokens are useless to Tailwind unless the config composes alpha onto them;
# a bare var() silently drops every /NN modifier at build time.
cfg="frontend/tailwind.config.cjs"
if grep -qE "'var\(--color-" "$PROJECT_ROOT/$cfg"; then
    echo -e "${RED}FAIL${NC} $cfg maps a token as bare 'var(--color-…)'."
    echo "     Tailwind drops opacity modifiers on those, so bg-token/20 emits nothing."
    echo "     Use 'rgb(var(--color-…) / <alpha-value>)'."
    grep -nE "'var\(--color-" "$PROJECT_ROOT/$cfg" | head -5
    FAILED=1
fi

[ "$FAILED" -eq 0 ] && echo -e "${GREEN}OK${NC}   the Tailwind config composes an alpha onto every token"
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
# HARD: DESIGN.md "The No Dark-on-Dark Rule" — a dark brand surface paired with a hardcoded
# text colour.
#
# Neither half is stock Tailwind, so the palette ratchet cannot see this, and it
# type-checks and lints cleanly. It is only visible when rendered: a filled CTA
# built this way is theme-invariant, so in dark mode it sits next to a real
# .btn-primary showing the inverse contrast. A visual sweep found exactly one
# (ConnectProviderBanner) after it had shipped.
# ----------------------------------------------------------------------------
echo "-- Forbidden CTA pattern (DESIGN.md \"The No Dark-on-Dark Rule\") --"
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
echo ""

# ----------------------------------------------------------------------------
# RATCHETED: Tailwind stock palette instead of Boreal tokens.
# DESIGN.md "Colors" defines the full palette; text-amber-400 (#fbbf24) next to the
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

# `white` has no shade number, so PALETTE_RE above cannot match it: a `bg-white`
# surface sits under the dark theme's light ink, and a `divide-white/10` hairline
# disappears on the light one. The baseline counts what is white in both themes on
# purpose: a switch knob, a QR plate.
echo "-- White literals that do not follow the theme --"
WEB_WHITE=$(grep -rEoh --exclude-dir=__tests__ '\b(bg|divide|border)-white\b' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "web white-literal classnames" \
    "$WEB_WHITE" "$BASELINE_WEB_WHITE_LITERAL" \
    "A surface or hairline is a token (surface-container-lowest, outline-variant); only something white in BOTH themes stays white."
echo ""

echo "-- Stock Tailwind palette vs Boreal tokens --"
WEB_PALETTE=$(grep -rEoh "$PALETTE_RE" "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "web stock-palette classnames" \
    "$WEB_PALETTE" "$BASELINE_WEB_RAW_PALETTE" \
    "Use the DESIGN.md \"Colors\" tokens (primary, surface, on-surface, outline, error, warning, success)."
echo ""

# ----------------------------------------------------------------------------
# HARD, Boreal v2 (DESIGN.md "Colors", "Typography", "Elevation & Depth"): four things the refresh removed and that a
# copy-paste from an old screen brings back without any other gate noticing.
#
#   emoji as icons — the audit found fifteen (a wrench, a lock, a robot, a
#   party popper); every icon is inline SVG or a lucide glyph now.
#   text under 12px — `text-[9px|10px|11px]` sat in 34 places below the text-xs floor.
#   caps-tracked labels — `uppercase tracking-wide(r)` was the Inter 11px label
#   idiom; the only tracked text in the product is the wordmark.
#   retired faces and scale — Inter, Space Grotesk, `font-label`,
#   `tracking-label`, and the stock `shadow-sm/md/lg/xl` classes (the Tailwind
#   scale now holds only `floating`).
# ----------------------------------------------------------------------------
echo "-- Boreal v2 retirements --"
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
    "The smallest step is text-xs (DESIGN.md \"Hierarchy (web)\")."

WEB_CAPS_LABELS=$(grep -rEoh 'uppercase tracking-wider?|tracking-wider? uppercase' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "web caps-tracked labels" "$WEB_CAPS_LABELS" 0 \
    "Labels are sentence case with no tracking; only the wordmark is tracked (DESIGN.md \"The Sentence-Case Rule\")."

WEB_RETIRED_FACES=$(grep -rEo "'Inter'|Space Grotesk|\bfont-label\b|\btracking-label\b|\bshadow-(sm|md|lg|xl|2xl)\b" \
    "$PROJECT_ROOT/frontend/src" "$PROJECT_ROOT/frontend/tailwind.config.cjs" --include='*.tsx' --include='*.ts' --include='*.css' --include='*.cjs' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "web retired faces and shadow classes" "$WEB_RETIRED_FACES" 0 \
    "Schibsted Grotesk for headings, Plus Jakarta Sans for everything else, and shadow-floating is the only shadow (DESIGN.md \"Typography\", \"Shadow Vocabulary\")."

WEB_BLUR=$(grep -rEo 'backdrop-blur(-[a-z]+)?' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "web backdrop-blur outside the overlay pattern" "$WEB_BLUR" "$BASELINE_WEB_BACKDROP_BLUR" \
    "Blur is for .card-boreal-overlay over photography; a scrim is bg-scrim/60 with no blur (DESIGN.md \"Elevation & Depth\")."

WEB_HERO_GRADIENT=$(grep -rEoh 'boreal-hero-gradient|bg-boreal-hero' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "web hero-gradient decoration" "$WEB_HERO_GRADIENT" "$BASELINE_WEB_HERO_GRADIENT" \
    "A filled surface is bg-primary; a tint is bg-primary-container; nothing is a gradient (DESIGN.md \"Do's and Don'ts\")."
echo ""

# ----------------------------------------------------------------------------
# Boreal v2.1 "Less" (DESIGN.md "Cards / Containers", "Layout"): the density pass took the athlete
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
echo "-- Boreal v2.1 density --"
WEB_CARD_SITES=$(grep -rEo '<Card\b' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "web Card sites" "$WEB_CARD_SITES" "$BASELINE_WEB_CARD_SITES" \
    "Group content with ui/Section (title, line, rows); keep Card for what floats (DESIGN.md \"Cards / Containers\")."

WEB_LEGACY_CARD_CLASSES=$(grep -rEoh 'className="(card-dark|card-admin)' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | wc -l | tr -d ' ')
check_ratchet "web legacy card-dark/card-admin wrappers" "$WEB_LEGACY_CARD_CLASSES" "$BASELINE_WEB_LEGACY_CARD_CLASSES" \
    "Move the group to ui/Section; the class is a flat shim, not a design (DESIGN.md \"Cards / Containers\")."

WEB_LARGE_TEXT=$(grep -rEo '\btext-(2xl|3xl)\b' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "web text above 18px" "$WEB_LARGE_TEXT" "$BASELINE_WEB_LARGE_TEXT" \
    "Titles are text-xl (18px), section titles text-sm 600; text-2xl/3xl are for auth headlines and hero numbers (DESIGN.md \"Hierarchy (web)\")."

WEB_HARD_44=$(grep -rEo 'min-[hw]-\[44px\]' "$PROJECT_ROOT/frontend/src" --include='*.tsx' 2>/dev/null \
    | grep -v '__tests__' | wc -l | tr -d ' ')
check_ratchet "web hard-coded 44px targets" "$WEB_HARD_44" 0 \
    "Use the touch-target class: 44px on a coarse pointer and under lg, the 32px scale on a fine pointer (DESIGN.md \"Layout\")."

# Hand-written feedback tints: `bg-<feedback>/NN` that is not a `hover:`
# variant, outside the two tone tables. The tables themselves are excluded by
# path, so the path has to survive into the piped text (no -h).
WEB_HAND_TINT=$(grep -rEo '(^|[^:a-z-])bg-(success|warning|error|info)/[0-9]+' "$PROJECT_ROOT/frontend/src" --include='*.tsx' --include='*.ts' 2>/dev/null \
    | grep -v '__tests__' | grep -vE 'components/ui/(badgeTone|alertTone|washTone)\.ts:' | wc -l | tr -d ' ')
check_ratchet "web hand-written feedback tints" "$WEB_HAND_TINT" "$BASELINE_WEB_HAND_TINT" \
    "Draw a banner through ALERT_TONE (ui/Alert) and a tinted chip or button through BADGE_TONE; their inks are measured on the tint (DESIGN.md \"Alerts\")."

WEB_AI_BUBBLE=$(grep -rEo 'chat-bubble-ai' "$PROJECT_ROOT/frontend/src" 2>/dev/null | wc -l | tr -d ' ')
check_ratchet "web agent bubble class" "$WEB_AI_BUBBLE" 0 \
    "The agent's turn is prose on the canvas; only .chat-bubble-user remains (DESIGN.md \"Chat surfaces\")."
echo ""

# ----------------------------------------------------------------------------
# HARD: every citation of the design spec points at something that exists.
# frontend/DESIGN.md is the one maintained spec. Code, tests and CI messages
# cite it by heading, never by section number, so a renamed heading or a
# revived numbering scheme is caught here instead of leaving a dangling pointer.
# This is a link check into one document, not a second copy of its rules: no
# file is exempted but the spec itself, and the correct count is zero forever.
# It fails closed: a git that cannot scan, or a scan that finds no citation at
# all, is a failure, never a pass.
# ----------------------------------------------------------------------------
echo "-- Design spec citations --"

SPEC="$PROJECT_ROOT/frontend/DESIGN.md"
SPEC_PATHSPEC=(':!frontend/DESIGN.md')

# Run `git grep` over the tree. Exit 1 is "no match" and is fine; anything else
# means the scan did not run, which must not read as a clean result.
GG_OUT=""
spec_grep() {
    local rc=0
    GG_OUT=$(git -C "$PROJECT_ROOT" grep "$@" -- . "${SPEC_PATHSPEC[@]}") || rc=$?
    if [ "$rc" -gt 1 ]; then
        echo -e "${RED}FAIL${NC} git grep could not scan the tree (exit $rc), so no citation was verified"
        FAILED=1
        SPEC_SCAN_BROKEN=1
    fi
}

SPEC_SCAN_BROKEN=0
NUMBERED=""; WRAPPED=""; DANGLING=0; CITED=""

if ! git -C "$PROJECT_ROOT" rev-parse --git-dir >/dev/null 2>&1; then
    echo -e "${RED}FAIL${NC} not a usable git checkout, so no design spec citation was verified"
    FAILED=1
    SPEC_SCAN_BROKEN=1
else
    # Numbered citations: the long-form spec that carried them is history.
    spec_grep -nIE 'DESIGN[.]md[^A-Za-z]{0,6}§'
    NUMBERED="$GG_OUT"
    if [ -n "$NUMBERED" ]; then
        echo -e "${RED}FAIL${NC} a numbered citation of the design spec (cite the heading instead):"
        echo "$NUMBERED" | head -10
        FAILED=1
    fi

    # A citation wrapped across two lines, or inside its quotes, cannot be checked.
    spec_grep -nIE '[(]DESIGN[.]md *$|DESIGN[.]md \\?"[^"]*$'
    WRAPPED="$GG_OUT"
    if [ -n "$WRAPPED" ]; then
        echo -e "${RED}FAIL${NC} a design spec citation wrapped across lines (keep it on one line):"
        echo "$WRAPPED" | head -10
        FAILED=1
    fi

    # Quoted citations must equal a heading or a bold Named Rule. A chain of
    # quoted names after one mention of the spec is checked name by name. "Chat surfaces" is the
    # one documented short form: it names the heading that starts with it.
    spec_grep -hIE 'DESIGN[.]md \\?"'
    CITED=$(printf '%s\n' "$GG_OUT" | perl -ne 'while (/DESIGN\.md \\?"[^"\\]+\\?"(?:(?:,| and|, and| or) \\?"[^"\\]+\\?")*/g) { my $m = $&; while ($m =~ /\\?"([^"\\]+)\\?"/g) { print "$1\n" } }' | sort -u)
    SPEC_NAMES=$( { grep -E '^#{2,4} ' "$SPEC" | sed -E 's/^#+ //'; grep -oE '[*][*][^*]+[.][*][*]' "$SPEC" | sed -E 's/^[*][*]//; s/[.][*][*]$//'; } )
    while IFS= read -r cited; do
        [ -z "$cited" ] && continue
        if [ "$cited" = "Chat surfaces" ]; then
            printf '%s\n' "$SPEC_NAMES" | grep -qE '^Chat surfaces( |$)' && continue
        elif printf '%s\n' "$SPEC_NAMES" | grep -qxF -- "$cited"; then
            continue
        fi
        echo -e "${RED}FAIL${NC} a citation names a heading frontend/DESIGN.md does not have: \"$cited\""
        DANGLING=1
    done <<< "$CITED"
    [ "$DANGLING" -eq 1 ] && FAILED=1

    if [ -z "$CITED" ] && [ "$SPEC_SCAN_BROKEN" -eq 0 ]; then
        echo -e "${RED}FAIL${NC} the scan found no design spec citation at all; there are about two hundred, so it did not read the tree"
        FAILED=1
        SPEC_SCAN_BROKEN=1
    fi
fi

[ -z "$NUMBERED$WRAPPED" ] && [ "$DANGLING" -eq 0 ] && [ "$SPEC_SCAN_BROKEN" -eq 0 ] \
    && echo -e "${GREEN}OK${NC}   every design spec citation names a heading that exists ($(printf '%s\n' "$CITED" | grep -c .) distinct)"
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
