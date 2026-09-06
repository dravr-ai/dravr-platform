// ABOUTME: Pins the phone login to the athlete's appearance setting and to the two-sheet pairing
// ABOUTME: DESIGN.md §5 "Auth and onboarding" — tint page + white sheet in light, paper-dark page + a tier above it in dark
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { BOREAL_DARK, BOREAL_LIGHT } from '@pierre/shared-constants';

// NativeWind's own hook needs a stub under jest; the resolved scheme comes
// from the persisted appearance preference, which each test writes.
jest.mock('nativewind', () => ({
  ...jest.requireActual('nativewind'),
  useColorScheme: () => ({ colorScheme: 'dark', setColorScheme: jest.fn() }),
}));

jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => ({ push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true }),
  useLocalSearchParams: () => ({}),
  useFocusEffect: (cb: () => void) => { require('react').useEffect(cb, []); },
}));

// Only the two calls this screen's providers make are stubbed; the rest of
// the module stays real so `onAuthFailure` and friends keep their shapes.
jest.mock('../src/services/api', () => ({
  ...jest.requireActual('../src/services/api'),
  authApi: { ...jest.requireActual('../src/services/api').authApi, login: jest.fn() },
  userApi: { ...jest.requireActual('../src/services/api').userApi, updateTheme: jest.fn().mockResolvedValue(undefined) },
}));

jest.mock('@expo/vector-icons', () => {
  const { View } = require('react-native');
  return { AntDesign: (props: Record<string, unknown>) => require('react').createElement(View, { testID: `icon-${props.name}` }) };
});

import { i18n } from '@pierre/i18n';
import { ThemeProvider } from '../src/contexts/ThemeContext';
import { AuthProvider } from '../src/contexts/AuthContext';
import { LoginScreen } from '../src/screens/auth/LoginScreen';

const APPEARANCE_KEY = 'pierre.appearance_pref';

function renderInTheme() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ThemeProvider>
        <AuthProvider>
          <LoginScreen />
        </AuthProvider>
      </ThemeProvider>
    </QueryClientProvider>,
  );
}

/** React Native flattens an array style; read the resolved background either way. */
function background(node: { props: { style: unknown } }): string | undefined {
  const style = node.props.style as { backgroundColor?: string } | Array<{ backgroundColor?: string }>;
  const list = Array.isArray(style) ? style : [style];
  return list.reduce<string | undefined>((found, layer) => layer?.backgroundColor ?? found, undefined);
}

describe('the phone login follows the appearance setting', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('pairs the sage tint with a white sheet in light', async () => {
    await AsyncStorage.setItem(APPEARANCE_KEY, 'light');
    const screen = renderInTheme();

    await waitFor(() => {
      expect(background(screen.getByTestId('login-screen'))).toBe(BOREAL_LIGHT.primaryContainer);
    });
    expect(background(screen.getByTestId('login-card'))).toBe(BOREAL_LIGHT.surfaceContainerLowest);
  });

  it('pairs the paper-dark canvas with a tier above it in dark', async () => {
    await AsyncStorage.setItem(APPEARANCE_KEY, 'dark');
    const screen = renderInTheme();

    await waitFor(() => {
      expect(background(screen.getByTestId('login-screen'))).toBe(BOREAL_DARK.surface);
    });
    // A `lowest` card sinks below a near-black ground; the sheet has to sit
    // ABOVE the canvas, which is the same rule the coach bubble follows.
    expect(background(screen.getByTestId('login-card'))).toBe(BOREAL_DARK.surfaceContainerHigh);
  });

  it('builds the headline from the shared keys, as one wrapping sentence', async () => {
    await AsyncStorage.setItem(APPEARANCE_KEY, 'light');
    const screen = renderInTheme();

    // The phone used to hold its own copy of the lead (`app.heroLead`) and
    // hard-break it with a `\n`. Two costs: editing the web pair silently
    // desynced the phone, and the forced break fought the wrap and rendered
    // three ragged lines at 390pt. It now renders the same two keys the web
    // aside uses, joined, and lets the width choose the break — so this
    // asserts ONE text node holding the whole sentence, with no newline.
    // Resolve through the same instance the screen renders with, so the
    // assertion holds in whatever locale a test pins rather than hardcoding
    // one language's words.
    const joined = `${i18n.t('auth.taglineLead')} ${i18n.t('auth.taglineTail')}`;
    await waitFor(() => expect(screen.getByText(joined)).toBeTruthy());
    expect(joined).not.toContain('\n');
  });

  it('carries no hardcoded brand fill that would ignore the setting', async () => {
    // The screen shipped a `#00241a → #0d3b2e` gradient and a card pinned to
    // BOREAL_LIGHT, so dark mode never reached it. Both grounds must differ
    // between the two schemes, which a pinned surface cannot do.
    await AsyncStorage.setItem(APPEARANCE_KEY, 'light');
    const light = renderInTheme();
    await waitFor(() => expect(background(light.getByTestId('login-screen'))).toBe(BOREAL_LIGHT.primaryContainer));
    const lightPage = background(light.getByTestId('login-screen'));
    const lightCard = background(light.getByTestId('login-card'));
    light.unmount();

    await AsyncStorage.setItem(APPEARANCE_KEY, 'dark');
    const dark = renderInTheme();
    await waitFor(() => expect(background(dark.getByTestId('login-screen'))).toBe(BOREAL_DARK.surface));

    expect(background(dark.getByTestId('login-screen'))).not.toBe(lightPage);
    expect(background(dark.getByTestId('login-card'))).not.toBe(lightCard);
  });
});
