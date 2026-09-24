// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The 8px state dot that sits before a status word — success, warning — never the only signal
// ABOUTME: One component, so every connected/pending line draws the same mark in both themes (DESIGN.md §10)

import React from 'react';
import { View } from 'react-native';

/** The state a dot marks; each is a theme token, so both schemes follow. */
export type StatusDotTone = 'success' | 'warning';

const TONE_CLASS: Record<StatusDotTone, string> = {
  success: 'bg-success',
  warning: 'bg-warning',
};

/** A dot beside the words that name the state; the words carry the meaning. */
export function StatusDot({ tone }: { tone: StatusDotTone }) {
  return <View className={`w-2 h-2 rounded-full ${TONE_CLASS[tone]}`} />;
}
