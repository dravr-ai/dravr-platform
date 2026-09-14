// ABOUTME: An empty state is one sentence and, when there is one, one ink action — no illustration, no card, no filled button
// ABOUTME: Sits left-aligned in the content column at the interface size, where the rows would have been (DESIGN.md §5)

import React, { type ReactNode } from 'react';
import { Text, View } from 'react-native';

export interface EmptyStateAction {
  label: string;
  onPress: () => void;
  testID?: string;
}

export interface EmptyStateProps {
  /** The sentence. Say what is not here and, if it helps, why. */
  children: ReactNode;
  /** The one thing to do about it, as an ink link after the sentence. */
  action?: EmptyStateAction;
  /** Extra classes on the wrapping view, for the column's own inset. */
  className?: string;
  testID?: string;
}

/**
 * The web's `EmptyState` on React Native: the sentence and the link sit on one
 * wrapping row so they read as a single flow — "Sentence. Link" — and the link
 * is its own `Text` with its own press, the way an inline button reads on the
 * web. A pressable word, not a bar. The link is a sibling, not a nested span:
 * Android gives a nested `Text` no native view, so a test id or a tap on it
 * reaches nothing there.
 */
export function EmptyState({ children, action, className, testID }: EmptyStateProps) {
  return (
    <View className={`flex-row flex-wrap items-baseline px-4 py-3 ${className ?? ''}`} testID={testID}>
      <Text className="text-sm text-text-secondary">{children}</Text>
      {action && (
        <Text
          className="text-sm text-primary font-medium ml-1"
          onPress={action.onPress}
          accessibilityRole="button"
          testID={action.testID}
        >
          {action.label}
        </Text>
      )}
    </View>
  );
}
