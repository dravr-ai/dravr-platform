// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Measures every badge tone's ink against its tint, composited over each ground, in both schemes
// ABOUTME: Joins the TS tone table to its index.css rule and the generated token triples (test/contrast), so a missing rule or a failing pair is caught

import { describe, expect, it } from 'vitest';
import { BADGE_TONE, type BadgeTone } from '../badgeTone';
import {
  AA_TEXT,
  composite,
  contrast,
  GROUNDS,
  SCHEMES,
  token,
  inkOnFill,
  toneRule,
  utilityFill,
  utilityInk,
} from '../../../test/contrast';
import { AVATAR_SLOT_CLASSES } from '../../chat/avatarSlots';

/**
 * The grounds a pressable chip sits on — a reply on the paper canvas, a card,
 * a sheet — where its deeper /25 hover tint must still hold AA. The two high
 * tiers are left out because the /25 tint falls under AA there in light
 * (error reads 4.34:1 over `surface-container-high`); the one reader of
 * `hover`, the verdict chip, sits on a chat reply.
 */
const CHIP_GROUNDS = ['surface', 'surface-container-lowest', 'surface-container-low', 'surface-container'] as const;

const TONES = Object.keys(BADGE_TONE) as BadgeTone[];

describe('badge tones', () => {
  it('has an index.css rule for every tone in the table', () => {
    for (const tone of TONES) {
      expect(() => toneRule(BADGE_TONE[tone].tone)).not.toThrow();
    }
  });

  it.each(SCHEMES)('every ink clears AA on its tint over every ground, in %s', (scheme) => {
    const failures: string[] = [];
    for (const tone of TONES) {
      const { fill, ink } = toneRule(BADGE_TONE[tone].tone);
      for (const ground of GROUNDS) {
        const ratio = contrast(token(scheme, ink), composite(fill, scheme, token(scheme, ground)));
        if (ratio < AA_TEXT) failures.push(`${tone} over ${ground}: ${ratio.toFixed(2)}:1`);
      }
    }
    expect(failures).toEqual([]);
  });

  it.each(SCHEMES)('every ink clears AA on its hover tint over a chip ground, in %s', (scheme) => {
    const failures: string[] = [];
    for (const tone of TONES) {
      const { ink } = toneRule(BADGE_TONE[tone].tone);
      const fill = utilityFill(BADGE_TONE[tone].hover);
      for (const ground of CHIP_GROUNDS) {
        const ratio = contrast(token(scheme, ink), composite(fill, scheme, token(scheme, ground)));
        if (ratio < AA_TEXT) failures.push(`${tone} hover over ${ground}: ${ratio.toFixed(2)}:1`);
      }
    }
    expect(failures).toEqual([]);
  });

  it('labels error and primary in an ink that clears 6:1 where each bare hue falls under AA on its own tint', () => {
    const error = toneRule(BADGE_TONE.error.tone);
    const primary = toneRule(BADGE_TONE.primary.tone);
    const errorTint = composite(error.fill, 'light', token('light', 'surface-container-low'));
    expect(contrast(token('light', 'error'), errorTint)).toBeLessThan(AA_TEXT);
    expect(contrast(token('light', error.ink), errorTint)).toBeGreaterThanOrEqual(6);
    const primaryTint = composite(primary.fill, 'light', token('light', 'surface-container-high'));
    expect(contrast(token('light', 'primary'), primaryTint)).toBeLessThan(AA_TEXT);
    expect(contrast(token('light', primary.ink), primaryTint)).toBeGreaterThanOrEqual(6);
  });
});

/** An avatar slot's fill and ink: a badge tone class, or a `bg-*`/`text-*` pair. */
function slotColours(classes: string) {
  const tokens = classes.split(/\s+/);
  const badge = tokens.find((c) => c.startsWith('badge-'));
  if (badge) return toneRule(badge);
  const bg = tokens.find((c) => c.startsWith('bg-'));
  const text = tokens.find((c) => c.startsWith('text-'));
  if (!bg || !text) throw new Error(`no fill and ink in ${classes}`);
  return { fill: utilityFill(bg), ink: utilityInk(text) };
}

describe('avatar slots', () => {
  it.each(SCHEMES)('every slot labels its initials at AA on its fill over every ground, in %s', (scheme) => {
    const failures: string[] = [];
    AVATAR_SLOT_CLASSES.forEach((classes, slot) => {
      const { fill, ink } = slotColours(classes);
      for (const ground of GROUNDS) {
        const ratio = inkOnFill(scheme, ink, fill, ground);
        if (ratio < AA_TEXT) failures.push(`slot ${slot} (${classes}) over ${ground}: ${ratio.toFixed(2)}:1`);
      }
    });
    expect(failures).toEqual([]);
  });

  it('keeps the tertiary slot in its own hue: its container ink is the light ink of a dark container', () => {
    // `on-tertiary-container` binds the dark `tertiary-container`, not a pale
    // tertiary tint: on `tertiary`/15 it reads 1.30:1 in light, where the
    // near-black `tertiary` itself reads 7.92:1 at worst.
    const tint = utilityFill('bg-tertiary/15');
    expect(inkOnFill('light', 'on-tertiary-container', tint, 'surface-container-highest')).toBeLessThan(AA_TEXT);
    expect(inkOnFill('light', 'tertiary', tint, 'surface-container-highest')).toBeGreaterThanOrEqual(AA_TEXT);
  });
});
