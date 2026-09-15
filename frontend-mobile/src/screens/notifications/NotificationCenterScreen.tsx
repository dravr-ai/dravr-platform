// ABOUTME: Notification center screen with feed, category filters, and swipe actions
// ABOUTME: Shows notifications grouped by day with pull-to-refresh, swipe/long-press delete, and an ink "mark all read" header action

import React, { useCallback, useMemo, useState } from 'react';
import {
  Alert,
  RefreshControl,
  ScrollView,
  SectionList,
  Text,
  TouchableOpacity,
  View,
} from 'react-native';
import { NotificationDetailModal } from '../../components/notifications/NotificationDetailModal';
import { mobileNotificationTarget } from '@pierre/shared-constants';
import { dayLabelFor, localDayKey } from '@pierre/chat-utils';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { Stack, useRouter } from 'expo-router';
import {
  Bell,
  Dumbbell,
  Heart,
  MessageCircle,
  Trophy,
  Settings,
  Brain,
  Clock,
} from 'lucide-react-native';
import { useThemeColors } from '../../constants/theme';
import {
  useNotificationFeed,
  useNotificationActions,
  useUnreadCount,
} from '../../hooks/useNotifications';
// Relative imports for Jest/Metro compatibility
import {
  NOTIFICATION_CATEGORY_META,
  NOTIFICATION_CATEGORIES,
} from '../../../../packages/shared-constants/src/notifications';
import type { NotificationCategory, NotificationItem } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { EmptyState, SwipeableRow, type SwipeAction } from '../../components/ui';
import { NotificationRow } from './NotificationRow';
import { presentNotificationMenu } from './presentNotificationMenu';

/** Map Lucide-native icon components by category for rendering */
const CATEGORY_ICONS: Record<NotificationCategory | 'all', React.ElementType> = {
  all: Bell,
  training: Dumbbell,
  recovery: Heart,
  coach: MessageCircle,
  achievement: Trophy,
  system: Settings,
  ai: Brain,
  reminders: Clock,
};

interface NotificationSection {
  title: string;
  data: NotificationItem[];
}

/**
 * The label between two days of the feed — left-aligned, no fill, inline in
 * the list. Not a reuse of the chat thread's `DaySeparator`: that one is
 * centered and pill-shaped, a different visual shape from what the vault
 * generator draws for this screen (Boreal v2.2 Phase 5, P5.7).
 */
function NotificationDayHeader({ label }: { label: string }) {
  return (
    <View className="px-4 pt-3 pb-0.5 bg-background-primary" testID="notification-day-header">
      <Text className="text-sm font-medium text-text-secondary">{label}</Text>
    </View>
  );
}

