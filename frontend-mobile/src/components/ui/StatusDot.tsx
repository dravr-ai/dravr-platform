// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The 8px state dot that sits before a status word — success, warning, error, a quiet neutral — never the only signal
// ABOUTME: One component, so every connected/pending line draws the same mark in both themes (DESIGN.md §10)

import React from 'react';
import { View } from 'react-native';

/** The state a dot marks; each is a theme token, so both schemes follow. */
/** `open` is a ring: a state still to come (a coach's "this week"), drawn apart from a filled one. */
export type StatusDotTone = 'success' | 'warning' | 'error' | 'neutral' | 'open';

const TONE_CLASS: Record<StatusDotTone, string> = {
  success: 'bg-success',
  warning: 'bg-warning',
  error: 'bg-error',
  neutral: 'bg-outline',
  open: 'border-[1.5px] border-outline',
};

/** A dot beside the words that name the state; the words carry the meaning. */
export function StatusDot({ tone, testID }: { tone: StatusDotTone; testID?: string }) {
  return (
    <View
      testID={testID}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      className={`w-2 h-2 rounded-full ${TONE_CLASS[tone]}`}
    />
  );
}
