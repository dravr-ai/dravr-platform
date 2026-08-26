// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Thin progress strip above the chat input showing what the turn is doing right now
// ABOUTME: Fed by the turn stream's own progress frames; hidden between turns to avoid visual noise

import React from 'react';
import { View, Text, ActivityIndicator } from 'react-native';

interface ChatProgressStripProps {
  /**
   * Latest status line for the in-flight turn, or `null` between turns.
   *
   * Comes from `sendTurn`'s `onProgress` callback via
   * `statusTextForProgress` — the same response body the reply arrives on, so
   * the strip cannot show progress for a turn that is not the one rendering.
   */
  statusText: string | null;
}

/**
 * Thin horizontal strip showing what the turn is working on. Hidden between
 * turns and until the first progress frame arrives, so the chat view stays
 * uncluttered for trivially-fast turns.
 *
 * Rendered between `MessageList` and `ChatInputBar` in `ChatScreen`.
 */
export function ChatProgressStrip({ statusText }: ChatProgressStripProps) {
  if (!statusText) {
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
