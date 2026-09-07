// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the category colour pair — the accent is the fill, the ink is what may be drawn on a tint of it
// ABOUTME: Measures every category in both schemes with the WCAG formula, because the accent drawn as its own label fails AA

import React from 'react';
import { renderHook, waitFor } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { BOREAL_DARK, BOREAL_LIGHT } from '@pierre/shared-constants';

// NativeWind's own hook needs a stub under jest; the resolved scheme comes
// from the persisted appearance preference, which each test writes.
jest.mock('nativewind', () => ({
  useColorScheme: () => ({ colorScheme: 'dark', setColorScheme: jest.fn() }),
}));

jest.mock('../src/services/api', () => ({
  userApi: { updateTheme: jest.fn().mockResolvedValue(undefined) },
}));

import { ThemeProvider, type ThemeColors } from '../src/contexts/ThemeContext';
import { categoryAccent, categoryInk, useThemeColors } from '../src/constants/theme';

const APPEARANCE_KEY = 'pierre.appearance_pref';

/** WCAG 1.4.3 AA for body-size text. */
const AA_TEXT = 4.5;

/**
 * The tint the phone actually paints under a category badge.
 *
 * `StoreScreen` builds it as `` `${categoryAccent(colors, category)}20` `` — an
 * eight-digit RN colour whose trailing byte is the alpha, so `20` there is
 * 0x20/255, not 20%. Both readings are measured below; this is the shipped one.
 */
const PRODUCTION_TINT = 0x20 / 255;

/** The nominal `/20` a Tailwind-style tint would mean. */
const NOMINAL_TINT = 0.2;

/** Every key `categoryAccent` answers for, plus one it has no pillar for. */
const CATEGORIES = ['training', 'nutrition', 'recipes', 'recovery', 'mobility', 'custom'] as const;

