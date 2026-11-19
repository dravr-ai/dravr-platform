// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The badge tone a claim verdict draws in — the turn's chip and the drawer's status word read the same answer
// ABOUTME: A toneless verdict is the neutral pair, never the muted `secondary` badge despite sharing the name

import type { VerdictTone } from '@pierre/shared-types';
import type { BadgeTone } from '../ui';

/**
 * The five feedback tones are the badge's own. A verdict's `secondary` tone
 * means "no hue to carry", which the chip has always drawn as the neutral
 * pair (`surface-container-high` under `on-surface`) — not the badge's muted
 * `secondary` tone, a lighter ground under the variant ink.
 */
export function verdictBadgeTone(tone: VerdictTone): BadgeTone {
  return tone === 'secondary' ? 'neutral' : tone;
}
