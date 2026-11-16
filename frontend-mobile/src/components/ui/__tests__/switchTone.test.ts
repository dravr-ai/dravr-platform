// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Measures the phone's one Switch colour set: each state's track against every ground, each thumb against its track
// ABOUTME: Both schemes, through the shared contrast kit, at the 3:1 a control's parts need (WCAG 1.4.11)

import { BOREAL_DARK, BOREAL_LIGHT } from '@pierre/shared-constants';
import { switchColors } from '../switchTone';
import { AA_GRAPHIC, contrast, GROUNDS, SCHEMES } from '../../../test/contrast';

const PALETTE = { light: BOREAL_LIGHT, dark: BOREAL_DARK } as const;

describe('switchColors', () => {
  it.each(SCHEMES)('each track clears 3:1 against every ground, and each thumb against its track, in %s', (scheme) => {
    const failures: string[] = [];
    for (const on of [true, false]) {
      const set = switchColors(PALETTE[scheme], on);
      const track = on ? set.trackColor.true : set.trackColor.false;
      for (const [groundName, ground] of GROUNDS[scheme]) {
        const ratio = contrast(track, ground);
        if (ratio < AA_GRAPHIC) failures.push(`${on ? 'on' : 'off'} track over ${groundName}: ${ratio.toFixed(2)}:1`);
      }
      const thumb = contrast(set.thumbColor, track);
      if (thumb < AA_GRAPHIC) failures.push(`${on ? 'on' : 'off'} thumb on track: ${thumb.toFixed(2)}:1`);
    }
    expect(failures).toEqual([]);
  });

  it('paints the iOS off ground as the off track', () => {
    const set = switchColors(BOREAL_LIGHT, false);
    expect(set.ios_backgroundColor).toBe(set.trackColor.false);
  });

  it('needs the track ink as the thumb: white reads under 3:1 on the dark scheme primary', () => {
    expect(contrast('#ffffff', BOREAL_DARK.primary)).toBeLessThan(AA_GRAPHIC);
  });
});