function channel(value: number): number {
  const c = value / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

/** WCAG relative luminance of a `#rrggbb` string. */
function luminance(hex: string): number {
  const n = hex.replace('#', '');
  const [r, g, b] = [0, 2, 4].map((i) => channel(parseInt(n.slice(i, i + 2), 16)));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG 2.x contrast ratio between two `#rrggbb` strings. */
function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Composite `fg` at `alpha` over opaque `bg` — how a tint resolves on screen. */
function composite(fg: string, alpha: number, bg: string): string {
  const parse = (hex: string) =>
    [0, 2, 4].map((i) => parseInt(hex.replace('#', '').slice(i, i + 2), 16));
  const front = parse(fg);
  const back = parse(bg);
  return `#${front
    .map((f, i) => Math.round(alpha * f + (1 - alpha) * back[i]))
    .map((v) => v.toString(16).padStart(2, '0'))
    .join('')}`;
}

/**
 * The grounds a category badge sits on: the page canvas, the resting card fill
 * — a different tier per scheme (`useCardStyle`) — and the tiers between them.
 */
const GROUNDS: Record<'light' | 'dark', ReadonlyArray<readonly [string, string]>> = {
  light: [
    ['surface', BOREAL_LIGHT.surface],
    ['card fill / surface-container-lowest', BOREAL_LIGHT.surfaceContainerLowest],
    ['surface-container-low', BOREAL_LIGHT.surfaceContainerLow],
    ['surface-container', BOREAL_LIGHT.surfaceContainer],
    ['surface-container-high', BOREAL_LIGHT.surfaceContainerHigh],
  ],
  dark: [
    ['surface', BOREAL_DARK.surface],
    ['card fill / surface-container-high', BOREAL_DARK.surfaceContainerHigh],
    ['surface-container-lowest', BOREAL_DARK.surfaceContainerLowest],
    ['surface-container-low', BOREAL_DARK.surfaceContainerLow],
    ['surface-container', BOREAL_DARK.surfaceContainer],
  ],
};

/** Resolve the live palette under a real ThemeProvider pinned to `pref`. */
async function paletteFor(pref: 'light' | 'dark'): Promise<ThemeColors> {
  await AsyncStorage.setItem(APPEARANCE_KEY, pref);
  const expectedPrimary = pref === 'dark' ? BOREAL_DARK.primary : BOREAL_LIGHT.primary;
  const view = renderHook(() => useThemeColors(), {
    wrapper: ({ children }: { children: React.ReactNode }) => <ThemeProvider>{children}</ThemeProvider>,
  });
  // The preference reads from storage on the first effect, so the first frame
  // is the 'dark' default regardless of what the test asked for.
  await waitFor(() => expect(view.result.current.tokens.primary).toBe(expectedPrimary));
  return view.result.current;
}

describe('the contrast helper agrees with WCAG', () => {
  it('reproduces the reference ratios', () => {
    expect(contrast('#000000', '#ffffff')).toBeCloseTo(21, 5);
    expect(contrast('#ffffff', '#ffffff')).toBeCloseTo(1, 5);
    // The canonical AA example: #767676 is the darkest grey that passes on white.
    expect(contrast('#767676', '#ffffff')).toBeGreaterThanOrEqual(AA_TEXT);
    expect(contrast('#777777', '#ffffff')).toBeLessThan(AA_TEXT);
  });

  it('composites a tint the way a renderer does', () => {
    expect(composite('#000000', 0.5, '#ffffff')).toBe('#808080');
    expect(composite('#0f7d68', 0, '#f7f6f2')).toBe('#f7f6f2');
    expect(composite('#0f7d68', 1, '#f7f6f2')).toBe('#0f7d68');
  });
});

describe('the category pair follows the appearance setting', () => {
  it('answers with a different accent and ink in each scheme, for every category', async () => {
    const light = await paletteFor('light');
    const dark = await paletteFor('dark');

    // A hardcoded hex map — which is what all three screens carried before —
    // returns one value per category and would fail every row here.
    const shared = CATEGORIES.filter(
      (category) =>
        categoryAccent(light, category) === categoryAccent(dark, category) ||
        categoryInk(light, category) === categoryInk(dark, category),
    );
    expect(shared).toEqual([]);
  });

  it('resolves each pillar category to its live token, not a literal', async () => {
    const light = await paletteFor('light');
    const dark = await paletteFor('dark');

    for (const colors of [light, dark]) {
      expect(categoryAccent(colors, 'training')).toBe(colors.pierre.activity);
      expect(categoryAccent(colors, 'nutrition')).toBe(colors.pierre.nutrition);
      expect(categoryAccent(colors, 'recovery')).toBe(colors.pierre.recovery);
      expect(categoryAccent(colors, 'mobility')).toBe(colors.pierre.mobility);

      expect(categoryInk(colors, 'training')).toBe(colors.ink.activity);
      expect(categoryInk(colors, 'nutrition')).toBe(colors.ink.nutrition);
      expect(categoryInk(colors, 'recovery')).toBe(colors.ink.recovery);
      expect(categoryInk(colors, 'mobility')).toBe(colors.ink.mobility);
    }

    // One concrete hex per scheme, so a rebase that repoints `colors.pierre.*`
    // at the wrong half of the token set is caught here rather than by eye.
    // These are the Product-tier pillar values DESIGN.md §2 carries.
    expect(categoryAccent(light, 'training')).toBe('#0f7d68');
    expect(categoryInk(light, 'training')).toBe('#0b5748');
    expect(categoryAccent(dark, 'training')).toBe('#79a694');
    expect(categoryInk(dark, 'training')).toBe('#9abcae');
  });

  it('gives `recipes` the nutrition hue in both schemes', async () => {
    // `recipes` has no pillar of its own — it is food, so it borrows one rather
    // than reaching for the stock Tailwind orange it used to carry.
    for (const scheme of ['light', 'dark'] as const) {
      const colors = await paletteFor(scheme);
      expect(categoryAccent(colors, 'recipes')).toBe(categoryAccent(colors, 'nutrition'));
      expect(categoryInk(colors, 'recipes')).toBe(categoryInk(colors, 'nutrition'));
      expect(categoryAccent(colors, 'recipes')).toBe(colors.pierre.nutrition);
      expect(categoryInk(colors, 'recipes')).toBe(colors.ink.nutrition);
    }
  });

  it('falls back to the primary pair for a category it has no pillar for', async () => {
    for (const scheme of ['light', 'dark'] as const) {
      const colors = await paletteFor(scheme);
      // `custom` is a real stored category with no pillar; the rest are keys a
      // future build could send. All four get the same honest answer.
      const resolved = ['custom', 'strength', '', 'Training'].map((key) => [
        categoryAccent(colors, key),
        categoryInk(colors, key),
      ]);
      expect(resolved).toEqual(
        resolved.map(() => [colors.tokens.primary, colors.tokens.onPrimaryContainer]),
      );

      // The match is on the stored English key, so casing is not normalised —
      // `Training` is a different category and takes the fallback, not activity.
      expect(categoryAccent(colors, 'Training')).not.toBe(categoryAccent(colors, 'training'));
    }
  });
});

describe('the ink clears AA on a tint of its own accent', () => {
  for (const scheme of ['light', 'dark'] as const) {
    it(`every category, ${scheme}`, async () => {
      const colors = await paletteFor(scheme);
      const failures: string[] = [];

      for (const category of CATEGORIES) {
        const accent = categoryAccent(colors, category);
        const ink = categoryInk(colors, category);

        for (const [groundName, ground] of GROUNDS[scheme]) {
          for (const alpha of [PRODUCTION_TINT, NOMINAL_TINT]) {
            const tinted = composite(accent, alpha, ground);
            const ratio = contrast(ink, tinted);
            if (ratio < AA_TEXT) {
              failures.push(
                `${category}: ink ${ink} on ${accent} @${alpha.toFixed(3)} over ${groundName} (${tinted}) = ${ratio.toFixed(2)}:1`,
              );
            }
          }
        }
      }

      expect(failures).toEqual([]);
    });
  }

  it('is the reason the accent itself may not be the label', async () => {
    // The pairing this replaces: the hue drawn as text on a tint of itself.
    // Light `nutrition` is the worst of the set — 2.81:1 on the canvas, 3.00:1
    // on a card, both short of AA — while its bound ink clears 6.5:1 on the
    // same two grounds. If a later edit makes the accent legible enough to pass
    // here, the pair has collapsed into one colour and this fails.
    const light = await paletteFor('light');
    const accent = categoryAccent(light, 'nutrition');
    const ink = categoryInk(light, 'nutrition');

    const onCanvas = composite(accent, PRODUCTION_TINT, BOREAL_LIGHT.surface);
    const onCard = composite(accent, PRODUCTION_TINT, BOREAL_LIGHT.surfaceContainerLowest);

    expect(contrast(accent, onCanvas)).toBeLessThan(AA_TEXT);
    expect(contrast(accent, onCard)).toBeLessThan(AA_TEXT);
    expect(contrast(ink, onCanvas)).toBeGreaterThanOrEqual(6.5);
    expect(contrast(ink, onCard)).toBeGreaterThanOrEqual(6.5);
  });
});
