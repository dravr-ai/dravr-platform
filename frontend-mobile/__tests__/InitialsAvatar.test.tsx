// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the initials avatar — one token colour per slot, exactly as many slots as the shared hash yields
// ABOUTME: Pins the palette length to AVATAR_SLOTS so a slot can never index past the colours a theme provides

import React from 'react';
import { render, renderHook } from '@testing-library/react-native';
import { AVATAR_SLOTS, avatarSlot } from '@pierre/chat-utils';
import { InitialsAvatar, avatarSlotColors } from '../src/components/ui/InitialsAvatar';
import { useThemeColors } from '../src/constants/theme';

describe('InitialsAvatar', () => {
  it('provides exactly one design-token colour per avatar slot', () => {
    const { result } = renderHook(() => useThemeColors());
    const palette = avatarSlotColors(result.current);
    expect(palette).toHaveLength(AVATAR_SLOTS);
    expect(new Set(palette).size).toBe(AVATAR_SLOTS);
    for (const colour of palette) expect(colour).toMatch(/^#[0-9a-f]{6}$/i);
  });

  it('renders the initials on the slot the shared hash picked', () => {
    const slot = avatarSlot({ id: 'conv-1', coach_id: null, group_id: null });
    const { getByTestId } = render(<InitialsAvatar initials="TT" slot={slot} testID="avatar" />);
    // Decorative by design: hidden from the accessibility tree, so the query
    // has to ask for hidden elements to see it at all.
    expect(getByTestId('avatar', { includeHiddenElements: true })).toHaveTextContent('TT');
    expect(slot).toBeGreaterThanOrEqual(0);
    expect(slot).toBeLessThan(AVATAR_SLOTS);
  });
});
