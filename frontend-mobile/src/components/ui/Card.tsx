// ABOUTME: Reusable card component for content containers
// ABOUTME: Supports elevated and flat variants with consistent padding

import React, { type ReactNode } from 'react';
import { View, type ViewStyle } from 'react-native';
import { useThemeColors } from '../../constants/theme';

interface CardProps {
  children: ReactNode;
  variant?: 'default' | 'elevated';
  style?: ViewStyle;
  noPadding?: boolean;
  className?: string;
}

export function Card({
  children,
  variant = 'default',
  style,
  noPadding = false,
  className = '',
}: CardProps) {
  const colors = useThemeColors();
  const baseClasses = 'bg-background-secondary rounded-xl border border-border-subtle';
  const elevatedClasses = variant === 'elevated' ? 'bg-background-elevated' : '';
  const paddingClasses = noPadding ? '' : 'p-4';

  const combinedClassName = [
    baseClasses,
    elevatedClasses,
    paddingClasses,
    className,
  ].filter(Boolean).join(' ');

  // Shadow styles for elevated variant (shadows need style objects in RN)
  const elevatedShadowStyle: ViewStyle = {
    shadowColor: colors.text.primary,
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.25,
    shadowRadius: 4,
    elevation: 4,
  };

  // Add shadow styles for elevated variant
  const combinedStyle: ViewStyle = {
    ...(variant === 'elevated' ? elevatedShadowStyle : {}),
    ...style,
  };

  return (
    <View className={combinedClassName} style={combinedStyle}>
      {children}
    </View>
  );
}
