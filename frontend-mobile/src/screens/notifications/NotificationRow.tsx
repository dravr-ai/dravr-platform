// ABOUTME: One notification: a leading unread dot, the hued category word and title on a baseline line, a mono time trailing, the body underneath
// ABOUTME: 56pt tall (Boreal v2.2 Phase 5, the vault generator's `screen_notifications`, `notif_row: 56`) — no badge, no delete button on the row

import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { useThemeColors } from '../../constants/theme';
// Relative import for Jest/Metro compatibility — `@pierre/shared-constants`
// has no jest moduleNameMapper entry, unlike its `@pierre/*` siblings.
import {
  formatCollapsedCount,
  formatNotificationTime,
  NOTIFICATION_CATEGORY_META,
} from '../../../../packages/shared-constants/src/notifications';
import type { NotificationItem } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';

export interface NotificationRowProps {
  item: NotificationItem;
  onPress: () => void;
  onLongPress: () => void;
}

/**
 * The row the vault generator draws: a small leading dot for unread, one
 * baseline line combining the category word (hued via
 * `NOTIFICATION_CATEGORY_META`) and the title, a trailing
 * `font-mono tabular-nums` time, then the body on its own line beneath. No
 * pill, no badge, no inline action buttons — those already live behind a tap
 * (the detail modal), so the row stays a single, scannable shape (P5.6).
 */
export function NotificationRow({ item, onPress, onLongPress }: NotificationRowProps) {
  const colors = useThemeColors();
  const { t } = useTranslation();
  const isUnread = !item.read_at;
  const meta = NOTIFICATION_CATEGORY_META[item.category];
  const collapsedLabel = formatCollapsedCount(item.collapsed_count);

  return (
    <Pressable
      className="flex-row items-center px-4 bg-background-primary"
      style={{ height: 56, gap: 10 }}
      onPress={onPress}
      onLongPress={onLongPress}
      delayLongPress={300}
      accessibilityRole="button"
      accessibilityLabel={item.title}
      testID={`notification-row-${item.id}`}
    >
      {/* The dot's own 8px footprint is reserved either way, so a read row's
          title lines up with an unread one directly above or below it. */}
      <View style={{ width: 8, height: 8, alignItems: 'center', justifyContent: 'center' }}>
        {isUnread && (
          <View
            testID="notification-unread-dot"
            style={{ width: 8, height: 8, borderRadius: 999, backgroundColor: colors.tokens.primary }}
          />
        )}
      </View>

      <View
        className="flex-1 min-w-0 h-full justify-center border-b border-border-faint"
        style={{ gap: 1 }}
      >
        <View className="flex-row items-baseline justify-between" style={{ gap: 8 }}>
          <View className="flex-row items-baseline flex-shrink" style={{ gap: 6, minWidth: 0 }}>
            <Text
              className="text-sm font-medium"
              style={{ color: meta.color }}
              testID="notification-category"
            >
              {t(meta.labelKey)}
            </Text>
            <Text
              className="flex-shrink"
              style={{
                fontSize: 16,
                lineHeight: 20,
                fontWeight: isUnread ? '600' : '400',
                color: colors.text.primary,
              }}
              numberOfLines={1}
            >
              {item.title}
            </Text>
          </View>
          <Text
            className="text-xs font-mono tabular-nums text-text-secondary"
            testID="notification-time"
          >
            {formatNotificationTime(item.created_at, t)}
          </Text>
        </View>
        <Text className="text-sm text-text-secondary" numberOfLines={1}>
          {item.body}
          {collapsedLabel ? (
            <Text className="text-xs text-text-tertiary"> {collapsedLabel}</Text>
          ) : null}
        </Text>
      </View>
    </Pressable>
  );
}
