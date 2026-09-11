// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that the phone's avatar palette follows AVATAR_SLOT_HUES — the shared slot order, bound to live theme tokens
// ABOUTME: Every slot is checked by content, fill and ink, in both schemes, so a thread is the same hue here as on the web

import React from 'react';
import { render, renderHook, screen, waitFor } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { AVATAR_SLOT_HUES, AVATAR_SLOTS, avatarSlot, type AvatarSlotHue } from '@pierre/chat-utils';
import { BOREAL_DARK, BOREAL_LIGHT } from '@pierre/shared-constants';

// NativeWind's own hook needs a stub under jest; the resolved scheme comes
// from the persisted appearance preference, which each test writes.
jest.mock('nativewind', () => ({
  useColorScheme: () => ({ colorScheme: 'dark', setColorScheme: jest.fn() }),
}));

jest.mock('../../../services/api', () => ({
  userApi: { updateTheme: jest.fn().mockResolvedValue(undefined) },
}));

import { ThemeProvider, type ThemeColors } from '../../../contexts/ThemeContext';
import { useThemeColors } from '../../../constants/theme';
import { InitialsAvatar, avatarSlotColors } from '../InitialsAvatar';

const APPEARANCE_KEY = 'pierre.appearance_pref';

/** The alpha byte `InitialsAvatar` appends to a fill to tint the circle. */
const TINT_ALPHA = '33';

const SCHEMES = ['light', 'dark'] as const;

/** The fill each hue tints the circle with, read off the live palette. */
const FILL_OF: Record<AvatarSlotHue, (colors: ThemeColors) => string> = {
  primary: (colors) => colors.tokens.primary,
  activity: (colors) => colors.pierre.activity,
  nutrition: (colors) => colors.pierre.nutrition,
  recovery: (colors) => colors.pierre.recovery,
  mobility: (colors) => colors.pierre.mobility,
  tertiary: (colors) => colors.tokens.tertiary,
};

/**
 * The ink each hue draws its initials in — the bound container ink for the
 * primary and the four pillars, and tertiary itself, which in light is already
 * the ink end of its own axis while `onTertiaryContainer` is the pale ink of a
 * dark container and reads 1.68:1 over this pale tint.
 */
