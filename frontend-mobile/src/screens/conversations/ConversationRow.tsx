// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One row of the unified conversation list — avatar, kind glyph, title, preview, time, unread and mention badges
// ABOUTME: Swipe right reveals Mark unread and swipe left reveals Delete; long-press hands the row to the host's menu

import React from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import { Feather } from '@expo/vector-icons';
import type { ComponentProps } from 'react';
import type { ConversationKind, ConversationRowModel } from '@pierre/chat-utils';
import { MENTION_PREFIX } from '@pierre/shared-constants';
import { useThemeColors } from '../../constants/theme';
import { InitialsAvatar } from '../../components/ui/InitialsAvatar';
import { SwipeableRow, type SwipeAction } from '../../components/ui/SwipeableRow';

type FeatherIconName = ComponentProps<typeof Feather>['name'];

/**
 * The glyph before the title of a row that is not a 1:1 thread, with the
 * label a screen reader announces in its place — the glyph is the only thing
 * on the row that says "group" or "from a messaging channel".
 */
const KIND_GLYPH: Partial<Record<ConversationKind, { icon: FeatherIconName; label: string }>> = {
  group: { icon: 'users', label: 'Group chat' },
  channel: { icon: 'send', label: 'Messaging chat' },
};

/** A `@handle` token the way the mention grammar spells one. */
const MENTION_TOKEN = new RegExp(`(^|\\s)${MENTION_PREFIX}[A-Za-z0-9_-]+`);

/**
 * Whether the row's preview addresses someone by handle.
 *
 * The list badges an unread row differently when what is unread is a
 * mention — the reason Telegram draws the `@` — and the preview is the one
 * line of that message the list holds.
 */
export function previewMentionsSomeone(preview: string): boolean {
  return MENTION_TOKEN.test(preview);
}

/** What the badge prints for a count; three digits is where it stops growing. */
function badgeLabel(count: number): string {
  return count > 99 ? '99+' : String(count);
}

export interface ConversationRowProps {
  row: ConversationRowModel;
  onPress: (row: ConversationRowModel) => void;
  onLongPress: (row: ConversationRowModel) => void;
  onMarkUnread: (row: ConversationRowModel) => void;
  onDelete: (row: ConversationRowModel) => void;
}

/**
 * The Telegram-shaped row: a 40 pt initials avatar, the kind glyph for a
 * group or a channel thread, the title in bold while something is unread,
 * the coach's `@handle`, the one-line preview, the relative time on the
 * right, the unread count, and the `@` badge when that unread is a mention.
 */
export function ConversationRow({ row, onPress, onLongPress, onMarkUnread, onDelete }: ConversationRowProps) {
  const colors = useThemeColors();
  const unread = row.unreadCount > 0;
  const mentioned = unread && previewMentionsSomeone(row.preview);
  const glyph = KIND_GLYPH[row.kind];

  const leftActions: SwipeAction[] = [
    {
      icon: 'mail',
      label: 'Mark unread',
      color: colors.tokens.onPrimary,
      backgroundColor: colors.tokens.primary,
      onPress: () => onMarkUnread(row),
    },
  ];
  const rightActions: SwipeAction[] = [
    {
      icon: 'trash-2',
      label: 'Delete',
      color: colors.tokens.onError,
      backgroundColor: colors.error,
      onPress: () => onDelete(row),
    },
  ];

  return (
    <SwipeableRow leftActions={leftActions} rightActions={rightActions} testID={`swipeable-conversation-${row.id}`}>
      <TouchableOpacity
        className="flex-row items-center px-4 py-3 border-b border-border-subtle bg-background-primary"
        onPress={() => onPress(row)}
        onLongPress={() => onLongPress(row)}
        delayLongPress={300}
        accessibilityRole="button"
        accessibilityLabel={unread ? `Open ${row.title}, ${row.unreadCount} unread` : `Open ${row.title}`}
        testID={`conversation-row-${row.id}`}
      >
        <InitialsAvatar initials={row.initials} slot={row.avatarSlot} testID={`conversation-avatar-${row.id}`} />

        <View className="flex-1 ml-3">
          <View className="flex-row items-center">
            {glyph && (
              <Feather
                name={glyph.icon}
                size={14}
                color={colors.text.tertiary}
                style={{ marginRight: 6 }}
                accessibilityLabel={glyph.label}
                testID={`conversation-kind-${row.id}`}
              />
            )}
            <Text
              className={`flex-shrink text-base text-text-primary ${unread ? 'font-bold' : 'font-medium'}`}
              numberOfLines={1}
              testID={`conversation-title-${row.id}`}
            >
              {row.title}
            </Text>
            {row.coachHandle && (
              <Text
                className="text-xs text-text-tertiary ml-1.5 flex-shrink"
                numberOfLines={1}
                testID={`conversation-handle-${row.id}`}
              >
                {MENTION_PREFIX}{row.coachHandle}
              </Text>
            )}
            <Text
              className={`text-xs ml-2 ${unread ? 'text-primary font-semibold' : 'text-text-tertiary'}`}
              style={{ marginLeft: 'auto' }}
              testID={`conversation-time-${row.id}`}
            >
              {row.timestamp}
            </Text>
          </View>

          <View className="flex-row items-center mt-0.5">
            {row.channel && (
              <View
                className="flex-row items-center rounded-full px-1.5 py-0.5 mr-1.5"
                style={{ backgroundColor: `${colors.pierre.violet}26` }}
                accessibilityLabel={`From ${row.channel.label}`}
                testID={`conversation-channel-badge-${row.id}`}
              >
                <Text className="text-[10px] font-medium" style={{ color: colors.pierre.violet }}>
                  {row.channel.label}
                </Text>
              </View>
            )}
            <Text
              className={`flex-1 text-sm ${unread ? 'text-text-primary' : 'text-text-tertiary'}`}
              numberOfLines={1}
              testID={`conversation-preview-${row.id}`}
            >
              {row.preview}
            </Text>
            {mentioned && (
              <View
                className="w-[18px] h-[18px] rounded-full items-center justify-center ml-2"
                style={{ backgroundColor: colors.pierre.nutrition }}
                accessibilityLabel="Mentions you"
                testID={`conversation-mention-${row.id}`}
              >
                <Text className="text-[11px] font-bold" style={{ color: colors.tokens.onPrimary }}>
                  {MENTION_PREFIX}
                </Text>
              </View>
            )}
            {unread && (
              <View
                className="min-w-[18px] h-[18px] rounded-full items-center justify-center px-1 ml-2"
                style={{ backgroundColor: colors.tokens.primary }}
                accessibilityLabel={`${row.unreadCount} unread`}
                testID={`conversation-unread-${row.id}`}
              >
                <Text className="text-[10px] font-bold" style={{ color: colors.tokens.onPrimary }}>
                  {badgeLabel(row.unreadCount)}
                </Text>
              </View>
            )}
          </View>
        </View>
      </TouchableOpacity>
    </SwipeableRow>
  );
}
