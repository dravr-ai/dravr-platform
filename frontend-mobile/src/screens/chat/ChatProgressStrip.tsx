// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Thin progress strip above the chat input showing the AG-UI pipeline status per turn
// ABOUTME: Driven by useAgUiProgress; hidden outside an active run to avoid visual noise

import React from 'react';
import { View, Text, ActivityIndicator } from 'react-native';
import { useAgUiProgress } from '../../hooks/useAgUiProgress';

interface ChatProgressStripProps {
  /** AG-UI run id for the in-flight turn, or `null` between turns. */
  runId: string | null;
}

/**
 * Thin horizontal strip showing the current pipeline stage while a
 * send-message turn is in flight. Hidden outside an active run and
 * when no status has arrived yet so the chat view stays uncluttered
 * for trivially-fast turns.
 *
 * Rendered between `MessageList` and `ChatInputBar` in `ChatScreen`.
 */
export function ChatProgressStrip({ runId }: ChatProgressStripProps) {
  const { statusText, isActive } = useAgUiProgress(runId);

  if (!isActive || !statusText) {
    return null;
  }

  return (
    <View
      className="flex-row items-center gap-2 px-4 py-2 bg-neutral-900/5 border-t border-neutral-900/10"
      accessibilityLiveRegion="polite"
    >
      <ActivityIndicator size="small" />
      <Text
        className="flex-1 text-xs italic text-neutral-500"
        numberOfLines={1}
      >
        {statusText}
      </Text>
    </View>
  );
}