const INK_OF: Record<AvatarSlotHue, (colors: ThemeColors) => string> = {
  primary: (colors) => colors.tokens.onPrimaryContainer,
  activity: (colors) => colors.ink.activity,
  nutrition: (colors) => colors.ink.nutrition,
  recovery: (colors) => colors.ink.recovery,
  mobility: (colors) => colors.ink.mobility,
  tertiary: (colors) => colors.tokens.tertiary,
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

/**
 * The circle withholds itself and its initials from the accessibility tree —
 * a screen reader hears the row's title, not two letters — and RNTL's
 * queries respect that by default, so reading the pixels means asking for
 * the hidden elements explicitly.
 */
const HIDDEN = { includeHiddenElements: true };

/** The circle's `backgroundColor` and the initials' `color`, as the tree carries them. */
function drawn(): { fill: string; ink: string } {
  const circle = screen.getByTestId('avatar', HIDDEN).props.style as { backgroundColor: string };
  const text = screen.getByText('AB', HIDDEN).props.style as { color: string };
  return { fill: circle.backgroundColor, ink: text.color };
}

/**
 * Render one avatar under a real ThemeProvider pinned to `pref` and return
 * what it drew alongside the palette it drew from.
 *
 * The provider's first frame is the dark default until storage answers, so
 * the render is awaited until the circle carries a fill of the asked-for
 * scheme — the same wait `paletteFor` does on `tokens.primary`.
 */
async function renderAvatar(pref: 'light' | 'dark', slot: number) {
  const colors = await paletteFor(pref);
  render(
    <ThemeProvider>
      <InitialsAvatar initials="AB" slot={slot} testID="avatar" />
    </ThemeProvider>,
  );
  const fills = avatarSlotColors(colors).map((fill) => `${fill}${TINT_ALPHA}`);
  await waitFor(() => expect(fills).toContain(drawn().fill));
  return { colors, ...drawn() };
}

describe('avatarSlotColors follows the shared slot order', () => {
  it('has one distinct design-token fill per shared slot', async () => {
    for (const scheme of SCHEMES) {
      const fills = avatarSlotColors(await paletteFor(scheme));
      expect(fills).toHaveLength(AVATAR_SLOTS);
      expect(new Set(fills).size).toBe(AVATAR_SLOTS);
      for (const fill of fills) expect(fill).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });

  it('puts each hue at the index AVATAR_SLOT_HUES gives it', async () => {
    for (const scheme of SCHEMES) {
      const colors = await paletteFor(scheme);
      const fills = avatarSlotColors(colors);

      // The shared list is the only order either client reads, so the check
      // is by hue name, not by a number this file would have to agree on.
      for (const hue of AVATAR_SLOT_HUES) {
        expect(fills[AVATAR_SLOT_HUES.indexOf(hue)]).toBe(FILL_OF[hue](colors));
      }
    }
  });

  it('resolves to the active scheme, not one literal per slot', async () => {
    const light = avatarSlotColors(await paletteFor('light'));
    const dark = avatarSlotColors(await paletteFor('dark'));

    const shared = AVATAR_SLOT_HUES.filter(
      (hue) => light[AVATAR_SLOT_HUES.indexOf(hue)] === dark[AVATAR_SLOT_HUES.indexOf(hue)],
    );
    expect(shared).toEqual([]);

    // One concrete hex per scheme, so a rebase that repoints `colors.pierre.*`
    // at the wrong half of the token set is caught here rather than by eye.
    expect(light[AVATAR_SLOT_HUES.indexOf('activity')]).toBe('#0f7d68');
    expect(dark[AVATAR_SLOT_HUES.indexOf('activity')]).toBe('#79a694');
  });
});

describe('InitialsAvatar draws the slot it is given', () => {
  for (const scheme of SCHEMES) {
    for (const hue of AVATAR_SLOT_HUES) {
      it(`tints the circle with ${hue} and inks the initials with its bound ink, ${scheme}`, async () => {
        const { colors, fill, ink } = await renderAvatar(scheme, AVATAR_SLOT_HUES.indexOf(hue));

        expect(fill).toBe(`${FILL_OF[hue](colors)}${TINT_ALPHA}`);
        expect(ink).toBe(INK_OF[hue](colors));
        // The initials take the ink, never the fill: a pillar hue drawn on a
        // tint of itself is the low-contrast pairing the bound ink replaces.
        // Tertiary is the one hue that is its own ink.
        if (hue !== 'tertiary') expect(ink).not.toBe(FILL_OF[hue](colors));
      });
    }
  }

  it('renders the initials on the slot the shared hash picked', async () => {
    const slot = avatarSlot({ id: 'conv-1', agent_id: null, group_id: null });
    const { colors, fill } = await renderAvatar('dark', slot);

    expect(slot).toBeGreaterThanOrEqual(0);
    expect(slot).toBeLessThan(AVATAR_SLOTS);
    expect(fill).toBe(`${FILL_OF[AVATAR_SLOT_HUES[slot]](colors)}${TINT_ALPHA}`);
  });

  it('shows the initials on screen and withholds them from the screen reader', async () => {
    await renderAvatar('dark', 0);

    // Visible text under WCAG 1.4.3 — the pixels are how a sighted athlete
    // reads the thread's identity — yet not a second announcement of the row.
    expect(screen.getByText('AB', HIDDEN)).toBeTruthy();
    expect(screen.queryByText('AB')).toBeNull();
    expect(screen.queryByTestId('avatar')).toBeNull();
  });
});

describe('InitialsAvatar wraps a slot outside the palette', () => {
  it('reads AVATAR_SLOTS as slot 0', async () => {
    const { colors, fill, ink } = await renderAvatar('dark', AVATAR_SLOTS);

    expect(fill).toBe(`${colors.tokens.primary}${TINT_ALPHA}`);
    expect(ink).toBe(colors.tokens.onPrimaryContainer);
  });

  it('reads -1 as the last slot', async () => {
    const { colors, fill, ink } = await renderAvatar('dark', -1);

    expect(fill).toBe(`${colors.tokens.tertiary}${TINT_ALPHA}`);
    expect(ink).toBe(colors.tokens.tertiary);
  });
});
