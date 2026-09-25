// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Dravr lockup — the badge mark beside the DRAVR wordmark, set the way DESIGN.md §1 defines it
// ABOUTME: The phone's only in-app identity: the Home and chat tabs' header title, and with `onPress` the way back to Home (DESIGN.md §5)

import React from 'react';
import { View, Text, Image, Pressable, type TextStyle } from 'react-native';
import { PRODUCT_WORDMARK } from '@pierre/shared-constants';
import { spacing, useThemeColors } from '../../constants/theme';

/** Tracking is defined as a ratio of the type size (`0.15em`); RN wants px. */
const BRAND_TRACKING_RATIO = 0.15;

/**
 * The badge's corner radius as a ratio of its size, so the square icon reads as
 * the rounded app badge at any `size`. `icon.png` is a flat square — iOS masks
 * it on the home screen and nothing masks it here.
 */
const BADGE_RADIUS_RATIO = 0.25;

/**
 * The 44 pt thumb floor (DESIGN.md §10). The lockup is 28 tall, so a pressable
 * one reaches the floor through its hit slop rather than a taller box, which
 * would push the header's own layout around.
 */
const TOUCH_TARGET = 44;

interface BrandLockupProps {
  /** Height and width of the badge mark. The wordmark is set to match. */
  size?: number;
  testID?: string;
  /**
   * Spoken name. For a heading lockup it names the region it heads; for a
   * pressable one it names where the press goes — `nav.home`.
   */
  accessibilityLabel?: string;
  /**
   * Makes the lockup a button — the way to Home from any header that carries
   * it. The look does not change: BRAND.md forbids recolouring or badging the
   * mark, so the lockup gains a press, not a new appearance.
   */
  onPress?: () => void;
}

/**
 * Mark plus name, together, the way the brand is written down.
 *
 * The mark is the boreal-ripple badge the phone already wears everywhere else
 * — the home-screen icon and the launch screen — so the header names the app
 * with the same mark the athlete tapped to open it. `icon.png` is the flattened
 * variant: the ink sits on its own `#f9f9f6` ground, which is what lets a
 * forest-ink mark hold on the near-black canvas. `splash-icon.png` is the same
 * artwork with an alpha channel and would sink into that canvas.
 *
 * The wordmark is Schibsted Grotesk 600 at `0.15em`, in the `primary` ink —
 * sage-forest in light, mint in dark, the one green legible at text sizes in
 * both schemes.
 */
export function BrandLockup({
  size = 28,
  testID = 'brand-lockup',
  accessibilityLabel,
  onPress,
}: BrandLockupProps) {
  const colors = useThemeColors();

  const wordmarkStyle: TextStyle = {
    fontFamily: 'SchibstedGrotesk',
    fontSize: size * 0.72,
    letterSpacing: size * 0.72 * BRAND_TRACKING_RATIO,
    color: colors.text.accent,
  };

  const content = (
    <>
      <Image
        source={require('../../../assets/icon.png')}
        style={{ width: size, height: size, borderRadius: size * BADGE_RADIUS_RATIO }}
        resizeMode="contain"
        testID={`${testID}-mark`}
      />
      <Text style={wordmarkStyle} testID={`${testID}-wordmark`}>
        {PRODUCT_WORDMARK}
      </Text>
    </>
  );

  if (onPress) {
    const slop = Math.max(0, (TOUCH_TARGET - size) / 2);
    return (
      <Pressable
        onPress={onPress}
        // No className: the press state reads through the style function,
        // which a class string cannot carry.
        style={({ pressed }) => ({
          flexDirection: 'row',
          alignItems: 'center',
          gap: spacing.sm,
          opacity: pressed ? 0.6 : 1,
        })}
        hitSlop={{ top: slop, bottom: slop, left: 0, right: 0 }}
        accessibilityRole="button"
        accessibilityLabel={accessibilityLabel ?? PRODUCT_WORDMARK}
        testID={testID}
      >
        {content}
      </Pressable>
    );
  }

  return (
    <View
      className="flex-row items-center"
      style={{ gap: spacing.sm }}
      accessible
      accessibilityRole="header"
      accessibilityLabel={accessibilityLabel ?? PRODUCT_WORDMARK}
      testID={testID}
    >
      {content}
    </View>
  );
}
