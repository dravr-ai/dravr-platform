// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Measures the mobile credential login's brand plates — a white label clears 4.5:1 on every stop, a white glyph 3:1 on its tile
// ABOUTME: Pins that a disabled plate keeps its brand and fades, and that the spinner and number-match digit follow the scheme

import React from 'react';
import { ActivityIndicator, StyleSheet } from 'react-native';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { Mail } from 'lucide-react-native';
import { BOREAL_DARK, PROVIDER_COLORS } from '@pierre/shared-constants';
import type { SciotteTarget } from '@pierre/shared-types';
import { SciotteLoginModal } from '../SciotteLoginModal';
import { oauthApi } from '../../services/api';

/** A rendered element, as the queries return it. */
type ReactTestInstance = ReturnType<typeof screen.getByTestId>;

jest.mock('expo-linking', () => ({ parse: jest.fn(), createURL: jest.fn() }));
jest.mock('../../utils/oauth', () => ({ getOAuthCallbackUrl: () => 'dravr://oauth-callback' }));
jest.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: jest.fn(),
    sciotteSelect2FA: jest.fn(),
    sciotteSubmitOTP: jest.fn(),
    initMobileOAuth: jest.fn(),
  },
}));
jest.mock('../OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));

const sciotteLogin = oauthApi.sciotteLogin as jest.Mock;
const sciotteSelect2FA = oauthApi.sciotteSelect2FA as jest.Mock;

const WHITE = '#FFFFFF';
const TARGETS: SciotteTarget[] = ['strava', 'garmin', 'trainingpeaks'];

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

/**
 * The nearest host view above `node` that is a brand plate. The jest setup's
 * gradient renders as a view that keeps its stops as `colors`.
 */
function plateAround(node: ReactTestInstance): { colors: string[]; opacity: number } {
  let current: ReactTestInstance | null = node;
  while (current) {
    if (typeof current.type === 'string' && Array.isArray(current.props.colors)) {
      const style = StyleSheet.flatten(current.props.style) ?? {};
      return { colors: current.props.colors as string[], opacity: (style.opacity as number) ?? 1 };
    }
    current = current.parent;
  }
  throw new Error('no brand plate above this node');
}

/** The nearest host view above `node` that paints a background. */
function tileAround(node: ReactTestInstance): string {
  let current: ReactTestInstance | null = node.parent;
  while (current) {
    if (typeof current.type === 'string') {
      const fill = StyleSheet.flatten(current.props.style)?.backgroundColor;
      if (typeof fill === 'string') return fill;
    }
    current = current.parent;
  }
  throw new Error('no filled tile above this node');
}

/** Render the login and bring it to its credentials form. */
function openCredentials(target: SciotteTarget, consentRequired = false) {
  const view = render(
    <SciotteLoginModal
      visible
      onClose={jest.fn()}
      onConnected={jest.fn()}
      target={target}
      consentRequired={consentRequired}
    />,
  );
  if (target === 'strava') {
    // Strava asks how to sign in first; the e-mail row opens its own form.
    fireEvent.press(screen.getByText('Strava Email'));
  }
  return view;
}

function fillCredentials() {
  fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
  fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
}

describe('SciotteLoginModal (mobile) — the brand plates', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it.each(TARGETS)('labels the %s Sign In plate in white that clears 4.5:1 on both stops', (target) => {
    openCredentials(target);
    const label = screen.getByText('Sign In');
    expect(StyleSheet.flatten(label.props.style)?.color).toBe(WHITE);

    const { colors } = plateAround(label);
    expect(colors).toHaveLength(2);
    for (const stop of colors) {
      expect(contrast(WHITE, stop)).toBeGreaterThanOrEqual(4.5);
    }
  });

  it('keeps the brand plate while Sign In is disabled, faded rather than swapped for a grey', () => {
    openCredentials('garmin');
    const disabled = plateAround(screen.getByText('Sign In'));
    expect(disabled.opacity).toBe(0.5);

    fillCredentials();
    const enabled = plateAround(screen.getByText('Sign In'));
    expect(enabled.opacity).toBe(1);
    expect(disabled.colors).toEqual(enabled.colors);
  });

  it('holds the TrainingPeaks plate faded until the notice is ticked', () => {
    openCredentials('trainingpeaks', true);
    fillCredentials();
    expect(plateAround(screen.getByText('Sign In')).opacity).toBe(0.5);

    fireEvent.press(screen.getByTestId('sciotte-tos-consent'));
    expect(plateAround(screen.getByText('Sign In')).opacity).toBe(1);
  });

  it.each([
    ['strava', PROVIDER_COLORS.strava],
    ['garmin', PROVIDER_COLORS.garmin],
    ['trainingpeaks', PROVIDER_COLORS.trainingpeaks],
  ] as const)('draws the %s form mark white on a solid tile of the brand colour', (target, brand) => {
    const { UNSAFE_getByType } = openCredentials(target);
    const mark = UNSAFE_getByType(Mail);
    expect(mark.props.color).toBe(WHITE);
    const tile = tileAround(mark);
    expect(tile).toBe(brand);
    expect(contrast(WHITE, tile)).toBeGreaterThanOrEqual(3);
  });

  it('spins in the Boreal primary, not the brand colour, while signing in', async () => {
    sciotteLogin.mockReturnValue(new Promise(() => undefined));
    openCredentials('trainingpeaks');
    fillCredentials();
    fireEvent.press(screen.getByTestId('sciotte-login-submit'));

    await waitFor(() => expect(sciotteLogin).toHaveBeenCalledTimes(1));
    const spinner = screen.UNSAFE_getByType(ActivityIndicator);
    // Outside a ThemeProvider the palette is the dark default.
    expect(spinner.props.color).toBe(BOREAL_DARK.primary);
    expect(spinner.props.color).not.toBe(PROVIDER_COLORS.trainingpeaks);
  });

  it('shows the number to match in body ink, not the brand colour', async () => {
    sciotteLogin.mockResolvedValue({ status: 'number_match', number: '47' });
    sciotteSelect2FA.mockReturnValue(new Promise(() => undefined));
    openCredentials('garmin');
    fillCredentials();
    fireEvent.press(screen.getByTestId('sciotte-login-submit'));

    const digit = await screen.findByText('47');
    expect(digit.props.className).toContain('text-text-primary');
    expect(StyleSheet.flatten(digit.props.style)?.color).not.toBe(PROVIDER_COLORS.garmin);
  });
});
