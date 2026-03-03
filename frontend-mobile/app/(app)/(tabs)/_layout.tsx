// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tab layout with 5 tabs (Chat, Coaches, Discover, Insights, Settings)
// ABOUTME: Uses floating expandable glass tab bar with glassmorphism effect

import React from 'react';
import { View } from 'react-native';
import { Tabs } from 'expo-router';
import { ServerStatusBanner } from '../../../src/components/ServerStatusBanner';
import { useServerStatus } from '../../../src/hooks/useServerStatus';
import { ExpandableTabBar } from '../../../src/components/ui/ExpandableTabBar';

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
        <Tabs.Screen name="(chat)" options={{ title: 'Chat' }} />
        <Tabs.Screen name="(coaches)" options={{ title: 'Coaches' }} />
        <Tabs.Screen name="(discover)" options={{ title: 'Discover' }} />
        <Tabs.Screen name="(social)" options={{ title: 'Insights' }} />
        <Tabs.Screen name="(settings)" options={{ title: 'Settings' }} />
      </Tabs>
    </View>
  );
}