export function NotificationCenterScreen() {
  const { t, language } = useTranslation();
  const insets = useSafeAreaInsets();
  const router = useRouter();
  const colors = useThemeColors();
  const [selectedCategory, setSelectedCategory] = useState<NotificationCategory | 'all'>('all');
  const [detailNotification, setDetailNotification] = useState<NotificationItem | null>(null);

  const feedParams = selectedCategory === 'all'
    ? { limit: 50 }
    : { limit: 50, category: selectedCategory as NotificationCategory };

  const { notifications, isLoading, isRefetching, refetch } = useNotificationFeed(feedParams);
  const { unreadCount } = useUnreadCount();
  const { markAsRead, markAllAsRead, deleteNotification, isMarkingAllRead } = useNotificationActions();

  const handleNotificationPress = useCallback((item: NotificationItem) => {
    if (!item.read_at) {
      markAsRead(item.id);
    }
    setDetailNotification(item);
  }, [markAsRead]);

  const handleDetailNavigate = useCallback((item: NotificationItem) => {
    // Resolve `data.screen` (coach messages carry the conversation id on
    // `data.id`) to a grouped expo-router target, through the same shared
    // resolver the web panel uses. The legacy `data.route` key this once read
    // was never wired server-side.
    const target = mobileNotificationTarget(item.data);
    if (target) {
      router.push(target as never);
    }
  }, [router]);

  const handleAction = useCallback((item: NotificationItem, actionId: string) => {
    if (!item.read_at) {
      markAsRead(item.id);
    }
    // Route to the screen specified in data with the action context. Coach
    // "Reply" resolves to the chat tab with the conversation preselected.
    const target = mobileNotificationTarget(item.data, actionId);
    if (target) {
      router.push(target as never);
    }
  }, [markAsRead, router]);

  /**
   * The confirm both the swipe action and the long-press menu land on —
   * mirroring Memory's `confirmForget`/`presentMemoryFactMenu` pair exactly
   * (Boreal v2.2 Phase 5, P5.8): one shared callback, so a delete triggered
   * either way asks the same question and calls the same mutation.
   */
  const confirmDelete = useCallback((item: NotificationItem) => {
    Alert.alert(t('shell.notificationDelete'), undefined, [
      { text: t('common.cancel'), style: 'cancel' },
      { text: t('common.delete'), style: 'destructive', onPress: () => deleteNotification(item.id) },
    ]);
  }, [t, deleteNotification]);

  /** Category filter list: 'all' + each category from shared constants */
  const categoryFilters = [
    { key: 'all' as const, label: t('notifPrefs.catAll') },
    ...NOTIFICATION_CATEGORIES.map((cat) => ({
      key: cat,
      label: t(NOTIFICATION_CATEGORY_META[cat].labelKey),
    })),
  ];

  /**
   * The feed grouped by local calendar day. The API returns notifications
   * newest-first and a `Map` keeps insertion order, so the grouped sections
   * read top to bottom exactly the way the flat feed did.
   */
  const sections = useMemo<NotificationSection[]>(() => {
    const byDay = new Map<string, NotificationSection>();
    for (const item of notifications) {
      const dayKey = localDayKey(item.created_at);
      let section = byDay.get(dayKey);
      if (!section) {
        const label = dayLabelFor(item.created_at, language);
        section = {
          title:
            label.kind === 'today'
              ? t('chat.dayToday')
              : label.kind === 'yesterday'
                ? t('chat.dayYesterday')
                : label.label,
          data: [],
        };
        byDay.set(dayKey, section);
      }
      section.data.push(item);
    }
    return Array.from(byDay.values());
  }, [notifications, language, t]);

  const renderItem = useCallback(({ item }: { item: NotificationItem }) => {
    const deleteAction: SwipeAction[] = [
      {
        icon: 'trash-2',
        label: t('common.delete'),
        color: colors.tokens.onError,
        backgroundColor: colors.error,
        onPress: () => confirmDelete(item),
      },
    ];
    return (
      <SwipeableRow rightActions={deleteAction} testID={`notification-row-${item.id}-swipe`}>
        <NotificationRow
          item={item}
          onPress={() => handleNotificationPress(item)}
          onLongPress={() => presentNotificationMenu({ onDelete: () => confirmDelete(item) }, t)}
        />
      </SwipeableRow>
    );
  }, [t, colors, confirmDelete, handleNotificationPress]);

  // The bug this replaces interpolated the raw category enum (`training`)
  // into the sentence instead of its translated label; both branches below
  // read the same corpus the filter tabs and the row's category word do.
  const emptyStateText = selectedCategory === 'all'
    ? `${t('app.noNotificationsYet')} ${t('app.allCaughtUp')}`
    : `${t('app.noNotificationsYet')} ${t('app.noCategoryNotifications', {
        category: t(NOTIFICATION_CATEGORY_META[selectedCategory].labelKey),
      })}`;

  return (
    <View className="flex-1 bg-background-primary" testID="notification-center-screen">
      {/* "Tout lire" replaces the old filled pill as an ink header action —
          it only exists while something is unread (Boreal v2.2 Phase 5, P5.8). */}
      <Stack.Screen
        options={{
          headerRight: () =>
            unreadCount > 0 ? (
              <TouchableOpacity
                onPress={() => markAllAsRead()}
                disabled={isMarkingAllRead}
                hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
                testID="mark-all-read"
              >
                <Text className="font-medium" style={{ fontSize: 13, color: colors.tokens.primary }}>
                  {t('app.readAll')}
                </Text>
              </TouchableOpacity>
            ) : null,
        }}
      />

      {/* Category filter tabs */}
      <View className="border-b border-border-faint">
        <ScrollView
          horizontal
          showsHorizontalScrollIndicator={false}
          contentContainerStyle={{ paddingHorizontal: 12, paddingVertical: 10, gap: 6, alignItems: 'center' }}
        >
        {categoryFilters.map(({ key, label }) => {
          const isActive = selectedCategory === key;
          const Icon = CATEGORY_ICONS[key];
          return (
            <TouchableOpacity
              key={key}
              className="flex-row items-center px-3 py-1.5 rounded-full"
              style={{
                backgroundColor: isActive ? `${colors.pierre.violet}26` : colors.background.tertiary,
                borderWidth: isActive ? 1 : 0,
                borderColor: isActive ? `${colors.pierre.violet}66` : 'transparent',
              }}
              onPress={() => setSelectedCategory(key)}
              testID={`filter-${key}`}
            >
              <Icon size={12} color={isActive ? colors.pierre.violet : colors.text.tertiary} />
              <Text
                className="text-xs ml-1"
                style={{ color: isActive ? colors.pierre.violet : colors.text.tertiary }}
              >
                {label}
              </Text>
            </TouchableOpacity>
          );
        })}
        </ScrollView>
      </View>

      {/* Notification feed, grouped by day */}
      {isLoading && !isRefetching ? (
        <View className="items-center py-12">
          <Text className="text-outline">{t('app.loadingNotifications')}</Text>
        </View>
      ) : notifications.length === 0 ? (
        <EmptyState testID="notifications-empty" className="pt-6">
          {emptyStateText}
        </EmptyState>
      ) : (
        <SectionList
          sections={sections}
          keyExtractor={(item) => item.id}
          renderItem={renderItem}
          renderSectionHeader={({ section }) => <NotificationDayHeader label={section.title} />}
          stickySectionHeadersEnabled={false}
          refreshControl={
            <RefreshControl
              refreshing={isRefetching}
              onRefresh={refetch}
              tintColor={colors.tokens.primary}
            />
          }
          contentContainerStyle={{ paddingBottom: insets.bottom + 80 }}
        />
      )}

      {/* Notification detail overlay */}
      <NotificationDetailModal
        visible={detailNotification !== null}
        notification={detailNotification}
        onClose={() => setDetailNotification(null)}
        onAction={handleAction}
        onNavigate={handleDetailNavigate}
      />
    </View>
  );
}
