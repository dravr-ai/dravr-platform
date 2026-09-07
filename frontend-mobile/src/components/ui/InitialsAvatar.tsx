// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: An initials circle on one of the six avatar slot colours — the list row, the thread header, a member row
// ABOUTME: The slot comes from @pierre/chat-utils avatarSlot, so the same thread is the same colour on web and mobile

import React from 'react';
import { View, Text } from 'react-native';
import { AVATAR_SLOTS } from '@pierre/chat-utils';
import { useThemeColors } from '../../constants/theme';

type ThemeColors = ReturnType<typeof useThemeColors>;

/** One avatar slot: the hue tinted behind the initials, and the ink they take on it. */
interface AvatarSlotPair {
  /** The hue tinted at {@link TINT_ALPHA} to make the circle. */
  readonly fill: string;
  /** The bound ink the initials are drawn in over that tint. */
  readonly ink: string;
}

/**
 * The avatar slots, indexed by `avatarSlot` — each a fill and its ink.
 *
 * Fill and ink are one entry because they are one decision. A hue drawn as
 * text on a tint of itself measures as low as 2.35:1 (light `nutrition` over
 * `surfaceContainerLow`), so every fill here carries the ink that clears
 * 4.5:1 over it. Two parallel lists would let a seventh slot arrive with a
 * fill and no ink; one list of pairs cannot.
 *
 * Every value is a design token of the active scheme, so an avatar flips with
 * the theme like the rest of the chrome. Each hue binds the ink of its own
 * container: the primary takes `onPrimaryContainer`, the four pillars take
 * `colors.ink.*`. Tertiary inks itself — in light it is `#03231d`, already the
 * ink end of its own lightness axis, and `onTertiaryContainer` is the pale ink
 * of a *dark* container, which reads 1.68:1 over this pale tint. That is the
 * same pairing the web ships in `avatarSlotClass` (`bg-tertiary/15
 * text-tertiary`).
 *
 * The list is exactly {@link AVATAR_SLOTS} long; the hash that picks a slot
 * counts on that.
 */
function avatarSlotPairs(colors: ThemeColors): readonly AvatarSlotPair[] {
  return [
    { fill: colors.tokens.primary, ink: colors.tokens.onPrimaryContainer },
    { fill: colors.pierre.activity, ink: colors.ink.activity },
    { fill: colors.pierre.nutrition, ink: colors.ink.nutrition },
    { fill: colors.pierre.recovery, ink: colors.ink.recovery },
    { fill: colors.pierre.mobility, ink: colors.ink.mobility },
    { fill: colors.tokens.tertiary, ink: colors.tokens.tertiary },
  ];
}

/** The fill of each avatar slot, in slot order — the palette without its inks. */
export function avatarSlotColors(colors: ThemeColors): readonly string[] {
  return avatarSlotPairs(colors).map((pair) => pair.fill);
}

/** Alpha suffix that tints the circle behind the initials. */
const TINT_ALPHA = '33';

export interface InitialsAvatarProps {
  /** Up to two letters, already upper-cased by `initialsFor`. */
  initials: string;
  /** Index into {@link avatarSlotColors}, `0..AVATAR_SLOTS-1`. */
  slot: number;
  /** Diameter in points; the standard list row uses 40. */
  size?: number;
  testID?: string;
}

/** The circle every conversation-shaped surface draws before a title. */
export function InitialsAvatar({ initials, slot, size = 40, testID }: InitialsAvatarProps) {
  const colors = useThemeColors();
  const palette = avatarSlotPairs(colors);
  const { fill, ink } = palette[((slot % AVATAR_SLOTS) + AVATAR_SLOTS) % AVATAR_SLOTS];

  return (
    <View
      testID={testID}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={{
        width: size,
        height: size,
        borderRadius: size / 2,
        backgroundColor: `${fill}${TINT_ALPHA}`,
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      {/*
        The initials take the slot's bound ink, never its fill. Hiding the
        circle from the accessibility tree settles what VoiceOver announces,
        not what anyone reads: the two audiences are disjoint, and withholding
        the initials from the screen reader leaves the pixels as the only way a
        low-vision athlete gets this thread's identity. So it is visible text
        under 1.4.3, not decoration, and it takes the 4.5:1 bar — at both
        shipped sizes, since 0.4x of 40 and of 32 is 16px and 13px, under the
        18.66px bold that would earn the 3:1 relief.
      */}
      <Text style={{ color: ink, fontSize: Math.round(size * 0.4), fontWeight: '700' }}>{initials}</Text>
    </View>
  );
}
