// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the ink ProviderGlyph hands each brand mark, per scheme — TrainingPeaks yields to body ink on dark, WHOOP on light
// ABOUTME: Every mark it draws is measured against its scheme's canvas, so no brand colour ships under the 3:1 icon floor

import React from 'react';
import { Text } from 'react-native';
import { render, screen, waitFor } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import Svg, { Circle, Rect } from 'react-native-svg';
import { BOREAL_DARK, BOREAL_LIGHT, PROVIDER_COLORS } from '@pierre/shared-constants';

// NativeWind's own hook needs a stub under jest; the resolved scheme comes
// from the persisted appearance preference, which each test writes.
jest.mock('nativewind', () => ({
  useColorScheme: () => ({ colorScheme: 'dark', setColorScheme: jest.fn() }),
}));

jest.mock('../../services/api', () => ({
  userApi: { updateTheme: jest.fn().mockResolvedValue(undefined) },
}));

import { ThemeProvider, useTheme } from '../../contexts/ThemeContext';
import { ProviderGlyph } from '../ProviderGlyph';

const APPEARANCE_KEY = 'pierre.appearance_pref';
const SCHEMES = ['light', 'dark'] as const;
type Scheme = (typeof SCHEMES)[number];

const TOKENS = { light: BOREAL_LIGHT, dark: BOREAL_DARK } as const;

/** WCAG relative luminance of a `#rrggbb` colour. */
function luminance(hex: string): number {
  const value = hex.replace('#', '');
  const channels = [0, 2, 4].map((offset) => {
    const c = parseInt(value.slice(offset, offset + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Renders the resolved scheme, so a test can wait for storage to answer. */
function SchemeProbe() {
  return <Text testID="scheme-probe">{useTheme().scheme}</Text>;
}

/**
 * Render one glyph under a real ThemeProvider pinned to `scheme`. The
 * provider's first frame is the dark default until storage answers, so the
 * render waits until the probe reports the asked-for scheme.
 */
async function renderGlyph(providerId: string, scheme: Scheme) {
  await AsyncStorage.setItem(APPEARANCE_KEY, scheme);
  const view = render(
    <ThemeProvider>
      <SchemeProbe />
      <ProviderGlyph providerId={providerId} label={providerId} />
    </ThemeProvider>,
  );
  await waitFor(() => expect(screen.getByTestId('scheme-probe')).toHaveTextContent(scheme));
  return view;
}

/** The ink a mark was drawn in — where each mark carries it. */
async function inkOf(providerId: string, scheme: Scheme): Promise<string> {
  const { UNSAFE_getByType } = await renderGlyph(providerId, scheme);
  if (providerId === 'whoop') return UNSAFE_getByType(Rect).props.stroke as string;
  if (providerId === 'garmin' || providerId === 'sciotte_garmin') {
    return UNSAFE_getByType(Circle).props.stroke as string;
  }
  return UNSAFE_getByType(Svg).props.fill as string;
}

describe('ProviderGlyph inks each mark for the active scheme', () => {
  it('draws TrainingPeaks in its blue on light and in body ink on dark', async () => {
    // #005695 is 7.02:1 on the light canvas and 2.46:1 on the dark one.
    expect(await inkOf('sciotte_trainingpeaks', 'light')).toBe(PROVIDER_COLORS.sciotte_trainingpeaks);
    expect(await inkOf('sciotte_trainingpeaks', 'dark')).toBe(BOREAL_DARK.onSurface);
  });

  it('draws WHOOP in body ink on light and in its green on dark', async () => {
    // #00D46A is 1.83:1 on the light canvas and 9.45:1 on the dark one.
    expect(await inkOf('whoop', 'light')).toBe(BOREAL_LIGHT.onSurface);
    expect(await inkOf('whoop', 'dark')).toBe(PROVIDER_COLORS.whoop);
  });

  it.each([
    ['sciotte', PROVIDER_COLORS.sciotte],
    ['strava', PROVIDER_COLORS.strava],
    ['sciotte_garmin', PROVIDER_COLORS.sciotte_garmin],
    ['intervals_icu', PROVIDER_COLORS.intervals_icu],
  ])('keeps %s in its brand colour in both schemes', async (providerId, brand) => {
    for (const scheme of SCHEMES) {
      expect(await inkOf(providerId, scheme)).toBe(brand);
    }
  });

  it.each(['sciotte', 'sciotte_garmin', 'sciotte_trainingpeaks', 'whoop', 'intervals_icu'])(
    'draws %s at 3:1 or better against the canvas in both schemes',
    async (providerId) => {
      for (const scheme of SCHEMES) {
        const ink = await inkOf(providerId, scheme);
        expect(contrast(ink, TOKENS[scheme].surface)).toBeGreaterThanOrEqual(3);
      }
    },
  );
});
