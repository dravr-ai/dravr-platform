// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Dravr lockup — the badge mark beside the DRAVR wordmark, set the way DESIGN.md §1 defines it
// ABOUTME: The phone's only in-app identity: it stands in for the chat tab's screen title (DESIGN.md §5)

import React from 'react';
import { View, Text, Image, type TextStyle } from 'react-native';
import { PRODUCT_WORDMARK } from '@pierre/shared-constants';
import { spacing, useThemeColors } from '../../constants/theme';

/** Tracking is defined as a ratio of the type size (`0.15em`); RN wants px. */
const BRAND_TRACKING_RATIO = 0.15;

interface BrandLockupProps {
  /** Height and width of the badge mark. The wordmark is set to match. */
  size?: number;
  testID?: string;
  /** Spoken name of the region the lockup heads. */
  accessibilityLabel?: string;
}

/**
 * Mark plus name, together, the way the brand is written down.
 *
 * The mark is self-contained — it brings its own deep-forest badge, so it
 * holds on the near-white canvas (14.9:1 at its darkest) and on the near-black
 * one (9.5:1 at its lightest ripple). The wordmark is Space Grotesk SemiBold
 * at `0.15em`, in the `brand` ink, which is the one green legible at text
 * sizes in both schemes: `primary` is `#00241a` in light and reads as black.
 */
export function BrandLockup({ size = 28, testID = 'brand-lockup', accessibilityLabel }: BrandLockupProps) {
  const colors = useThemeColors();

  const wordmarkStyle: TextStyle = {
    fontFamily: 'SpaceGrotesk_SemiBold',
    fontSize: size * 0.72,
    letterSpacing: size * 0.72 * BRAND_TRACKING_RATIO,
    color: colors.brand,
  };

  return (
    <View
      className="flex-row items-center"
      style={{ gap: spacing.sm }}
      accessible
      accessibilityRole="header"
      accessibilityLabel={accessibilityLabel ?? PRODUCT_WORDMARK}
      testID={testID}
    >
      <Image
        source={require('../../../assets/dravr-logo.png')}
        style={{ width: size, height: size }}
        resizeMode="contain"
        testID={`${testID}-mark`}
      />
      <Text style={wordmarkStyle} testID={`${testID}-wordmark`}>
        {PRODUCT_WORDMARK}
      </Text>
    </View>
  );
}
