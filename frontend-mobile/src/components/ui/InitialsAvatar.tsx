// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: An initials circle on one of the six avatar slot colours — the list row, the thread header, a member row
// ABOUTME: The slot comes from @pierre/chat-utils avatarSlot, so the same thread is the same colour on web and mobile

import React from 'react';
import { View, Text } from 'react-native';
import { AVATAR_SLOTS } from '@pierre/chat-utils';
import { useThemeColors } from '../../constants/theme';

type ThemeColors = ReturnType<typeof useThemeColors>;

/**
 * The avatar colours, indexed by `avatarSlot`.
 *
 * Every entry is a design token of the active scheme — the primary, the four
 * pillar tints and the tertiary — so an avatar flips with the theme like the
 * rest of the chrome. The list is exactly {@link AVATAR_SLOTS} long; the hash
 * that picks a slot counts on that.
 */
export function avatarSlotColors(colors: ThemeColors): readonly string[] {
  return [
    colors.tokens.primary,
    colors.pierre.activity,
    colors.pierre.nutrition,
    colors.pierre.recovery,
    colors.pierre.mobility,
    colors.tokens.tertiary,
  ];
}

/** Alpha suffix that tints the circle behind same-hue initials. */
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
  const palette = avatarSlotColors(colors);
  const tint = palette[((slot % AVATAR_SLOTS) + AVATAR_SLOTS) % AVATAR_SLOTS];

  return (
    <View
      testID={testID}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={{
        width: size,
        height: size,
        borderRadius: size / 2,
        backgroundColor: `${tint}${TINT_ALPHA}`,
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <Text style={{ color: tint, fontSize: Math.round(size * 0.4), fontWeight: '700' }}>{initials}</Text>
    </View>
  );
}
