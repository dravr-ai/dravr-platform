// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Floating expandable tab bar with glassmorphism inspired by Linear
// ABOUTME: Collapsed pill with icons + "+" button that expands to show labeled menu items

import React, { useCallback, useMemo, useState } from 'react';
import { Pressable, View } from 'react-native';
import Animated, {
  useAnimatedStyle,
  useSharedValue,
  withSpring,
  withTiming,
  withSequence,
} from 'react-native-reanimated';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import type { BottomTabBarProps } from '@react-navigation/bottom-tabs';
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

export function ExpandableTabBar({ state, descriptors, navigation }: BottomTabBarProps) {
  const insets = useSafeAreaInsets();
  const [isExpanded, setIsExpanded] = useState(false);

  const expandHeight = useSharedValue(COLLAPSED_HEIGHT);
  const expandOpacity = useSharedValue(0);
  const plusRotation = useSharedValue(0);
  const tabBarScale = useSharedValue(1);
  const activeIndicatorX = useSharedValue(0);

  const tabWidth = useMemo(() => {
    const totalPadding = 32;
    const pillWidth = 300;
    return (pillWidth - totalPadding) / state.routes.length;
  }, [state.routes.length]);

  // Compute indicator position from active index
  const updateIndicatorPosition = useCallback(
    (index: number) => {
      const targetX = 16 + index * tabWidth + tabWidth / 2 - 10;
      activeIndicatorX.value = withSpring(targetX, { damping: 20, stiffness: 200 });
    },
    [tabWidth, activeIndicatorX],
  );

  // Set initial indicator position
  React.useEffect(() => {
    const targetX = 16 + state.index * tabWidth + tabWidth / 2 - 10;
    activeIndicatorX.value = targetX;
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

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
    (routeKey: string, routeName: string, index: number) => {
      const event = navigation.emit({
        type: 'tabPress',
        target: routeKey,
        canPreventDefault: true,
      });

      if (!event.defaultPrevented) {
        const isFocused = state.index === index;
        if (!isFocused) {
          navigation.navigate(routeName);
        } else if (routeName === '(chat)') {
          // Re-tap ChatTab resets to coach selection
          navigation.navigate(routeName, { conversationId: undefined });
        }
      }

      updateIndicatorPosition(index);

      if (isExpanded) {
        setIsExpanded(false);
        expandHeight.value = withTiming(COLLAPSED_HEIGHT, { duration: 150 });
        expandOpacity.value = withTiming(0, { duration: 100 });
        plusRotation.value = withTiming(0, { duration: 200 });
      }
    },
    [
      state.index,
      navigation,
      isExpanded,
      updateIndicatorPosition,
      expandHeight,
      expandOpacity,
      plusRotation,
    ],
  );

  const quickActions: QuickAction[] = useMemo(
    () => [
      {
        icon: MessageSquarePlus,
        label: 'New Chat',
        onPress: () => {
          navigation.navigate('(chat)', { conversationId: undefined });
          updateIndicatorPosition(0);
        },
      },
      {
        icon: UserPlus,
        label: 'New Coach',
        onPress: () => {
          navigation.navigate('(coaches)', { screen: 'editor' });
          updateIndicatorPosition(1);
        },
      },
    ],
    [navigation, updateIndicatorPosition],
  );

  const handleQuickAction = useCallback(
    (action: QuickAction) => {
      action.onPress();
      setIsExpanded(false);
      expandHeight.value = withTiming(COLLAPSED_HEIGHT, { duration: 150 });
      expandOpacity.value = withTiming(0, { duration: 100 });
      plusRotation.value = withTiming(0, { duration: 200 });
    },
    [expandHeight, expandOpacity, plusRotation],
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
              {state.routes.map((route, index) => (
                <TabMenuItem
                  key={route.key}
                  icon={TAB_ICONS[index]}
                  label={TAB_LABELS[index]}
                  isActive={state.index === index}
                  delay={index * 80}
                  onPress={() => handleTabPress(route.key, route.name, index)}
                  testID={`tab-menu-item-${route.name}`}
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
                  delay={(state.routes.length + index) * 80}
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
              {state.routes.map((route, index) => {
                const isFocused = state.index === index;
                const IconComponent = TAB_ICONS[index];
                const iconColor = isFocused ? colors.pierre.violet : colors.text.tertiary;

                return (
                  <Pressable
                    key={route.key}
                    accessibilityRole="button"
                    accessibilityState={isFocused ? { selected: true } : {}}
                    accessibilityLabel={descriptors[route.key].options.title ?? route.name}
                    onPress={() => handleTabPress(route.key, route.name, index)}
                    onLongPress={() => {
                      navigation.emit({ type: 'tabLongPress', target: route.key });
                    }}
                    style={{
                      flex: 1,
                      alignItems: 'center',
                      justifyContent: 'center',
                      height: COLLAPSED_HEIGHT,
                    }}
                    testID={`tab-icon-${route.name}`}
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
