// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for Badge — the phone's pill for a state, mirroring the web's `.badge` variants
// ABOUTME: Pins the label, each variant's tint with its bound ink, that ink clearing AA on every ground, and the runtime ink twin naming the class's token

import React from 'react';
import { readFileSync } from 'fs';
import { join } from 'path';
import { render, renderHook, screen, waitFor } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { BOREAL_DARK, BOREAL_LIGHT } from '@pierre/shared-constants';

// NativeWind's own hook needs a stub under jest; the resolved scheme comes
// from the persisted appearance preference, which each test writes.
jest.mock('nativewind', () => ({
  useColorScheme: () => ({ colorScheme: 'dark', setColorScheme: jest.fn() }),
}));

jest.mock('../../../services/api', () => ({
  userApi: { updateTheme: jest.fn().mockResolvedValue(undefined) },
}));

import { ThemeProvider, useThemeColors, type ThemeColors } from '../../../contexts/ThemeContext';
import { Badge, BADGE_TONE, badgeInkColor, type BadgeVariant } from '../Badge';

/** Each variant's ground and the ink bound to it — the web's `.badge-*` pairs, error and primary on their container inks. */
const LOOK: Record<BadgeVariant, { fill: string; ink: string }> = {
  neutral: { fill: 'bg-surface-container-high', ink: 'text-on-surface' },
  primary: { fill: 'bg-primary/15', ink: 'text-on-primary-container' },
  success: { fill: 'bg-success/15', ink: 'text-on-success-container' },
  warning: { fill: 'bg-warning/15', ink: 'text-on-warning-container' },
  error: { fill: 'bg-error/15', ink: 'text-on-error-container' },
  info: { fill: 'bg-info/15', ink: 'text-on-info-container' },
};

const VARIANTS = Object.keys(LOOK) as BadgeVariant[];

const APPEARANCE_KEY = 'pierre.appearance_pref';

/**
 * The generated token block both schemes' classes resolve through. Read
 * because a tone is a class name while `badgeInkColor` and the contrast
 * grounds are runtime hexes, and nothing at runtime joins the two: this is a
 * parity check between the generated CSS and the live palette.
 */
const TOKENS_CSS = readFileSync(join(__dirname, '../../../../boreal-tokens.generated.css'), 'utf8');

/** The `#rrggbb` a `text-<token>` or `bg-<token>[/NN]` class names in one scheme, read from the generated block. */
function classColor(colorClass: string, scheme: 'light' | 'dark'): string {
  const darkStart = TOKENS_CSS.indexOf(':root.dark');
  const block = scheme === 'light' ? TOKENS_CSS.slice(0, darkStart) : TOKENS_CSS.slice(darkStart);
  const token = colorClass.replace(/^(text|bg)-/, '').replace(/\/\d+$/, '');
  const match = block.match(new RegExp(`--color-${token}:\\s*(\\d+) (\\d+) (\\d+);`));
  if (!match) throw new Error(`no --color-${token} in the ${scheme} block`);
  return `#${match
    .slice(1, 4)
    .map((v) => Number(v).toString(16).padStart(2, '0'))
    .join('')}`;
}

/** WCAG 1.4.3 AA for body-size text. */
const AA_TEXT = 4.5;

