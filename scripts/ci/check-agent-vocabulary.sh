#!/usr/bin/env bash
# ABOUTME: Fails when the AI persona is called a coach again — in the catalogue, or newly in this push
# ABOUTME: The rename was enforced by reading once; this is the fence that keeps it, per ADR-026
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# carnet#366. The coach→agent rename shipped and audited clean, and then a
# route-block change landed on main hours later carrying ten sense-A comments —
# not carelessness, but the natural pull of a codebase whose identifiers all
# still say coach. Decision D6 kept those identifiers on purpose (renaming 27
# MCP tool names, 15 tables and 37 routes would break every SDK consumer for a
# vocabulary change), so the code reads coach while the product speaks agent,
# and every new line is written next to something pulling the wrong way.
#
# Two checks, deliberately different in strictness:
#
#   A. THE CATALOGUE, whole-tree and exact. No value outside `humanCoach.*` may
#      carry the persona noun in any of the five locales. The legitimate
#      survivors are excluded by principle rather than by an allowlist — see
#      the exclusions below — so a genuinely new "AI coach" has nowhere to hide.
#
#   B. NEW PROSE, diff-scoped and conservative. Fails only on high-confidence
#      sense-A phrases this push ADDS. Classifying prose is a judgement call —
#      the same judgement that made a blind sed wrong in the first place — so
#      this half stays narrow and reports the standing stock without blocking.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

FAILED=0

# ---------------------------------------------------------------------------
# A. Catalogue values
# ---------------------------------------------------------------------------
echo "==== Agent vocabulary: the string catalogue ===="

python3 - <<'PY' || FAILED=1
import json, re, sys, pathlib

# The persona noun per locale. Sense B (a human professional) lives under
# humanCoach.* and is not scanned; sense C (coaching, the activity) is a
# different word and is not matched here.
NOUN = {
    'en': r'\bcoach(?:es)?\b',
    'fr': r'\bcoachs?\b|\bentra[iî]neur(?:s|e|es)?\b',
    'es': r'\bcoach(?:es)?\b|\bentrenador(?:a|es|as)?\b',
    'pt': r'\bcoach(?:es)?\b|\btreinador(?:a|es|as)?\b',
    'de': r'\bCoach(?:es|s)?\b|\bTrainer(?:in|innen)?\b',
}

# Exclusions by PRINCIPLE, never a list of blessed strings. Each names a shape
# in which the word is not the AI persona, so a new sense-A value cannot slip
# through by resembling one.
EXCLUSIONS = [
    # A placeholder's own name: `Delete agent "{{coach}}"?` interpolates the
    # persona's title; the token is an identifier, not prose.
    (r'\{\{[^}]*\}\}', 'placeholder token'),
    # The verb, which stayed by decision D2: "Dravr coaches you on…",
    # "so I can coach you on your real data".
    (r'\bcoach(?:es)?\s+you\b|\bcoaches\s+(?:you|athletes)\b', 'the verb "coach you"'),
    # An explicit human coach, which keeps the word by decision D1. The
    # adjective leads in en/de and trails in fr/es/pt, so both orders count:
    # "human coach", "coach humain", "coach humano", "menschlicher Coach".
    (r'human[- ]coach'
     r'|(?:coach|entrenador|treinador|entra[iî]neur)\w*[- ]hum\w+'
     r'|hum\w+[- ](?:coach|entrenador|treinador|entra[iî]neur)'
     r'|menschlich\w*\s+Coach',
     'an explicit human coach'),
    # A slash command the athlete types: /group coach keeps its name (D4).
    (r'/(?:group|agent|coach)\s+coach\b|/coach\b', 'a slash-command token'),
]

root = pathlib.Path('packages/i18n/src/locales')
bad = []
scanned = 0

def flatten(node, prefix=''):
    if isinstance(node, dict):
        for k, v in node.items():
            yield from flatten(v, f'{prefix}.{k}' if prefix else k)
    else:
        yield prefix, node

for locale, pattern in NOUN.items():
    path = root / locale / 'translation.json'
    if not path.exists():
        print(f'   missing catalogue: {path}', file=sys.stderr)
        sys.exit(1)
    noun = re.compile(pattern, re.IGNORECASE if locale != 'de' else 0)
    for key, value in flatten(json.loads(path.read_text())):
        if not isinstance(value, str) or key.startswith('humanCoach'):
            continue
        scanned += 1
        stripped = value
        for shape, _why in EXCLUSIONS:
            stripped = re.sub(shape, ' ', stripped, flags=re.IGNORECASE)
        if noun.search(stripped):
            bad.append((locale, key, value.strip()[:90]))

if bad:
    print('❌ Catalogue values that call the AI persona a coach:\n')
    for locale, key, value in bad:
        print(f'   [{locale}] {key}')
        print(f'       {value}')
    print(
        '\n   The AI persona is an agent (ADR-026). A human professional coach\n'
        '   keeps the word and belongs under `humanCoach.*`; the activity\n'
        '   ("coaching style", "coaching group") and the verb ("coaches you\n'
        '   on") are unchanged and are already excluded by this check.'
    )
    sys.exit(1)

print(f'✅ Catalogue: {scanned} values across 5 locales, none names the persona a coach.')
PY

# ---------------------------------------------------------------------------
# B. Prose this push adds
# ---------------------------------------------------------------------------
echo ""
echo "==== Agent vocabulary: prose added by this push ===="

BASE="${VOCAB_DIFF_BASE:-origin/main}"
if ! git rev-parse --verify --quiet "$BASE" >/dev/null; then
    echo "ℹ️  $BASE unavailable — skipping the diff-scoped half."
    exit $FAILED
fi

MERGE_BASE="$(git merge-base HEAD "$BASE" 2>/dev/null || echo '')"
if [[ -z "$MERGE_BASE" ]]; then
    echo "ℹ️  no merge base with $BASE — skipping the diff-scoped half."
    exit $FAILED
fi

# High-confidence sense-A phrases only. Each names the persona directly; none
# of them is a plausible way to describe a human coach, the activity, or an
# identifier. Deliberately narrow: a miss costs a follow-up, a false positive
# costs trust in the gate.
PHRASES='AI coach|AI-coach|[Cc]oach [Ss]tore|the coach.s reply|coach persona|system coach|[Ii]nstall Coach|your coach\b'

ADDED="$(git diff "$MERGE_BASE"...HEAD --unified=0 -- \
    ':!packages/i18n/src/locales' ':!scripts/ci/check-agent-vocabulary.sh' \
    2>/dev/null | rg '^\+' | rg -v '^\+\+\+' || true)"

# A line that also names an identifier keeping the old spelling is describing
# that identifier, which is correct and stays. Route literals are no longer
# among them — carnet#384 renamed the wire paths to the agent spelling — so
# the route entry that used to sit in this list is gone, not retargeted.
OFFENDERS="$(printf '%s\n' "$ADDED" \
    | rg "$PHRASES" 2>/dev/null \
    | rg -v 'coach_id|coach_slug|coach_visuals|coaches\.rs|list_coaches|coach-card|CoachA-Z|humanCoach' \
    || true)"

if [[ -n "$OFFENDERS" ]]; then
    echo "❌ This push adds prose calling the AI persona a coach:"
    echo ""
    printf '%s\n' "$OFFENDERS" | head -20 | sed 's/^/   /'
    echo ""
    echo "   The persona is an agent (ADR-026). If the line is really about a"
    echo "   human coach, the activity, or an identifier that kept its name,"
    echo "   say so in the line and this check will pass."
    FAILED=1
else
    echo "✅ Prose: this push adds no line that renames the persona back."
fi

exit $FAILED
