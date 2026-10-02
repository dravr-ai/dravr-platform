// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that the palette a screen reaches for follows the appearance setting, and that no fixed one is on offer
// ABOUTME: The retired gradient strip, the v1 primary literals and the second `warning` source each shipped for months

import React from 'react';
import { Text } from 'react-native';
import { render, waitFor } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import {
  BOREAL_DARK,
  BOREAL_LIGHT,
  SEMANTIC_COLORS,
  SEMANTIC_COLORS_DARK,
} from '@pierre/shared-constants';

// NativeWind's own hook needs a stub under jest; the resolved scheme comes
// from the persisted appearance preference, which each test writes.
jest.mock('nativewind', () => ({
  ...jest.requireActual('nativewind'),
  useColorScheme: () => ({ colorScheme: 'dark', setColorScheme: jest.fn() }),
}));

jest.mock('../src/services/api', () => ({
  userApi: { updateTheme: jest.fn().mockResolvedValue(undefined) },
}));

import * as theme from '../src/constants/theme';
import { ThemeProvider } from '../src/contexts/ThemeContext';

const APPEARANCE_KEY = 'pierre.appearance_pref';
const FEEDBACK = ['success', 'warning', 'error', 'info'] as const;

/** Prints the live palette's feedback colours and its primary, as a screen would read them. */
function Probe(): React.ReactElement {
  const colors = theme.useThemeColors();
  return (
    <>
      {FEEDBACK.map((name) => (
        <Text key={name} testID={`feedback-${name}`}>
          {colors[name]}
        </Text>
      ))}
      <Text testID="primary">{colors.tokens.primary}</Text>
    </>
  );
}

async function paletteUnder(pref: 'light' | 'dark'): Promise<Record<string, string>> {
  await AsyncStorage.setItem(APPEARANCE_KEY, pref);
  const view = render(
    <ThemeProvider>
      <Probe />
    </ThemeProvider>,
  );
  const expectedPrimary = pref === 'light' ? BOREAL_LIGHT.primary : BOREAL_DARK.primary;
  // The stored preference loads asynchronously; wait for the scheme it names.
  await waitFor(() => expect(view.getByTestId('primary').props.children).toBe(expectedPrimary));
  const read = Object.fromEntries(
    FEEDBACK.map((name) => [name, view.getByTestId(`feedback-${name}`).props.children as string]),
  );
  view.unmount();
  return read;
}

/**
 * Two of the four properties this file guards are rules about what may be
 * written anywhere under src/ — no `#00241a`/`#0d3b2e` literal, no gradient
 * drawn from a module-level `gradients` palette. Those are ESLint rules
 * (`schemeAndChromeRestrictions` in eslint.config.js), proven to fire by
 * __tests__/designRuleLint.test.ts. The two below are about what
 * the modules do, so they are asked of the modules.
 */
describe('no athlete-facing surface pins a colour the scheme cannot move', () => {
  it('offers no module-level palette a screen could reach for', () => {
    // `gradients` and `colors` were module-level `as const`s, so any component
    // drawing from either rendered the same colours in light and dark. Seven
    // screens took `gradients.violetCyan`; the settings screen took `colors`.
    // Pinning the exports rather than the call sites is the stronger claim: a
    // palette that does not exist cannot acquire a caller.
    expect(Object.keys(theme)).not.toContain('gradients');
    expect(Object.keys(theme)).not.toContain('colors');
    // The live palette is the one way to a colour that follows the setting.
    expect(typeof theme.useThemeColors).toBe('function');
  });

  it('resolves every feedback colour from the one shared set, in each scheme', async () => {
    // The phone answered with two different ambers for `warning` depending on
    // whether a component read a NativeWind class or useThemeColors(): the
    // hook carried `#8f6a2e`, the Editorial-tier value, in light.
    const light = await paletteUnder('light');
    const dark = await paletteUnder('dark');

    expect(light).toEqual({ ...SEMANTIC_COLORS });
    expect(dark).toEqual({ ...SEMANTIC_COLORS_DARK });
    // The setting moves them: a palette fixed at module level would not.
    expect(light.warning).not.toBe(dark.warning);

    // And the shared set is the Product tier, so the fix points at the right one.
    expect(SEMANTIC_COLORS.warning).toBe('#b08326');
    expect(SEMANTIC_COLORS_DARK.warning).toBe('#d6b87a');
  });
});
