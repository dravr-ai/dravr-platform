// ABOUTME: Notification center screen with feed, category filters, and swipe actions
// ABOUTME: Shows grouped notifications with pull-to-refresh and mark-all-read

import React, { useCallback, useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  RefreshControl,
  ScrollView,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import {
  Bell,
  CheckCheck,
  ChevronLeft,
  Trash2,
  Dumbbell,
  Heart,
  Users,
  MessageCircle,
  Trophy,
  Settings,
  Brain,
  Clock,
} from 'lucide-react-native';
import { colors } from '../../constants/theme';
import {
  useNotificationFeed,
  useNotificationActions,
  useUnreadCount,
} from '../../hooks/useNotifications';
import type { NotificationCategory, NotificationItem, NotificationAction } from '@pierre/shared-types';

const CATEGORY_FILTERS: { key: NotificationCategory | 'all'; label: string; icon: React.ElementType }[] = [
  { key: 'all', label: 'All', icon: Bell },
  { key: 'training', label: 'Training', icon: Dumbbell },
  { key: 'recovery', label: 'Recovery', icon: Heart },
  { key: 'social', label: 'Social', icon: Users },
  { key: 'coach', label: 'Coach', icon: MessageCircle },
  { key: 'achievement', label: 'Achieve', icon: Trophy },
  { key: 'system', label: 'System', icon: Settings },
  { key: 'ai', label: 'AI', icon: Brain },
  { key: 'reminders', label: 'Remind', icon: Clock },
];

function getCategoryColor(category: NotificationCategory): string {
  const map: Record<NotificationCategory, string> = {
    training: colors.pierre.activity,
    recovery: colors.pierre.recovery,
    social: '#818CF8',
    coach: colors.primary[400],
    achievement: '#FBBF24',
    system: '#94A3B8',
    ai: colors.pierre.cyan,
    reminders: '#F472B6',
  };
  return map[category];
}

function formatRelativeTime(dateStr: string): string {
  const now = Date.now();
  const date = new Date(dateStr).getTime();
  const diffMs = now - date;
  const diffMin = Math.floor(diffMs / 60_000);
  const diffHr = Math.floor(diffMs / 3_600_000);
  const diffDay = Math.floor(diffMs / 86_400_000);

  if (diffMin < 1) return 'Just now';
  if (diffMin < 60) return `${diffMin}m ago`;
  if (diffHr < 24) return `${diffHr}h ago`;
  if (diffDay < 7) return `${diffDay}d ago`;
  return new Date(dateStr).toLocaleDateString();
}

function NotificationRow({
  item,
  onPress,
  onDelete,
}: {
  item: NotificationItem;
  onPress: (item: NotificationItem) => void;
  onDelete: (id: string) => void;
}) {
  const isUnread = !item.read_at;
  const categoryColor = getCategoryColor(item.category);

  return (
    <TouchableOpacity
      className="flex-row items-start px-4 py-3 border-b border-white/5"
      style={{ backgroundColor: isUnread ? 'rgba(139, 92, 246, 0.05)' : 'transparent' }}
      onPress={() => onPress(item)}
      activeOpacity={0.7}
      testID={`notification-${item.id}`}
    >
      {/* Unread dot */}
      <View className="w-6 items-center pt-1.5">
        {isUnread && (
          <View
            className="w-2.5 h-2.5 rounded-full"
            style={{ backgroundColor: categoryColor }}
          />
        )}
      </View>

      {/* Content */}
      <View className="flex-1 mr-2">
        <View className="flex-row items-center mb-0.5">
          <Text
            className="text-xs font-medium mr-2"
            style={{ color: categoryColor }}
          >
            {item.category.toUpperCase()}
          </Text>
          <Text className="text-xs text-zinc-500">
            {formatRelativeTime(item.created_at)}
          </Text>
        </View>
        <Text
          className="text-sm mb-0.5"
          style={{ color: isUnread ? '#F8FAFC' : '#CBD5E1', fontWeight: isUnread ? '600' : '400' }}
          numberOfLines={1}
        >
          {item.title}
        </Text>
        <Text className="text-xs text-zinc-400" numberOfLines={2}>
          {item.body}
        </Text>
        {/* Action buttons */}
        {item.actions && item.actions.length > 0 && (
          <View className="flex-row mt-2 gap-2">
            {item.actions.map((action: NotificationAction) => (
              <TouchableOpacity
                key={action.id}
                className="px-3 py-1.5 rounded-md"
                style={{ backgroundColor: 'rgba(139, 92, 246, 0.15)' }}
                onPress={() => onPress(item)}
                testID={`action-${action.id}`}
              >
                <Text className="text-xs font-medium" style={{ color: '#A78BFA' }}>
                  {action.title}
                </Text>
              </TouchableOpacity>
            ))}
          </View>
        )}
      </View>

      {/* Delete button */}
      <TouchableOpacity
        className="w-8 h-8 items-center justify-center"
        onPress={() => onDelete(item.id)}
        hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
      >
        <Trash2 size={14} color="#64748B" />
      </TouchableOpacity>
    </TouchableOpacity>
  );
}

export function NotificationCenterScreen() {
  const insets = useSafeAreaInsets();
  const router = useRouter();
  const [selectedCategory, setSelectedCategory] = useState<NotificationCategory | 'all'>('all');

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
    // Deep-link routing based on notification data
    if (item.data?.route && typeof item.data.route === 'string') {
      router.push(item.data.route as never);
    }
  }, [markAsRead, router]);

  const handleDelete = useCallback((id: string) => {
    deleteNotification(id);
  }, [deleteNotification]);

  return (
    <View className="flex-1 bg-background-primary" style={{ paddingTop: insets.top }}>
      {/* Header */}
      <View className="flex-row items-center px-4 py-3 border-b border-white/10">
        <TouchableOpacity
          className="w-10 h-10 items-center justify-center mr-2"
          onPress={() => router.back()}
          testID="notifications-back"
        >
          <ChevronLeft size={24} color="#E2E8F0" />
        </TouchableOpacity>

        <View className="flex-1 flex-row items-center">
          <Bell size={20} color={colors.primary[400]} />
          <Text className="text-lg font-semibold text-white ml-2">Notifications</Text>
          {unreadCount > 0 && (
            <View className="ml-2 px-2 py-0.5 rounded-full bg-violet-600">
              <Text className="text-xs font-bold text-white">{unreadCount}</Text>
            </View>
          )}
        </View>

        {unreadCount > 0 && (
          <TouchableOpacity
            className="flex-row items-center px-3 py-1.5 rounded-lg bg-white/5"
            onPress={() => markAllAsRead()}
            disabled={isMarkingAllRead}
            testID="mark-all-read"
          >
            <CheckCheck size={14} color={colors.primary[400]} />
            <Text className="text-xs text-primary-400 ml-1">Read all</Text>
          </TouchableOpacity>
        )}
      </View>

      {/* Category filter tabs */}
      <ScrollView
        horizontal
        showsHorizontalScrollIndicator={false}
        className="border-b border-white/5"
        contentContainerStyle={{ paddingHorizontal: 12, paddingVertical: 8, gap: 6 }}
      >
        {CATEGORY_FILTERS.map(({ key, label, icon: Icon }) => {
          const isActive = selectedCategory === key;
          return (
            <TouchableOpacity
              key={key}
              className="flex-row items-center px-3 py-1.5 rounded-full"
              style={{
                backgroundColor: isActive ? 'rgba(139, 92, 246, 0.2)' : 'rgba(255, 255, 255, 0.05)',
                borderWidth: isActive ? 1 : 0,
                borderColor: isActive ? 'rgba(139, 92, 246, 0.4)' : 'transparent',
              }}
              onPress={() => setSelectedCategory(key)}
              testID={`filter-${key}`}
            >
              <Icon size={12} color={isActive ? '#A78BFA' : '#94A3B8'} />
              <Text
                className="text-xs ml-1"
                style={{ color: isActive ? '#A78BFA' : '#94A3B8' }}
              >
                {label}
              </Text>
            </TouchableOpacity>
          );
        })}
      </ScrollView>

      {/* Notification list */}
      <ScrollView
        className="flex-1"
        refreshControl={
          <RefreshControl
            refreshing={isRefetching}
            onRefresh={refetch}
            tintColor={colors.primary[400]}
          />
        }
      >
        {isLoading && !isRefetching ? (
          <View className="items-center py-12">
            <Text className="text-zinc-500">Loading notifications...</Text>
          </View>
        ) : notifications.length === 0 ? (
          <View className="items-center py-16">
            <Bell size={48} color="#475569" />
            <Text className="text-zinc-400 mt-4 text-base">No notifications yet</Text>
            <Text className="text-zinc-500 mt-1 text-sm">
              {selectedCategory === 'all'
                ? "You're all caught up!"
                : `No ${selectedCategory} notifications`}
            </Text>
          </View>
        ) : (
          notifications.map((item) => (
            <NotificationRow
              key={item.id}
              item={item}
              onPress={handleNotificationPress}
              onDelete={handleDelete}
            />
          ))
        )}

        {/* Bottom padding */}
        <View style={{ height: insets.bottom + 80 }} />
      </ScrollView>
    </View>
  );
}
