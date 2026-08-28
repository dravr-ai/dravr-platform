// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Floating expandable tab bar with glassmorphism inspired by Linear
// ABOUTME: Collapsed pill with Chat/Discover/Settings + "+" that expands into the tabs and the chat quick actions

import React, { useCallback, useMemo, useState } from 'react';
import { Pressable, Text, useWindowDimensions, View } from 'react-native';
import Animated, {
  useAnimatedStyle,
  useSharedValue,
  withSpring,
  withTiming,
  withSequence,
} from 'react-native-reanimated';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { useGlobalSearchParams, useRouter, useSegments } from 'expo-router';
import * as Haptics from 'expo-haptics';
import { MessageCircle, Compass, Settings, Plus } from 'lucide-react-native';
import type { LucideIcon } from 'lucide-react-native';
import { useThemeColors } from '../../constants/theme';
import { CHAT_LIST_ROUTE, NEW_CONVERSATION_ID } from '../../navigation/routes';
import { useChatPlusActions, type ChatPlusAction } from '../../screens/chat/useChatPlusActions';
import { ChatPlusFlows } from '../../screens/chat/ChatPlusFlows';
import { useConversationRows } from '../../screens/conversations/useConversationList';
import { GlassContainer } from './GlassContainer';
import { TabMenuItem } from './TabMenuItem';
import { useTranslation } from '@pierre/i18n';

/** The route group a tab opens. */
export type TabBarRoute = '(chat)' | '(discover)' | '(settings)';

/** One tab of the bar: the route group it opens, its label, icon and test id. */
export interface TabBarTab {
  route: TabBarRoute;
  /** Corpus key; module scope cannot hold a hook, so the bar resolves it. */
  labelKey: string;
  icon: LucideIcon;
  testID: string;
  /** A count drawn over the icon; the chat tab carries the unread total. */
  badge?: number;
}

/** What the badge prints for a count; three digits is where it stops growing. */
function badgeLabel(count: number): string {
  return count > 99 ? '99+' : String(count);
}

/**
 * The tab set, in order. Chat is first because it is where the app lands.
 *
 * This is the only copy. The tabs layout renders one `Tabs.Screen` per entry
 * here, so the bar and the router cannot list different tabs — the structure
 * that let a tab be filtered from one list and not the other is gone.
 */
export const TAB_BAR_TABS: readonly TabBarTab[] = [
  { route: '(chat)', labelKey: 'app.navChat', icon: MessageCircle, testID: 'tab-chat' },
  { route: '(discover)', labelKey: 'app.discover', icon: Compass, testID: 'tab-discover' },
  { route: '(settings)', labelKey: 'common.settings', icon: Settings, testID: 'tab-settings' },
];
const TAB_COUNT = TAB_BAR_TABS.length;

const ICON_SIZE = 22;
const COLLAPSED_HEIGHT = 56;
const EXPANDED_HEIGHT = 380;
const PLUS_BUTTON_SIZE = 48;

/**
 * Space screens reserve at the bottom so nothing sits under the floating tab bar.
 *
 * The `+ 40` is a hardcoded stand-in for the home indicator, and it is wrong on
 * most hardware: an iPhone SE has a bottom inset of 0 and gets 40dp of dead
 * space, Android gesture navigation reports its own value, and a tablet
 * reports another. `tabBarBottomOffset(insets.bottom)` is the honest version
 * and is what every screen should call.
 *
 * The constant is kept because it is the correct value for an inset of 0 plus
 * the bar's own breathing room, and it is what a component without access to
 * the safe-area context falls back to.
 */
export const TAB_BAR_GAP = 12;
export const TAB_BAR_BOTTOM_OFFSET = COLLAPSED_HEIGHT + TAB_BAR_GAP;

/** The bottom space to reserve, given the device's real safe-area inset. */
export function tabBarBottomOffset(bottomInset: number): number {
  return COLLAPSED_HEIGHT + TAB_BAR_GAP + bottomInset;
}

const AnimatedPressable = Animated.createAnimatedComponent(Pressable);

