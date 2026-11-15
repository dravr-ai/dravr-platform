// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the verdict tone to badge tone mapping both the chip and the drawer's status word read
// ABOUTME: A toneless verdict is the neutral pair, never the badge's muted secondary tone that shares its name

import { describe, expect, it } from 'vitest';
import type { VerdictTone } from '@pierre/shared-types';
import { verdictBadgeTone } from '../verdictBadgeTone';

describe('verdictBadgeTone', () => {
  it.each(['success', 'warning', 'error', 'info'] as VerdictTone[])('keeps the %s tone', (tone) => {
    expect(verdictBadgeTone(tone)).toBe(tone);
  });

  it('draws a toneless verdict as the neutral pair', () => {
    expect(verdictBadgeTone('secondary')).toBe('neutral');
  });
});
