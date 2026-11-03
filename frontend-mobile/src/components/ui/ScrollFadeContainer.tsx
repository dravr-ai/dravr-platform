// ABOUTME: Horizontal scroll container with edge-fading gradients per Stitch UX recommendations
// ABOUTME: Indicates scrollability with gradient fades, improving discoverability of hidden content

import React, { useState, useCallback, type ReactNode } from 'react';
import {
  View,
  ScrollView,
  StyleSheet,
  type ViewStyle,
  type NativeSyntheticEvent,
  type NativeScrollEvent,
  type LayoutChangeEvent,
} from 'react-native';
import { LinearGradient } from 'expo-linear-gradient';
import { useThemeColors } from '../../constants/theme';

/**
 * The same colour at alpha 0. A gradient stop of `'transparent'` is black at
 * alpha 0, so on iOS a fade from it into a light page runs through grey; the
 * fade has to end in the page's own colour, only clear.
 */
export function clearOf(color: string): string {
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(color);
  if (hex) {
    const digits = hex[1];
    const rgb =
      digits.length === 3
        ? digits
            .split('')
            .map((digit) => digit + digit)
            .join('')
        : digits.slice(0, 6);
    return `#${rgb}00`;
  }
  const functional = /^rgba?\(\s*([^,\s]+)\s*,\s*([^,\s]+)\s*,\s*([^,\s)]+)/i.exec(color);
  if (functional) return `rgba(${functional[1]}, ${functional[2]}, ${functional[3]}, 0)`;
  return 'transparent';
}

interface ScrollFadeContainerProps {
  children: ReactNode;
  /** Background color for the gradient fade (should match container background) */
  backgroundColor?: string;
  /** Width of the fade gradient in pixels */
  fadeWidth?: number;
  /** Style for the outer container */
  containerStyle?: ViewStyle;
  /** Style for the scroll view content container */
  contentContainerStyle?: ViewStyle;
  /** Classes for the scroll view content container */
  contentContainerClassName?: string;
  /** The scroll view, for a host that scrolls it itself (a selection brought into view) */
  scrollRef?: React.Ref<ScrollView>;
  /** Told the horizontal offset on every scroll */
  onScrollX?: (x: number) => void;
  /** Test ID for the component */
  testID?: string;
}

/**
 * A horizontal scroll container with edge-fading gradients.
 *
 * Per Stitch UX recommendations, this component:
 * - Shows a gradient fade on the right edge when content extends beyond view
 * - Shows a gradient fade on the left edge when scrolled right
 * - Hides fades when at scroll boundaries
 * - Provides visual affordance that indicates "there is more content here"
 */
export function ScrollFadeContainer({
  children,
  backgroundColor,
  fadeWidth = 32,
  containerStyle,
  contentContainerStyle,
  contentContainerClassName,
  scrollRef,
  onScrollX,
  testID,
}: ScrollFadeContainerProps) {
  const colors = useThemeColors();
  const resolvedBackgroundColor = backgroundColor ?? colors.background.primary;
  const [showLeftFade, setShowLeftFade] = useState(false);
  const [showRightFade, setShowRightFade] = useState(false);
  const [contentWidth, setContentWidth] = useState(0);
  const [containerWidth, setContainerWidth] = useState(0);

  // Check if content is scrollable (content wider than container)
  const isScrollable = contentWidth > containerWidth;

  const handleScroll = useCallback(
    (event: NativeSyntheticEvent<NativeScrollEvent>) => {
      const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
      const scrollX = contentOffset.x;
      onScrollX?.(scrollX);
      const maxScrollX = contentSize.width - layoutMeasurement.width;

      // Show left fade when scrolled past threshold (not at start)
      setShowLeftFade(scrollX > 10);

      // Show right fade when not at end (with small threshold for float precision)
      setShowRightFade(scrollX < maxScrollX - 10);
    },
    [onScrollX]
  );

  const handleContentSizeChange = useCallback((width: number) => {
    setContentWidth(width);
  }, []);

  const handleContainerLayout = useCallback((event: LayoutChangeEvent) => {
    const { width } = event.nativeEvent.layout;
    setContainerWidth(width);
  }, []);

  // Initialize right fade visibility when layout is measured
  React.useEffect(() => {
    if (contentWidth > 0 && containerWidth > 0) {
      setShowRightFade(contentWidth > containerWidth);
    }
  }, [contentWidth, containerWidth]);

  // The page's colour, clear, into the page's colour: the right edge's content fades into the page.
  const clear = clearOf(resolvedBackgroundColor);
  const rightGradientColors: [string, string] = [clear, resolvedBackgroundColor];
  // The page's colour into clear: the left edge's content comes out of the page.
  const leftGradientColors: [string, string] = [resolvedBackgroundColor, clear];

  return (
    <View
      className="relative overflow-hidden"
      style={containerStyle}
      onLayout={handleContainerLayout}
      testID={testID}
    >
      <ScrollView
        ref={scrollRef}
        horizontal
        showsHorizontalScrollIndicator={false}
        onScroll={handleScroll}
        onContentSizeChange={handleContentSizeChange}
        scrollEventThrottle={16}
        contentContainerStyle={contentContainerStyle}
        contentContainerClassName={contentContainerClassName}
        testID={testID ? `${testID}-scroll` : undefined}
      >
        {children}
      </ScrollView>

      {/* Left fade gradient - shows when scrolled right */}
      {isScrollable && showLeftFade && (
        <View
          className="absolute top-0 bottom-0 left-0 z-10"
          style={{ width: fadeWidth }}
          pointerEvents="none"
          testID={testID ? `${testID}-left-fade` : undefined}
        >
          <LinearGradient
            colors={leftGradientColors}
            start={{ x: 0, y: 0 }}
            end={{ x: 1, y: 0 }}
            // A style, not a class: NativeWind does not reach a third-party
            // view, so a `className` leaves the gradient 0 wide and draws no
            // fade.
            style={StyleSheet.absoluteFill}
          />
        </View>
      )}

      {/* Right fade gradient - shows when more content available */}
      {isScrollable && showRightFade && (
        <View
          className="absolute top-0 bottom-0 right-0 z-10"
          style={{ width: fadeWidth }}
          pointerEvents="none"
          testID={testID ? `${testID}-right-fade` : undefined}
        >
          <LinearGradient
            colors={rightGradientColors}
            start={{ x: 0, y: 0 }}
            end={{ x: 1, y: 0 }}
            // A style, not a class: NativeWind does not reach a third-party
            // view, so a `className` leaves the gradient 0 wide and draws no
            // fade.
            style={StyleSheet.absoluteFill}
          />
        </View>
      )}
    </View>
  );
}