export function ExpandableTabBar() {
  const { t } = useTranslation();
  const insets = useSafeAreaInsets();
  const segments = useSegments();
  const globalParams = useGlobalSearchParams<{ conversationId?: string }>();
  const router = useRouter();
  const colors = useThemeColors();
  const [isExpanded, setIsExpanded] = useState(false);

  const { width: screenWidth } = useWindowDimensions();
  const expandHeight = useSharedValue(COLLAPSED_HEIGHT);
  const expandOpacity = useSharedValue(0);
  const plusRotation = useSharedValue(0);
  const tabBarScale = useSharedValue(1);
  const activeIndicatorX = useSharedValue(0);

  // useSegments() returns route segments including group names like '(chat)'
  const activeIndex = useMemo(() => {
    const idx = TAB_BAR_TABS.findIndex((tab) => segments.includes(tab.route));
    return Math.max(0, idx);
  }, [segments]);

  // The thread the athlete is reading, when the focused route is one. The
  // "add someone to this discussion" action exists only then — a fresh
  // composer has no conversation for the participants routes to act on.
  const openConversationId = useMemo(() => {
    const inThread = segments.includes('(chat)') && segments.includes('[conversationId]');
    const id = globalParams.conversationId;
    return inThread && typeof id === 'string' && id !== NEW_CONVERSATION_ID ? id : null;
  }, [segments, globalParams.conversationId]);

  const chatPlus = useChatPlusActions(openConversationId);

  // The chat tab wears the unread total of the same list the chat tab shows,
  // so the badge and the rows can never disagree about what is unread.
  const { unreadTotal } = useConversationRows();
  const tabs = useMemo<TabBarTab[]>(
    () => TAB_BAR_TABS.map((tab) => (tab.route === '(chat)' ? { ...tab, badge: unreadTotal } : tab)),
    [unreadTotal],
  );

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
      const tab = TAB_BAR_TABS[index];
      if (activeIndex !== index) {
        router.navigate(`/(app)/(tabs)/${tab.route}`);
      } else if (tab.route === '(chat)') {
        // Re-tapping Chat from inside a thread pops back to the conversation list.
        router.navigate(CHAT_LIST_ROUTE);
      }

      updateIndicatorPosition(index);
      collapseIfExpanded();
    },
    [activeIndex, router, updateIndicatorPosition, collapseIfExpanded],
  );

  const handleQuickAction = useCallback(
    (action: ChatPlusAction) => {
      collapseIfExpanded();
      action.onPress();
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
              {TAB_BAR_TABS.map((tab, index) => (
                <TabMenuItem
                  key={tab.route}
                  icon={tab.icon}
                  label={t(tab.labelKey)}
                  isActive={activeIndex === index}
                  delay={index * 80}
                  onPress={() => handleTabPress(index)}
                  testID={`tab-menu-item-${tab.route}`}
                />
              ))}

              {/* Separator */}
              <View
                style={{
                  height: 1,
                  backgroundColor: colors.border.default,
                  marginHorizontal: 16,
                  marginVertical: 8,
                }}
              />

              {/* Chat quick actions — the same set the chat screens' "+" offers */}
              {chatPlus.actions.map((action, index) => (
                <TabMenuItem
                  key={action.id}
                  icon={action.icon}
                  label={action.label}
                  isActive={false}
                  isQuickAction
                  delay={(TAB_COUNT + index) * 80}
                  onPress={() => handleQuickAction(action)}
                  testID={`quick-action-${action.id}`}
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
              {tabs.map((tab, index) => {
                const isFocused = activeIndex === index;
                const IconComponent = tab.icon;
                const iconColor = isFocused ? colors.pierre.violet : colors.text.secondary;
                const badge = tab.badge ?? 0;

                return (
                  <Pressable
                    key={tab.route}
                    accessibilityRole="button"
                    accessibilityState={isFocused ? { selected: true } : {}}
                    accessibilityLabel={
                      badge > 0
                        ? t('app.tabWithUnread', { tab: t(tab.labelKey), count: badge })
                        : t(tab.labelKey)
                    }
                    onPress={() => handleTabPress(index)}
                    style={{
                      flex: 1,
                      alignItems: 'center',
                      justifyContent: 'center',
                      height: COLLAPSED_HEIGHT,
                    }}
                    testID={tab.testID}
                  >
                    <View>
                      <IconComponent size={ICON_SIZE} color={iconColor} />
                      {badge > 0 && (
                        <View
                          style={{
                            position: 'absolute',
                            top: -6,
                            right: -12,
                            minWidth: 18,
                            height: 18,
                            borderRadius: 9,
                            paddingHorizontal: 4,
                            alignItems: 'center',
                            justifyContent: 'center',
                            backgroundColor: colors.pierre.violet,
                          }}
                          testID={`${tab.testID}-badge`}
                        >
                          <Text style={{ fontSize: 10, fontWeight: '700', color: colors.tokens.onPrimary }}>
                            {badgeLabel(badge)}
                          </Text>
                        </View>
                      )}
                    </View>
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
          accessibilityLabel={isExpanded ? t('app.closeMenu') : t('app.openMenu')}
        >
          <Animated.View style={plusIconStyle}>
            <Plus size={24} color={colors.pierre.violet} />
          </Animated.View>
        </AnimatedPressable>
      </GlassContainer>

      <ChatPlusFlows flows={chatPlus.flows} />
    </View>
  );
}
