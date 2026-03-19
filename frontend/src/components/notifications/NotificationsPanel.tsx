// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Full-page notification center panel with category filters and actions
// ABOUTME: Renders as a tab in the Dashboard with feed, mark-all-read, and pagination

import { useState, useCallback } from 'react';
import { clsx } from 'clsx';
import {
  Bell,
  CheckCheck,
  Trash2,
  Dumbbell,
  Heart,
  Users,
  MessageCircle,
  Trophy,
  Settings,
  Brain,
  Clock,
} from 'lucide-react';
import {
  useNotificationFeed,
  useNotificationActions,
} from '../../hooks/useNotifications';
import {
  NOTIFICATION_CATEGORY_META,
  NOTIFICATION_CATEGORIES,
  formatNotificationTime,
  formatCollapsedCount,
} from '@pierre/shared-constants';
import type { NotificationCategory, NotificationItem, NotificationAction } from '@pierre/shared-types';

/** Map Lucide icon components by category for rendering */
const CATEGORY_ICONS: Record<NotificationCategory | 'all', React.ElementType> = {
  all: Bell,
  training: Dumbbell,
  recovery: Heart,
  social: Users,
  coach: MessageCircle,
  achievement: Trophy,
  system: Settings,
  ai: Brain,
  reminders: Clock,
};

interface NotificationsPanelProps {
  /** Callback when a notification with route data is clicked */
  onNavigate?: (route: string) => void;
}

