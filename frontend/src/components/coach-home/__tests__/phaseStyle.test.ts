// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Measures every Coach Home phase band's label against its fill, composited over each ground, in both schemes
// ABOUTME: A band reads its colour from a badge tone or a token pair; either way its words must clear AA wherever the pane sits

import { describe, expect, it } from 'vitest';
import type { CoachRosterPhaseKind } from '@pierre/shared-types';
import { phaseClass } from '../phaseStyle';
import { AA_TEXT, GROUNDS, inkOnFill, SCHEMES, toneRule, utilityFill, utilityInk, type Fill } from '../../../test/contrast';

const KINDS: Array<CoachRosterPhaseKind | 'draft'> = [
  'prep', 'base', 'build', 'specialty', 'peak', 'taper', 'race', 'transition', 'recovery', 'draft',
];

/** A band's fill and ink: a `.badge-*` tone class, or a `bg-*`/`text-*` pair, or an outline with no fill. */
function bandColours(classes: string): { fill: Fill; ink: string } {
  const tokens = classes.split(/\s+/);
  const badge = tokens.find((c) => /^badge-/.test(c));
  if (badge) return toneRule(badge);
  const bg = tokens.find((c) => /^bg-/.test(c));
  const text = tokens.find((c) => /^text-[a-z-]+$/.test(c) && !/^text-(xs|sm|base|lg|xl)$/.test(c));
  if (!text) throw new Error(`no ink in ${classes}`);
  // An outlined band has no fill: its label sits on the ground.
  return { fill: bg ? utilityFill(bg) : { token: 'surface', alpha: 0 }, ink: utilityInk(text) };
}

describe('phase bands', () => {
  it.each(SCHEMES)('every label clears AA on its band over every ground, in %s', (scheme) => {
    const failures: string[] = [];
    for (const kind of KINDS) {
      const { fill, ink } = bandColours(phaseClass(kind));
      for (const ground of GROUNDS) {
        const ratio = inkOnFill(scheme, ink, fill, ground);
        if (ratio < AA_TEXT) failures.push(`${kind} over ${ground}: ${ratio.toFixed(2)}:1`);
      }
    }
    expect(failures).toEqual([]);
  });

  it('draws peak and taper on the warning badge tone, since its ink falls under AA at /20', () => {
    expect(phaseClass('peak')).toBe('badge-warning');
    expect(phaseClass('taper')).toBe('badge-warning');
    expect(inkOnFill('light', 'on-warning-container', utilityFill('bg-warning/20'), 'surface-container-highest')).toBeLessThan(AA_TEXT);
  });
});
