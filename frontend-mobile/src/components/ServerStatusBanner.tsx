// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Banner displayed when Pierre server is unreachable
// ABOUTME: Shows a compact red bar at the top of the app with server status

import React from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

interface ServerStatusBannerProps {
  onRetry: () => void;
  isChecking: boolean;
}

export function ServerStatusBanner({ onRetry, isChecking }: ServerStatusBannerProps) {
  const insets = useSafeAreaInsets();

  return (
    <View
      className="bg-red-900/90 px-4 py-2 flex-row items-center justify-between"
      style={{ paddingTop: insets.top + 4 }}
    >
      <View className="flex-row items-center flex-1 mr-3">
        <Feather name="wifi-off" size={14} color="#fca5a5" />
        <Text className="text-red-300 text-sm font-medium ml-2">
          Server unreachable
        </Text>
      </View>
      <TouchableOpacity
        onPress={onRetry}
        disabled={isChecking}
        className="bg-red-800 px-3 py-1 rounded-md"
        accessibilityLabel="Retry server connection"
      >
        <Text className="text-red-200 text-xs font-medium">
          {isChecking ? 'Checking...' : 'Retry'}
        </Text>
      </TouchableOpacity>
    </View>
  );
}