export default function NotificationsPanel({ onNavigate }: NotificationsPanelProps) {
  const [selectedCategory, setSelectedCategory] = useState<NotificationCategory | 'all'>('all');

  const feedParams = selectedCategory === 'all'
    ? { limit: 50 }
    : { limit: 50, category: selectedCategory as NotificationCategory };

  const { notifications, total, unreadCount, isLoading } = useNotificationFeed(feedParams);
  const { markAsRead, markAllAsRead, deleteNotification, isMarkingAllRead } = useNotificationActions();

  const handleNotificationClick = useCallback(
    (item: NotificationItem) => {
      if (!item.read_at) {
        markAsRead(item.id);
      }
      if (item.data?.route && typeof item.data.route === 'string' && onNavigate) {
        onNavigate(item.data.route);
      }
    },
    [markAsRead, onNavigate],
  );

  const handleActionClick = useCallback(
    (item: NotificationItem, action: NotificationAction) => {
      if (!item.read_at) {
        markAsRead(item.id);
      }
      const data = item.data as Record<string, string> | undefined;
      const screen = data?.screen ?? action.id;
      if (onNavigate) {
        onNavigate(screen);
      }
    },
    [markAsRead, onNavigate],
  );

  /** Category filter list: 'all' + each category from shared constants */
  const categoryFilters = [
    { key: 'all' as const, label: 'All' },
    ...NOTIFICATION_CATEGORIES.map((cat) => ({
      key: cat,
      label: NOTIFICATION_CATEGORY_META[cat].label,
    })),
  ];

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between px-6 py-4 border-b border-white/10">
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-xl bg-gradient-to-br from-violet-500 to-cyan-500 flex items-center justify-center">
            <Bell className="w-5 h-5 text-white" />
          </div>
          <div>
            <h1 className="text-lg font-semibold text-white">Notifications</h1>
            <p className="text-xs text-zinc-400">
              {unreadCount > 0 ? `${unreadCount} unread` : 'All caught up'}
              {total > 0 && ` · ${total} total`}
            </p>
          </div>
        </div>

        {unreadCount > 0 && (
          <button
            onClick={() => markAllAsRead()}
            disabled={isMarkingAllRead}
            className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium text-violet-400 bg-violet-500/10 hover:bg-violet-500/20 rounded-lg transition-colors disabled:opacity-50"
          >
            <CheckCheck className="w-3.5 h-3.5" />
            Mark all read
          </button>
        )}
      </div>

      {/* Category filters */}
      <div className="flex items-center gap-2 px-6 py-3 border-b border-white/5 overflow-x-auto">
        {categoryFilters.map(({ key, label }) => {
          const isActive = selectedCategory === key;
          const Icon = CATEGORY_ICONS[key];
          return (
            <button
              key={key}
              onClick={() => setSelectedCategory(key)}
              className={clsx(
                'flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium whitespace-nowrap transition-colors',
                isActive
                  ? 'bg-violet-500/20 text-violet-300 border border-violet-500/30'
                  : 'bg-white/5 text-zinc-400 hover:bg-white/10 hover:text-zinc-300 border border-transparent',
              )}
            >
              <Icon className="w-3 h-3" />
              {label}
            </button>
          );
        })}
      </div>

      {/* Notification list */}
      <div className="flex-1 overflow-y-auto">
        {isLoading ? (
          <div className="flex items-center justify-center py-16">
            <div className="pierre-spinner" />
          </div>
        ) : notifications.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-20 text-zinc-500">
            <Bell className="w-12 h-12 mb-3 opacity-30" />
            <p className="text-base font-medium text-zinc-400">No notifications</p>
            <p className="text-sm mt-1">
              {selectedCategory === 'all'
                ? "You're all caught up!"
                : `No ${selectedCategory} notifications`}
            </p>
          </div>
        ) : (
          <div className="divide-y divide-white/5">
            {notifications.map((item) => {
              const isUnread = !item.read_at;
              const meta = NOTIFICATION_CATEGORY_META[item.category];
              const collapsedLabel = formatCollapsedCount(item.collapsed_count);

              return (
                <div
                  key={item.id}
                  className={clsx(
                    'flex items-start gap-4 px-6 py-4 cursor-pointer hover:bg-white/[0.02] transition-colors group',
                    isUnread && 'bg-violet-500/[0.03]',
                  )}
                  onClick={() => handleNotificationClick(item)}
                >
                  {/* Unread indicator */}
                  <div className="w-2 pt-2 flex-shrink-0">
                    {isUnread && (
                      <div
                        className="w-2 h-2 rounded-full"
                        style={{ backgroundColor: meta.color }}
                      />
                    )}
                  </div>

                  {/* Image thumbnail */}
                  {item.image_url && (
                    <img
                      src={item.image_url}
                      alt=""
                      className="w-10 h-10 rounded-lg object-cover flex-shrink-0 mt-0.5"
                    />
                  )}

                  {/* Category badge */}
                  <div
                    className="px-2 py-0.5 rounded-md text-[10px] font-semibold uppercase flex-shrink-0 mt-0.5"
                    style={{ color: meta.color, backgroundColor: `${meta.color}15` }}
                  >
                    {meta.label}
                  </div>

                  {/* Content */}
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <p className={clsx('text-sm', isUnread ? 'text-white font-medium' : 'text-zinc-300')}>
                        {item.title}
                      </p>
                      {collapsedLabel && (
                        <span className="text-[10px] text-zinc-500 bg-white/5 px-1.5 py-0.5 rounded whitespace-nowrap">
                          {collapsedLabel}
                        </span>
                      )}
                    </div>
                    <p className="text-xs text-zinc-500 mt-0.5 line-clamp-2">{item.body}</p>

                    {/* Action buttons */}
                    {item.actions && item.actions.length > 0 && (
                      <div className="flex items-center gap-2 mt-2">
                        {item.actions.map((action: NotificationAction) => (
                          <button
                            key={action.id}
                            onClick={(e) => {
                              e.stopPropagation();
                              handleActionClick(item, action);
                            }}
                            className="text-xs font-medium px-2.5 py-1 rounded-md bg-violet-500/15 text-violet-300 hover:bg-violet-500/25 transition-colors"
                          >
                            {action.title}
                          </button>
                        ))}
                      </div>
                    )}
                  </div>

                  {/* Time and actions */}
                  <div className="flex items-center gap-2 flex-shrink-0">
                    <span className="text-[11px] text-zinc-500">{formatNotificationTime(item.created_at)}</span>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        deleteNotification(item.id);
                      }}
                      className="text-zinc-600 hover:text-red-400 opacity-0 group-hover:opacity-100 transition-opacity p-1 rounded hover:bg-white/5"
                      title="Delete"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
