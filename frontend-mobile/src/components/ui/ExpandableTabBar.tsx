// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Floating expandable tab bar with glassmorphism inspired by Linear
// ABOUTME: Collapsed pill with icons + "+" button that expands to show labeled menu items

import React, { useCallback, useMemo, useState } from 'react';
import { Pressable, useWindowDimensions, View } from 'react-native';
import Animated, {
  useAnimatedStyle,
  useSharedValue,
  withSpring,
  withTiming,
  withSequence,
} from 'react-native-reanimated';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { useRouter, useSegments } from 'expo-router';
import * as Haptics from 'expo-haptics';
import {
  MessageCircle,
  Award,
  Compass,
  Zap,
  Settings,
  Plus,
  MessageSquarePlus,
  UserPlus,
} from 'lucide-react-native';
import type { LucideIcon } from 'lucide-react-native';
import { colors } from '../../constants/theme';
import { GlassContainer } from './GlassContainer';
import { TabMenuItem } from './TabMenuItem';

const TAB_ICONS: LucideIcon[] = [MessageCircle, Award, Compass, Zap, Settings];
const TAB_LABELS = ['Chat', 'Coaches', 'Discover', 'Insights', 'Settings'];
const TAB_ROUTES = ['(chat)', '(coaches)', '(discover)', '(social)', '(settings)'] as const;
const TAB_TEST_IDS = ['tab-chat', 'tab-coaches', 'tab-discover', 'tab-insights', 'tab-settings'];
const TAB_COUNT = TAB_ROUTES.length;

const ICON_SIZE = 22;
const COLLAPSED_HEIGHT = 56;
const EXPANDED_HEIGHT = 380;
const PLUS_BUTTON_SIZE = 48;

/** Space screens should reserve at bottom to avoid overlapping the floating tab bar */
export const TAB_BAR_BOTTOM_OFFSET = COLLAPSED_HEIGHT + 40;

const AnimatedPressable = Animated.createAnimatedComponent(Pressable);

interface QuickAction {
  icon: LucideIcon;
  label: string;
  onPress: () => void;
}

