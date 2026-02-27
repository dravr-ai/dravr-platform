// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tab layout with 5 tabs (Chat, Coaches, Discover, Insights, Settings)
// ABOUTME: Uses Expo Router Tabs with custom styling matching Pierre design system

import React from 'react';
import { View } from 'react-native';
import { Tabs } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { colors } from '../../../src/constants/theme';
import { ServerStatusBanner } from '../../../src/components/ServerStatusBanner';
import { useServerStatus } from '../../../src/hooks/useServerStatus';

export default function TabsLayout() {
  const { isServerReachable, isChecking, checkNow } = useServerStatus();

  return (
    <View className="flex-1">
      {!isServerReachable && (
        <ServerStatusBanner onRetry={checkNow} isChecking={isChecking} />
      )}
      <Tabs
        screenOptions={{
          headerShown: false,
          tabBarActiveTintColor: colors.pierre.violet,
          tabBarInactiveTintColor: colors.text.tertiary,
          tabBarStyle: {
            backgroundColor: colors.background.secondary,
            borderTopColor: colors.border.subtle,
          },
          tabBarLabelStyle: {
            fontSize: 10,
            fontWeight: '500',
          },
        }}
      >
        <Tabs.Screen
          name="(chat)"
          options={{
            title: 'Chat',
            tabBarIcon: ({ color, size }) => (
              <Feather name="message-circle" size={size} color={color} />
            ),

          }}
        />
        <Tabs.Screen
          name="(coaches)"
          options={{
            title: 'Coaches',
            tabBarIcon: ({ color, size }) => (
              <Feather name="award" size={size} color={color} />
            ),

          }}
        />
        <Tabs.Screen
          name="(discover)"
          options={{
            title: 'Discover',
            tabBarIcon: ({ color, size }) => (
              <Feather name="compass" size={size} color={color} />
            ),

          }}
        />
        <Tabs.Screen
          name="(social)"
          options={{
            title: 'Insights',
            tabBarIcon: ({ color, size }) => (
              <Feather name="zap" size={size} color={color} />
            ),

          }}
        />
        <Tabs.Screen
          name="(settings)"
          options={{
            title: 'Settings',
            tabBarIcon: ({ color, size }) => (
              <Feather name="settings" size={size} color={color} />
            ),

          }}
        />
      </Tabs>
    </View>
  );
}
