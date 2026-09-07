// ABOUTME: Reusable card component for content containers
// ABOUTME: Draws the phone's one resting-card recipe — scheme-aware fill, hairline, no shadow

import React, { type ReactNode } from 'react';
import { View, type ViewStyle } from 'react-native';
import { useCardStyle } from '../../constants/theme';

interface CardProps {
  children: ReactNode;
  /**
   * Both values name the same sheet — Boreal has one resting-card tier. See
   * the component doc below for why a second tier has no honest fill in light.
   */
  variant?: 'default' | 'elevated';
  style?: ViewStyle;
  noPadding?: boolean;
  className?: string;
}

/**
 * A content container drawn at the phone's resting-card tier.
 *
 * The fill and hairline come from `useCardStyle()` — the same recipe the
 * screens that style their own sheets consume — so a card reads identically
 * whichever path drew it. ConnectionsScreen is the seam that proves it: the
 * provider rows come through this component and the privacy note styles itself,
 * and the two sit on one scroll.
 *
 * Boreal has a single resting tier, and both `variant` values resolve to it.
 * The fill step is the only channel that lifts a surface in both schemes
 * (DESIGN.md §4, "Which channel does the lifting" — a hairline does not
 * substitute for it), and light has no tier above the `#ffffff` a card already
 * takes. A second tier would therefore be a real step in dark and nothing at
 * all in light.
 *
 * No shadow: hairlines lift, shadows float, and a card in a scroll does not
 * float. A surface that genuinely floats over the page carries the floating
 * shadow itself.
 */
export function Card({
  children,
  style,
  noPadding = false,
  className = '',
}: CardProps) {
  const cardStyle = useCardStyle();
  const paddingClasses = noPadding ? '' : 'p-4';

  const combinedClassName = [
    'rounded-xl',
    paddingClasses,
    className,
  ].filter(Boolean).join(' ');

  const combinedStyle: ViewStyle = {
    ...cardStyle,
    ...style,
  };

  return (
    <View className={combinedClassName} style={combinedStyle}>
      {children}
    </View>
  );
}
