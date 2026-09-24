// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Measures every provider glyph ink against the canvas of its scheme, from the token values
// ABOUTME: A brand keeps its hex only where it clears the 3:1 icon floor, and yields to body ink only where it fails

import { describe, expect, it } from 'vitest';
import {
  BOREAL,
  PROVIDER_COLORS,
  PROVIDER_GLYPH_INK,
  providerGlyphInk,
  type ColorScheme,
} from '../src/design-system';

/** WCAG 1.4.11: a graphic that identifies something needs 3:1 against its ground. */
const ICON_FLOOR = 3;
const SCHEMES: readonly ColorScheme[] = ['light', 'dark'];

function channel(value: number): number {
  const c = value / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function luminance(hex: string): number {
  const h = hex.replace('#', '');
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16));
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** The ink a client actually draws: the brand hex, or the scheme's body ink for `null`. */
function drawnInk(providerId: string, scheme: ColorScheme): string {
  return providerGlyphInk(providerId, scheme) ?? BOREAL[scheme].onSurface;
}

describe('PROVIDER_GLYPH_INK', () => {
  const ids = Object.keys(PROVIDER_GLYPH_INK);

  it('covers every branded id the clients draw a glyph for', () => {
    expect([...ids].sort()).toEqual([
      'coros',
      'garmin',
      'intervals_icu',
      'sciotte',
      'sciotte_coros',
      'sciotte_garmin',
      'sciotte_trainingpeaks',
      'strava',
      'trainingpeaks',
      'whoop',
    ]);
  });

  for (const scheme of SCHEMES) {
    it(`draws every glyph at 3:1 or better on the ${scheme} surface`, () => {
      const surface = BOREAL[scheme].surface;
      for (const id of ids) {
        const ratio = contrast(drawnInk(id, scheme), surface);
        expect({ id, pass: ratio >= ICON_FLOOR }).toEqual({ id, pass: true });
      }
    });

    it(`yields to body ink on the ${scheme} surface only where the brand hex fails`, () => {
      const surface = BOREAL[scheme].surface;
      for (const id of ids) {
        const brand = PROVIDER_COLORS[id as keyof typeof PROVIDER_COLORS];
        const brandPasses = contrast(brand, surface) >= ICON_FLOOR;
        expect({ id, ink: PROVIDER_GLYPH_INK[id][scheme] }).toEqual({
          id,
          ink: brandPasses ? brand : null,
        });
      }
    });
  }

  it('gives TrainingPeaks its blue on light and body ink on dark, WHOOP the reverse', () => {
    expect(providerGlyphInk('sciotte_trainingpeaks', 'light')).toBe('#005695');
    expect(providerGlyphInk('sciotte_trainingpeaks', 'dark')).toBeNull();
    expect(contrast('#005695', BOREAL.dark.surface)).toBeLessThan(ICON_FLOOR);

    expect(providerGlyphInk('whoop', 'light')).toBeNull();
    expect(providerGlyphInk('whoop', 'dark')).toBe('#00D46A');
    expect(contrast('#00D46A', BOREAL.light.surface)).toBeLessThan(ICON_FLOOR);
  });

  it('keeps Strava, Garmin, intervals.icu and COROS in their own colour in both schemes', () => {
    for (const scheme of SCHEMES) {
      expect(providerGlyphInk('sciotte_coros', scheme)).toBe('#F8273B');
      expect(providerGlyphInk('sciotte', scheme)).toBe('#FC4C02');
      expect(providerGlyphInk('sciotte_garmin', scheme)).toBe('#007CC3');
      expect(providerGlyphInk('intervals_icu', scheme)).toBe('#1273DE');
    }
  });

  it('draws an id with no brand row in the body ink', () => {
    expect(providerGlyphInk('synthetic', 'light')).toBeNull();
    expect(providerGlyphInk('synthetic', 'dark')).toBeNull();
    // An inherited Object key is not a provider row.
    expect(providerGlyphInk('toString', 'light')).toBeNull();
  });
});
