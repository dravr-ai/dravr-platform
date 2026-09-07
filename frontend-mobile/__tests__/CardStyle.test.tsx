// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the resting-card recipe: a real fill that follows the appearance setting, a hairline, no shadow
// ABOUTME: The const it replaced named `background` (which RN ignores) and paired it with a 24pt shadow, so ~15 surfaces drew a ghost of their own text

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

import { ThemeProvider } from '../src/contexts/ThemeContext';
import { useCardStyle } from '../src/constants/theme';

const APPEARANCE_KEY = 'pierre.appearance_pref';

/** WCAG relative luminance, so "lighter than" is measured rather than eyeballed. */
function luminance(hex: string): number {
  const value = hex.replace('#', '');
  const channels = [0, 2, 4].map((offset) => {
    const c = parseInt(value.slice(offset, offset + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

/** Render the hook under a real ThemeProvider resolved to `pref`. */
async function cardStyleFor(pref: 'light' | 'dark') {
  await AsyncStorage.setItem(APPEARANCE_KEY, pref);
  const expectedFill =
    pref === 'dark' ? BOREAL_DARK.surfaceContainerHigh : BOREAL_LIGHT.surfaceContainerLowest;
  const view = renderHook(() => useCardStyle(), {
    wrapper: ({ children }: { children: React.ReactNode }) => <ThemeProvider>{children}</ThemeProvider>,
  });
  // The preference reads from storage on the first effect, so the first frame
  // is the 'dark' default regardless of what the test asked for.
  await waitFor(() => expect(view.result.current.backgroundColor).toBe(expectedFill));
  return view.result.current;
}

describe('the resting-card recipe', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('fills the card, and fills it differently in each scheme', async () => {
    const light = await cardStyleFor('light');
    const dark = await cardStyleFor('dark');

    // `glassCard` passed `background:`, which React Native does not implement,
    // so every surface wearing it had no fill at all.
    expect(light.backgroundColor).toBe(BOREAL_LIGHT.surfaceContainerLowest);
    expect(dark.backgroundColor).toBe(BOREAL_DARK.surfaceContainerHigh);
    expect(dark.backgroundColor).not.toBe(light.backgroundColor);
    expect('background' in light).toBe(false);
  });

  it('lifts the card ABOVE the canvas in both schemes', async () => {
    const light = await cardStyleFor('light');
    const dark = await cardStyleFor('dark');

    // `surfaceContainerLowest` is #0b0e0b in dark — BELOW the #11130f canvas —
    // so the token that lifts a card in light sinks it in dark. Measured, so a
    // future token edit that inverts the pair fails here rather than shipping.
    expect(luminance(dark.backgroundColor as string)).toBeGreaterThan(
      luminance(BOREAL_DARK.surface),
    );
    expect(luminance(light.backgroundColor as string)).toBeGreaterThan(
      luminance(BOREAL_LIGHT.surface),
    );
  });

  it('carries a hairline and no shadow', async () => {
    const light = await cardStyleFor('light');
    const dark = await cardStyleFor('dark');

    // DESIGN.md §4 — hairlines lift, shadows float, and a resting card does
    // not float. A fill-less view with `shadowOffset: { height: 24 }` casts
    // the alpha of its CHILDREN on iOS, which is how these screens drew a soft
    // duplicate of their own text 24pt below itself.
    for (const key of ['shadowColor', 'shadowOffset', 'shadowOpacity', 'shadowRadius', 'elevation']) {
      expect(light).not.toHaveProperty(key);
      expect(dark).not.toHaveProperty(key);
    }

    expect(light.borderWidth).toBe(1);
    expect(dark.borderWidth).toBe(1);
    // The hairline is scheme-bound too: light takes the darker Product-tier
    // ghost border, dark the pale one. A fixed border is the same bug one
    // layer out.
    expect(light.borderColor).toBe('rgba(155, 165, 159, 0.4)');
    expect(dark.borderColor).toBe('rgba(192, 200, 195, 0.14)');
  });
});
