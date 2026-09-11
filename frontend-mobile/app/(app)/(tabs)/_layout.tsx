// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The system tab bar — one labelled trigger per entry of the tab list (Chat, Discover, Settings)
// ABOUTME: UIKit's bar on iOS (glass on iOS 26, minimizing on scroll), Material's on Android; nothing hand-drawn

import React from 'react';
import { Platform, View } from 'react-native';
import { NativeTabs } from 'expo-router/unstable-native-tabs';
import { useTranslation } from '@pierre/i18n';
import { ServerStatusBanner } from '../../../src/components/ServerStatusBanner';
import { useServerStatus } from '../../../src/hooks/useServerStatus';
import { useThemeColors } from '../../../src/constants/theme';
import { badgeLabel, TAB_BAR_TABS } from '../../../src/navigation/tabs';
import { useConversationRows } from '../../../src/screens/conversations/useConversationList';

export default function TabsLayout() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { isServerReachable, isChecking, checkNow } = useServerStatus();
  // The chat tab wears the unread total of the same list the chat tab shows,
  // so the badge and the rows can never disagree about what is unread.
  const { unreadTotal } = useConversationRows();

  return (
    <View className="flex-1">
      {!isServerReachable && <ServerStatusBanner onRetry={checkNow} isChecking={isChecking} />}
      <NativeTabs
        tintColor={colors.tokens.primary}
        iconColor={{ default: colors.text.secondary }}
        labelStyle={{ default: { color: colors.text.secondary } }}
        // Android draws its bar opaque; iOS keeps the system material so the
        // list shows through it, and the glass on iOS 26.
        backgroundColor={Platform.OS === 'android' ? colors.background.primary : undefined}
        minimizeBehavior="onScrollDown"
      >
        {TAB_BAR_TABS.map((tab) => (
          <NativeTabs.Trigger
            key={tab.route}
            name={tab.route}
            unstable_nativeProps={{ tabBarItemTestID: tab.testID }}
          >
            <NativeTabs.Trigger.Icon sf={tab.sf} md={tab.md} />
            <NativeTabs.Trigger.Label>{t(tab.labelKey)}</NativeTabs.Trigger.Label>
            {/* Mounted only with something unread: the native badge draws a
                "0" when it is merely marked hidden. */}
            {tab.route === '(chat)' && unreadTotal > 0 && (
              <NativeTabs.Trigger.Badge>{badgeLabel(unreadTotal)}</NativeTabs.Trigger.Badge>
            )}
          </NativeTabs.Trigger>
        ))}
      </NativeTabs>
    </View>
  );
}
