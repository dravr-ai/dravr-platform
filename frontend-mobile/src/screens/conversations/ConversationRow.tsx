// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One row of the unified conversation list — avatar, kind glyph, title, preview, time and the one count capsule
// ABOUTME: Swipe right reveals Mark unread and swipe left reveals Delete; long-press hands the row to the host's menu

import React from 'react';
import { View, Text, TouchableOpacity, StyleSheet } from 'react-native';
import { Feather } from '@expo/vector-icons';
import type { ComponentProps } from 'react';
import type { ConversationKind, ConversationRowModel } from '@pierre/chat-utils';
import { MENTION_PREFIX, badgeLabel } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { useThemeColors } from '../../constants/theme';
import { InitialsAvatar } from '../../components/ui/InitialsAvatar';
import { SwipeableRow, type SwipeAction } from '../../components/ui/SwipeableRow';

type FeatherIconName = ComponentProps<typeof Feather>['name'];

/**
 * The glyph before the title of a row that is not a 1:1 thread, with the
 * label a screen reader announces in its place — the glyph is the only thing
 * on the row that says "group" or "from a messaging channel". The channel's
 * name itself lives in the thread's info sheet, not on the row.
 */
const KIND_GLYPH: Partial<Record<ConversationKind, { icon: FeatherIconName; labelKey: string }>> = {
  group: { icon: 'users', labelKey: 'app.rowKindGroup' },
  channel: { icon: 'send', labelKey: 'app.rowKindChannel' },
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

/**
 * The capsule's text: the count alone, or the `@` prefixed to it when the
 * unread line is a mention (`@ 3`). A mention is badged only while unread,
 * so the capsule always has a count to show. One capsule carries both readings.
 */
function capsuleLabel(count: number, mentioned: boolean): string {
  return mentioned ? `${MENTION_PREFIX} ${badgeLabel(count)}` : badgeLabel(count);
}

export interface ConversationRowProps {
  row: ConversationRowModel;
  onPress: (row: ConversationRowModel) => void;
  onLongPress: (row: ConversationRowModel) => void;
  onMarkUnread: (row: ConversationRowModel) => void;
  onDelete: (row: ConversationRowModel) => void;
}

/**
 * The Telegram-shaped row: a 48 pt initials avatar — a circle for an agent, a
 * square for a room — the kind glyph for a group or a channel thread, the
 * title at 600 while something is unread and 500 once read, the coach's
 * `@handle` when the title does not already name the coach, the one-line
 * preview, the relative time on the right, and one count capsule that takes
 * the `@` when that unread is a mention.
 *
 * The hairline sits on the text column, not the touchable, so the divider
 * is inset to the text and the avatars stand in an unbroken column.
 */
export function ConversationRow({ row, onPress, onLongPress, onMarkUnread, onDelete }: ConversationRowProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const unread = row.unreadCount > 0;
  const mentioned = unread && previewMentionsSomeone(row.preview);
  const glyph = KIND_GLYPH[row.kind];
  // Shape reaches no screen reader (WCAG 1.3.3), so the row's own label says
  // what kind of thread it is, in words, before the title.
  const spokenTitle = glyph ? `${t(glyph.labelKey)}, ${row.title}` : row.title;

  const leftActions: SwipeAction[] = [
    {
      icon: 'mail',
      label: t('app.rowMarkUnread'),
      color: colors.tokens.onPrimary,
      backgroundColor: colors.tokens.primary,
      onPress: () => onMarkUnread(row),
    },
  ];
  const rightActions: SwipeAction[] = [
    {
      icon: 'trash-2',
      label: t('common.delete'),
      color: colors.tokens.onError,
      backgroundColor: colors.error,
      onPress: () => onDelete(row),
    },
  ];

  return (
    <SwipeableRow leftActions={leftActions} rightActions={rightActions} testID={`swipeable-conversation-${row.id}`}>
      <TouchableOpacity
        className="flex-row items-center min-h-[72px] px-4 bg-background-primary"
        onPress={() => onPress(row)}
        onLongPress={() => onLongPress(row)}
        delayLongPress={300}
        accessibilityRole="button"
        accessibilityLabel={unread ? t('app.openRowUnread', { title: spokenTitle, count: row.unreadCount }) : t('app.openRow', { title: spokenTitle })}
        testID={`conversation-row-${row.id}`}
      >
        <InitialsAvatar
          initials={row.initials}
          slot={row.avatarSlot}
          size={48}
          shape={row.kind === 'group' ? 'square' : 'circle'}
          testID={`conversation-avatar-${row.id}`}
        />

        {/*
          The column stretches to the row's height so its bottom hairline is
          the row's divider; `border-b` alone draws a full point, and the
          system's hairline is `StyleSheet.hairlineWidth` (DESIGN.md §10).
        */}
        <View
          className="flex-1 ml-3 self-stretch justify-center border-b border-border-faint"
          style={{ borderBottomWidth: StyleSheet.hairlineWidth }}
        >
          <View className="flex-row items-center">
            {glyph && (
              <Feather
                name={glyph.icon}
                size={14}
                color={colors.text.tertiary}
                style={{ marginRight: 6 }}
                accessibilityLabel={t(glyph.labelKey)}
                testID={`conversation-kind-${row.id}`}
              />
            )}
            <Text
              className={`flex-shrink text-base text-text-primary ${unread ? 'font-semibold' : 'font-medium'}`}
              numberOfLines={1}
              testID={`conversation-title-${row.id}`}
            >
              {row.title}
            </Text>
            {row.agentHandle && (
              <Text
                className="text-sm text-text-tertiary ml-1.5 flex-shrink"
                numberOfLines={1}
                testID={`conversation-handle-${row.id}`}
              >
                {MENTION_PREFIX}{row.agentHandle}
              </Text>
            )}
            <Text
              className={`text-sm font-mono tabular-nums ml-2 ${unread ? 'text-primary' : 'text-text-tertiary'}`}
              style={{ marginLeft: 'auto' }}
              testID={`conversation-time-${row.id}`}
            >
              {row.timestamp}
            </Text>
          </View>

          <View className="flex-row items-center mt-0.5">
            <Text
              className="flex-1 text-md text-text-secondary"
              numberOfLines={1}
              testID={`conversation-preview-${row.id}`}
            >
              {row.preview}
            </Text>
            {/*
              The one mark on the row. A mention is only ever badged while
              unread, so the `@` rides the same capsule as the count rather
              than a second circle beside it; the inner text carries the
              mention id then, and the plain count id otherwise.
            */}
            {unread && (
              <View
                className="h-[22px] min-w-[22px] rounded-full px-2 items-center justify-center ml-2"
                style={{ backgroundColor: colors.tokens.primary }}
                accessibilityLabel={mentioned ? t('app.rowMentionsYou') : `${row.unreadCount} unread`}
                testID={`conversation-unread-${row.id}`}
              >
                <Text
                  className="text-sm font-semibold font-mono tabular-nums"
                  style={{ color: colors.tokens.onPrimary }}
                  testID={mentioned ? `conversation-mention-${row.id}` : `conversation-unread-text-${row.id}`}
                >
                  {capsuleLabel(row.unreadCount, mentioned)}
                </Text>
              </View>
            )}
          </View>
        </View>
      </TouchableOpacity>
    </SwipeableRow>
  );
}
