// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One line of a group's roster — a leading avatar, a name line over a role line, controls trailing, 52 tall
// ABOUTME: The AI agent, the human coach and every member draw through it, so Group info reads as one list of who is who

import React from 'react';
import { View, Text, StyleSheet } from 'react-native';

export interface RosterRowProps {
  /** The leading 40pt avatar. */
  avatar: React.ReactNode;
  /** The name line. */
  name: string;
  /** Muted text after the name on the same line — an agent's `· @handle`, the viewer's `(you)`. */
  nameSuffix?: string | null;
  /** The role line under the name; a row without one is a single muted line. */
  subtitle?: string;
  /** Controls after the text column. */
  trailing?: React.ReactNode;
  /** The last row of the list draws no hairline under itself. */
  last?: boolean;
  testID: string;
}

/**
 * A roster line at `Row`'s own 52-tall shape, with the leading avatar `Row`
 * has no slot for.
 *
 * Group info mounts it inside the Members `CollapsibleSection`, which pays no
 * side inset of its own (the host `Sheet` already does), so the row pays none
 * either. The divider is the system hairline (DESIGN.md §10): `border-b`
 * alone draws a full point.
 */
export function RosterRow({ avatar, name, nameSuffix, subtitle, trailing, last = false, testID }: RosterRowProps) {
  return (
    <View
      className={['flex-row items-center min-h-[52px]', last ? '' : 'border-b border-border-faint']
        .filter(Boolean)
        .join(' ')}
      style={last ? undefined : { borderBottomWidth: StyleSheet.hairlineWidth }}
      testID={testID}
    >
      {avatar}
      <View className="flex-1 min-w-0 ml-3 py-2">
        <Text
          className={subtitle === undefined ? 'text-sm text-text-tertiary' : 'text-base text-text-primary'}
          numberOfLines={subtitle === undefined ? undefined : 1}
        >
          {name}
          {nameSuffix ? <Text className="text-text-tertiary">{` ${nameSuffix}`}</Text> : null}
        </Text>
        {subtitle !== undefined && (
          <Text className="text-sm text-text-secondary" numberOfLines={1}>
            {subtitle}
          </Text>
        )}
      </View>
      {trailing}
    </View>
  );
}
