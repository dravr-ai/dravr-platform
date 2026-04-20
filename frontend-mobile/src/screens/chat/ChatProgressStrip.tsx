// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Thin progress strip above the chat input showing the AG-UI pipeline status per turn
// ABOUTME: Driven by useAgUiProgress; hidden outside an active run to avoid visual noise

import React from 'react';
import { View, Text, ActivityIndicator, StyleSheet } from 'react-native';
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
    <View style={styles.container} accessibilityLiveRegion="polite">
      <ActivityIndicator size="small" />
      <Text style={styles.text} numberOfLines={1}>
        {statusText}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 8,
    gap: 8,
    backgroundColor: 'rgba(0,0,0,0.04)',
  },
  text: {
    flex: 1,
    fontSize: 13,
    color: '#555',
    fontStyle: 'italic',
  },
});
