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
import type { NotificationCategory, NotificationItem } from '@pierre/shared-types';

const CATEGORY_FILTERS: { key: NotificationCategory | 'all'; label: string; Icon: React.ElementType }[] = [
  { key: 'all', label: 'All', Icon: Bell },
  { key: 'training', label: 'Training', Icon: Dumbbell },
  { key: 'recovery', label: 'Recovery', Icon: Heart },
  { key: 'social', label: 'Social', Icon: Users },
  { key: 'coach', label: 'Coach', Icon: MessageCircle },
  { key: 'achievement', label: 'Achievements', Icon: Trophy },
  { key: 'system', label: 'System', Icon: Settings },
  { key: 'ai', label: 'AI Insights', Icon: Brain },
  { key: 'reminders', label: 'Reminders', Icon: Clock },
];

const CATEGORY_COLORS: Record<NotificationCategory, string> = {
  training: 'text-green-400 bg-green-500/10',
  recovery: 'text-blue-400 bg-blue-500/10',
  social: 'text-indigo-400 bg-indigo-500/10',
  coach: 'text-sky-400 bg-sky-500/10',
  achievement: 'text-amber-400 bg-amber-500/10',
  system: 'text-slate-400 bg-slate-500/10',
  ai: 'text-cyan-400 bg-cyan-500/10',
  reminders: 'text-pink-400 bg-pink-500/10',
};

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

export default function NotificationsPanel() {
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
    },
    [markAsRead],
  );

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
        {CATEGORY_FILTERS.map(({ key, label, Icon }) => {
          const isActive = selectedCategory === key;
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
              const colorClass = CATEGORY_COLORS[item.category];

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
                      <div className="w-2 h-2 rounded-full bg-violet-500" />
                    )}
                  </div>

                  {/* Category badge */}
                  <div className={clsx('px-2 py-0.5 rounded-md text-[10px] font-semibold uppercase flex-shrink-0 mt-0.5', colorClass)}>
                    {item.category}
                  </div>

                  {/* Content */}
                  <div className="flex-1 min-w-0">
                    <p className={clsx('text-sm', isUnread ? 'text-white font-medium' : 'text-zinc-300')}>
                      {item.title}
                    </p>
                    <p className="text-xs text-zinc-500 mt-0.5 line-clamp-2">{item.body}</p>
                  </div>

                  {/* Time and actions */}
                  <div className="flex items-center gap-2 flex-shrink-0">
                    <span className="text-[11px] text-zinc-500">{formatRelativeTime(item.created_at)}</span>
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