export function ExpandableTabBar() {
  const insets = useSafeAreaInsets();
  const segments = useSegments();
  const router = useRouter();
  const [isExpanded, setIsExpanded] = useState(false);

  const { width: screenWidth } = useWindowDimensions();
  const expandHeight = useSharedValue(COLLAPSED_HEIGHT);
  const expandOpacity = useSharedValue(0);
  const plusRotation = useSharedValue(0);
  const tabBarScale = useSharedValue(1);
  const activeIndicatorX = useSharedValue(0);

  // useSegments() returns route segments including group names like '(coaches)'
  const activeIndex = useMemo(() => {
    const idx = TAB_ROUTES.findIndex((route) => segments.includes(route));
    return Math.max(0, idx);
  }, [segments]);

  // Pill width = screen - outer padding (32) - gap (10) - plus button (48)
  const pillWidth = screenWidth - 32 - 10 - PLUS_BUTTON_SIZE;

  const tabWidth = useMemo(() => {
    const iconRowPadding = 32;
    return (pillWidth - iconRowPadding) / TAB_COUNT;
  }, [pillWidth]);

  // Compute indicator position from active index
  const updateIndicatorPosition = useCallback(
    (index: number) => {
      const targetX = 16 + index * tabWidth + tabWidth / 2 - 10;
      activeIndicatorX.value = withSpring(targetX, { damping: 20, stiffness: 200 });
    },
    [tabWidth, activeIndicatorX],
  );

  // Update indicator when active tab changes
  React.useEffect(() => {
    updateIndicatorPosition(activeIndex);
  }, [activeIndex, updateIndicatorPosition]);

  const collapseIfExpanded = useCallback(() => {
    if (!isExpanded) return;
    setIsExpanded(false);
    expandHeight.value = withTiming(COLLAPSED_HEIGHT, { duration: 150 });
    expandOpacity.value = withTiming(0, { duration: 100 });
    plusRotation.value = withTiming(0, { duration: 200 });
  }, [isExpanded, expandHeight, expandOpacity, plusRotation]);

  const toggleExpand = useCallback(() => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);

    const expanding = !isExpanded;
    setIsExpanded(expanding);

    if (expanding) {
      expandHeight.value = withTiming(EXPANDED_HEIGHT, { duration: 150 });
      expandOpacity.value = withSpring(1);
      plusRotation.value = withTiming(45, { duration: 200 });
    } else {
      expandHeight.value = withTiming(COLLAPSED_HEIGHT, { duration: 150 });
      expandOpacity.value = withTiming(0, { duration: 100 });
      plusRotation.value = withTiming(0, { duration: 200 });
    }

    tabBarScale.value = withSequence(
      withTiming(0.95, { duration: 75 }),
      withSpring(1, { damping: 15, stiffness: 200 }),
    );
  }, [isExpanded, expandHeight, expandOpacity, plusRotation, tabBarScale]);

  const handleTabPress = useCallback(
    (index: number) => {
      if (activeIndex !== index) {
        router.navigate(`/(app)/(tabs)/${TAB_ROUTES[index]}`);
      } else if (TAB_ROUTES[index] === '(chat)') {
        // Re-tap ChatTab resets to coach selection
        router.navigate({ pathname: '/(app)/(tabs)/(chat)', params: { conversationId: undefined } });
      }

      updateIndicatorPosition(index);
      collapseIfExpanded();
    },
    [activeIndex, router, updateIndicatorPosition, collapseIfExpanded],
  );

  const quickActions: QuickAction[] = useMemo(
    () => [
      {
        icon: MessageSquarePlus,
        label: 'New Chat',
        onPress: () => {
          router.navigate({ pathname: '/(app)/(tabs)/(chat)', params: { conversationId: undefined } });
          updateIndicatorPosition(0);
        },
      },
      {
        icon: UserPlus,
        label: 'New Coach',
        onPress: () => {
          router.navigate('/(app)/(tabs)/(coaches)/editor');
          updateIndicatorPosition(1);
        },
      },
    ],
    [router, updateIndicatorPosition],
  );

  const handleQuickAction = useCallback(
    (action: QuickAction) => {
      action.onPress();
      collapseIfExpanded();
    },
    [collapseIfExpanded],
  );

  // Animated styles
  const pillAnimatedStyle = useAnimatedStyle(() => ({
    height: expandHeight.value,
    transform: [{ scale: tabBarScale.value }],
  }));

  const expandedContentStyle = useAnimatedStyle(() => ({
    opacity: expandOpacity.value,
    display: expandOpacity.value > 0.01 ? 'flex' : 'none',
  }));

  const collapsedIconsStyle = useAnimatedStyle(() => ({
    opacity: expandOpacity.value > 0.5 ? 0 : 1,
  }));

  const indicatorStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: activeIndicatorX.value }],
  }));

  const plusIconStyle = useAnimatedStyle(() => ({
    transform: [{ rotate: `${plusRotation.value}deg` }],
  }));

  const bottomOffset = Math.max(insets.bottom, 12);

  return (
    <View
      style={{
        position: 'absolute',
        bottom: bottomOffset,
        left: 0,
        right: 0,
        flexDirection: 'row',
        alignItems: 'flex-end',
        justifyContent: 'center',
        paddingHorizontal: 16,
        gap: 10,
      }}
      pointerEvents="box-none"
    >
      {/* Main pill */}
      <Animated.View style={[{ flex: 1 }, pillAnimatedStyle]}>
        <GlassContainer
          style={{ flex: 1 }}
          borderRadius={28}
          testID="expandable-tab-bar"
        >
          <View style={{ flex: 1, justifyContent: 'space-between' }}>
            {/* Expanded menu items */}
            <Animated.View style={[{ paddingTop: 12, paddingHorizontal: 8 }, expandedContentStyle]}>
              {TAB_ROUTES.map((route, index) => (
                <TabMenuItem
                  key={route}
                  icon={TAB_ICONS[index]}
                  label={TAB_LABELS[index]}
                  isActive={activeIndex === index}
                  delay={index * 80}
                  onPress={() => handleTabPress(index)}
                  testID={`tab-menu-item-${route}`}
                />
              ))}

              {/* Separator */}
              <View
                style={{
                  height: 1,
                  backgroundColor: 'rgba(255, 255, 255, 0.1)',
                  marginHorizontal: 16,
                  marginVertical: 8,
                }}
              />

              {/* Quick actions */}
              {quickActions.map((action, index) => (
                <TabMenuItem
                  key={action.label}
                  icon={action.icon}
                  label={action.label}
                  isActive={false}
                  isQuickAction
                  delay={(TAB_COUNT + index) * 80}
                  onPress={() => handleQuickAction(action)}
                  testID={`quick-action-${action.label.toLowerCase().replace(' ', '-')}`}
                />
              ))}
            </Animated.View>

            {/* Collapsed icon row */}
            <Animated.View
              style={[
                {
                  flexDirection: 'row',
                  alignItems: 'center',
                  justifyContent: 'space-around',
                  height: COLLAPSED_HEIGHT,
                  paddingHorizontal: 16,
                },
                collapsedIconsStyle,
              ]}
            >
              {TAB_ROUTES.map((route, index) => {
                const isFocused = activeIndex === index;
                const IconComponent = TAB_ICONS[index];
                const iconColor = isFocused ? colors.pierre.violet : colors.text.tertiary;

                return (
                  <Pressable
                    key={route}
                    accessibilityRole="button"
                    accessibilityState={isFocused ? { selected: true } : {}}
                    accessibilityLabel={TAB_LABELS[index]}
                    onPress={() => handleTabPress(index)}
                    style={{
                      flex: 1,
                      alignItems: 'center',
                      justifyContent: 'center',
                      height: COLLAPSED_HEIGHT,
                    }}
                    testID={TAB_TEST_IDS[index]}
                  >
                    <IconComponent size={ICON_SIZE} color={iconColor} />
                  </Pressable>
                );
              })}

              {/* Sliding active indicator */}
              <Animated.View
                style={[
                  {
                    position: 'absolute',
                    left: 0,
                    bottom: 8,
                    width: 20,
                    height: 3,
                    borderRadius: 1.5,
                    backgroundColor: colors.pierre.violet,
                  },
                  indicatorStyle,
                ]}
              />
            </Animated.View>
          </View>
        </GlassContainer>
      </Animated.View>

      {/* "+" / "x" floating button */}
      <GlassContainer
        style={{
          width: PLUS_BUTTON_SIZE,
          height: PLUS_BUTTON_SIZE,
          marginBottom: (COLLAPSED_HEIGHT - PLUS_BUTTON_SIZE) / 2,
        }}
        borderRadius={PLUS_BUTTON_SIZE / 2}
        testID="expandable-tab-bar-plus"
      >
        <AnimatedPressable
          onPress={toggleExpand}
          style={{
            flex: 1,
            alignItems: 'center',
            justifyContent: 'center',
          }}
          accessibilityRole="button"
          accessibilityLabel={isExpanded ? 'Close menu' : 'Open menu'}
        >
          <Animated.View style={plusIconStyle}>
            <Plus size={24} color={colors.pierre.violet} />
          </Animated.View>
        </AnimatedPressable>
      </GlassContainer>
    </View>
  );
}