function channel(value: number): number {
  const c = value / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function rgb(hex: string): number[] {
  return [0, 2, 4].map((i) => parseInt(hex.replace('#', '').slice(i, i + 2), 16));
}

/** WCAG 2.x contrast ratio between two `#rrggbb` strings. */
function contrast(a: string, b: string): number {
  const lum = (hex: string) => {
    const [r, g, b] = rgb(hex).map(channel);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** The colour a `bg-<token>` or `bg-<token>/NN` fill resolves to over an opaque ground. */
function fillOver(fillClass: string, scheme: 'light' | 'dark', ground: string): string {
  const alphaMatch = fillClass.match(/\/(\d+)$/);
  const alpha = alphaMatch ? Number(alphaMatch[1]) / 100 : 1;
  const front = rgb(classColor(fillClass, scheme));
  const back = rgb(ground);
  return `#${front
    .map((f, i) => Math.round(alpha * f + (1 - alpha) * back[i]))
    .map((v) => v.toString(16).padStart(2, '0'))
    .join('')}`;
}

/**
 * The grounds a badge can sit on: every tier from the canvas to
 * `surface-container-highest`, in both schemes — the card fill of each scheme
 * (`surface-container-lowest` in light, `surface-container-high` in dark), the
 * tiers a sheet or picker row uses, and the top tier for good measure.
 */
const GROUNDS: Record<'light' | 'dark', ReadonlyArray<readonly [string, string]>> = {
  light: [
    ['surface', BOREAL_LIGHT.surface],
    ['surface-container-lowest', BOREAL_LIGHT.surfaceContainerLowest],
    ['surface-container-low', BOREAL_LIGHT.surfaceContainerLow],
    ['surface-container', BOREAL_LIGHT.surfaceContainer],
    ['surface-container-high', BOREAL_LIGHT.surfaceContainerHigh],
    ['surface-container-highest', BOREAL_LIGHT.surfaceContainerHighest],
  ],
  dark: [
    ['surface', BOREAL_DARK.surface],
    ['surface-container-lowest', BOREAL_DARK.surfaceContainerLowest],
    ['surface-container-low', BOREAL_DARK.surfaceContainerLow],
    ['surface-container', BOREAL_DARK.surfaceContainer],
    ['surface-container-high', BOREAL_DARK.surfaceContainerHigh],
    ['surface-container-highest', BOREAL_DARK.surfaceContainerHighest],
  ],
};

/** Resolve the live palette under a real ThemeProvider pinned to `pref`. */
async function paletteFor(pref: 'light' | 'dark'): Promise<ThemeColors> {
  await AsyncStorage.setItem(APPEARANCE_KEY, pref);
  const expectedPrimary = pref === 'dark' ? BOREAL_DARK.primary : BOREAL_LIGHT.primary;
  const view = renderHook(() => useThemeColors(), {
    wrapper: ({ children }: { children: React.ReactNode }) => <ThemeProvider>{children}</ThemeProvider>,
  });
  await waitFor(() => expect(view.result.current.tokens.primary).toBe(expectedPrimary));
  return view.result.current;
}

describe('Badge', () => {
  it('says its label', () => {
    render(<Badge variant="warning" label="Draft · not shared yet" testID="badge" />);
    expect(screen.getByTestId('badge')).toHaveTextContent('Draft · not shared yet');
  });

  it.each(VARIANTS)('draws the %s variant as a pill on its tint, its label in the bound ink', (variant) => {
    render(<Badge variant={variant} label={variant} testID="badge" />);
    const pill = screen.getByTestId('badge');
    expect(pill.props.className).toContain('rounded-full');
    expect(pill.props.className).toContain(LOOK[variant].fill);
    // React Native has no inherited ink: the label must carry its own colour,
    // or a filled pill falls back to the platform default in dark mode.
    expect(screen.getByText(variant).props.className).toContain(LOOK[variant].ink);
  });

  it('adds layout classes to the recipe without dropping it', () => {
    render(<Badge variant="neutral" label="Convention" className="mt-1 self-start" testID="badge" />);
    const { className } = screen.getByTestId('badge').props;
    expect(className).toContain('mt-1 self-start');
    expect(className).toContain(LOOK.neutral.fill);
  });
});

describe('badgeInkColor', () => {
  it.each(['light', 'dark'] as const)(
    'is, in %s, the colour each variant ink class resolves to',
    async (scheme) => {
      const colors = await paletteFor(scheme);
      for (const variant of VARIANTS) {
        expect(badgeInkColor(variant, colors).toLowerCase()).toBe(classColor(BADGE_TONE[variant].ink, scheme));
      }
    },
  );
});

describe('BADGE_TONE contrast', () => {
  it.each(['light', 'dark'] as const)('every ink clears AA on its fill over every ground, in %s', (scheme) => {
    const failures: string[] = [];
    for (const variant of VARIANTS) {
      const { fill, ink } = BADGE_TONE[variant];
      const inkColor = classColor(ink, scheme);
      for (const [groundName, ground] of GROUNDS[scheme]) {
        const ratio = contrast(inkColor, fillOver(fill, scheme, ground));
        if (ratio < AA_TEXT) failures.push(`${variant} over ${groundName}: ${ratio.toFixed(2)}:1`);
      }
    }
    expect(failures).toEqual([]);
  });

  it('draws error and primary in their container inks because each bare hue falls under AA on its own tint', () => {
    // Error labelled in `error` reads 4.2:1 on light surface-container-low;
    // primary labelled in `primary` reads 4.17:1 on light surface-container-high.
    const errorTint = fillOver(BADGE_TONE.error.fill, 'light', BOREAL_LIGHT.surfaceContainerLow);
    expect(contrast(classColor('text-error', 'light'), errorTint)).toBeLessThan(AA_TEXT);
    expect(contrast(classColor(BADGE_TONE.error.ink, 'light'), errorTint)).toBeGreaterThanOrEqual(6);
    const primaryTint = fillOver(BADGE_TONE.primary.fill, 'light', BOREAL_LIGHT.surfaceContainerHigh);
    expect(contrast(classColor('text-primary', 'light'), primaryTint)).toBeLessThan(AA_TEXT);
    expect(contrast(classColor(BADGE_TONE.primary.ink, 'light'), primaryTint)).toBeGreaterThanOrEqual(6);
  });
});
