// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Measures every phone alert tone's ink against its tint, composited over each ground, in both schemes
// ABOUTME: Pins the table to the feedback hues and to the badge's bound inks, which alertInkColor's runtime twin reads

import { ALERT_TONE, type AlertTone } from '../alertTone';
import { BADGE_TONE } from '../Badge';
import { AA_GRAPHIC, AA_TEXT, classColor, contrast, fillOver, GROUNDS, SCHEMES } from '../../../test/contrast';

const TONES = Object.keys(ALERT_TONE) as AlertTone[];

describe('ALERT_TONE', () => {
  it.each(TONES)('labels %s in the ink the badge binds to the same hue', (tone) => {
    // alertInkColor hands a glyph badgeInkColor's value, whose parity with
    // the class is Badge.test's; this keeps the two tables on one ink.
    expect(ALERT_TONE[tone].ink).toBe(BADGE_TONE[tone].ink);
  });

  it.each(SCHEMES)('every ink clears AA — and so the glyph floor — on its tint over every ground, in %s', (scheme) => {
    const failures: string[] = [];
    for (const tone of TONES) {
      const { fill, ink } = ALERT_TONE[tone];
      const inkColor = classColor(ink, scheme);
      for (const [groundName, ground] of GROUNDS[scheme]) {
        const ratio = contrast(inkColor, fillOver(fill, scheme, ground));
        if (ratio < Math.max(AA_TEXT, AA_GRAPHIC)) failures.push(`${tone} over ${groundName}: ${ratio.toFixed(2)}:1`);
      }
    }
    expect(failures).toEqual([]);
  });

  it('needs the bound ink: a bare hue on its own tint falls under the floor', () => {
    // `text-error` on `bg-error/10` and a `colors.warning` glyph on
    // `bg-warning/15`, over light grounds.
    const [, surface] = GROUNDS.light[0];
    const [, highest] = GROUNDS.light[5];
    expect(contrast(classColor('text-error', 'light'), fillOver('bg-error/10', 'light', highest))).toBeLessThan(AA_TEXT);
    expect(contrast(classColor('text-warning', 'light'), fillOver('bg-warning/15', 'light', surface))).toBeLessThan(AA_GRAPHIC);
  });
});
