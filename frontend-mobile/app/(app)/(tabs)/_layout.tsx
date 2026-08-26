// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tab layout rendering one screen per entry of the tab bar's own list (Chat, Discover, Groups, Settings)
// ABOUTME: Uses floating expandable glass tab bar with glassmorphism effect

import React from 'react';
import { View } from 'react-native';
import { Tabs } from 'expo-router';
import { ServerStatusBanner } from '../../../src/components/ServerStatusBanner';
import { useServerStatus } from '../../../src/hooks/useServerStatus';
import { ExpandableTabBar, TAB_BAR_TABS } from '../../../src/components/ui/ExpandableTabBar';

export default function TabsLayout() {
  const { isServerReachable, isChecking, checkNow } = useServerStatus();

  return (
    <View className="flex-1">
      {!isServerReachable && (
        <ServerStatusBanner onRetry={checkNow} isChecking={isChecking} />
      )}
      <Tabs
        tabBar={() => <ExpandableTabBar />}
        screenOptions={{ headerShown: false, tabBarStyle: { display: 'none' } }}
      >
        {TAB_BAR_TABS.map((tab) => (
          <Tabs.Screen key={tab.route} name={tab.route} options={{ title: tab.label }} />
        ))}
      </Tabs>
    </View>
  );
}
