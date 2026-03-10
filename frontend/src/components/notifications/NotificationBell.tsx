// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Notification bell icon with unread badge and dropdown panel
// ABOUTME: Shows latest notifications in a dropdown; links to full notification page

import { useState, useRef, useEffect, useCallback } from 'react';
import { Bell, CheckCheck, Trash2, X } from 'lucide-react';
import { clsx } from 'clsx';
import {
  useNotificationFeed,
  useUnreadCount,
  useNotificationActions,
} from '../../hooks/useNotifications';
import type { NotificationItem, NotificationCategory } from '@pierre/shared-types';

const CATEGORY_COLORS: Record<NotificationCategory, string> = {
  training: 'text-green-400',
  recovery: 'text-blue-400',
  social: 'text-indigo-400',
  coach: 'text-sky-400',
  achievement: 'text-amber-400',
  system: 'text-slate-400',
  ai: 'text-cyan-400',
  reminders: 'text-pink-400',
};

function formatRelativeTime(dateStr: string): string {
  const now = Date.now();
  const date = new Date(dateStr).getTime();
  const diffMs = now - date;
  const diffMin = Math.floor(diffMs / 60_000);
  const diffHr = Math.floor(diffMs / 3_600_000);
  const diffDay = Math.floor(diffMs / 86_400_000);

  if (diffMin < 1) return 'Just now';
  if (diffMin < 60) return `${diffMin}m`;
  if (diffHr < 24) return `${diffHr}h`;
  if (diffDay < 7) return `${diffDay}d`;
  return new Date(dateStr).toLocaleDateString();
}

interface NotificationBellProps {
  /** Callback when user navigates to full notifications page */
  onViewAll?: () => void;
}

export function NotificationBell({ onViewAll }: NotificationBellProps) {
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const { unreadCount } = useUnreadCount();
  const { notifications } = useNotificationFeed({ limit: 10 });
  const { markAsRead, markAllAsRead, deleteNotification } = useNotificationActions();

  // Close dropdown on outside click
  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
        setIsOpen(false);
      }
    }
    if (isOpen) {
      document.addEventListener('mousedown', handleClickOutside);
    }
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [isOpen]);

  const handleNotificationClick = useCallback(
    (item: NotificationItem) => {
      if (!item.read_at) {
        markAsRead(item.id);
      }
    },
    [markAsRead],
  );

  return (
    <div className="relative" ref={dropdownRef}>
      {/* Bell button */}
      <button
        onClick={() => setIsOpen(!isOpen)}
        className="relative text-zinc-400 hover:text-white transition-colors min-w-[44px] min-h-[44px] flex items-center justify-center"
        title="Notifications"
        aria-label="Notifications"
      >
        <Bell className="w-4 h-4" />
        {unreadCount > 0 && (
          <span className="absolute top-1.5 right-1.5 min-w-[16px] h-[16px] rounded-full bg-violet-600 text-[9px] font-bold text-white flex items-center justify-center px-0.5">
            {unreadCount > 99 ? '99+' : unreadCount}
          </span>
        )}
      </button>

      {/* Dropdown panel */}
      {isOpen && (
        <div className="absolute right-0 top-full mt-2 w-[360px] max-h-[480px] bg-pierre-slate border border-white/10 rounded-xl shadow-xl z-50 flex flex-col overflow-hidden">
          {/* Header */}
          <div className="flex items-center justify-between px-4 py-3 border-b border-white/10">
            <div className="flex items-center gap-2">
              <Bell className="w-4 h-4 text-violet-400" />
              <span className="text-sm font-semibold text-white">Notifications</span>
              {unreadCount > 0 && (
                <span className="px-1.5 py-0.5 rounded-full bg-violet-600/20 text-[10px] font-bold text-violet-300">
                  {unreadCount}
                </span>
              )}
            </div>
            <div className="flex items-center gap-1">
              {unreadCount > 0 && (
                <button
                  onClick={() => markAllAsRead()}
                  className="text-xs text-violet-400 hover:text-violet-300 px-2 py-1 rounded hover:bg-white/5 transition-colors flex items-center gap-1"
                  title="Mark all as read"
                >
                  <CheckCheck className="w-3 h-3" />
                  Read all
                </button>
              )}
              <button
                onClick={() => setIsOpen(false)}
                className="text-zinc-500 hover:text-white p-1 rounded hover:bg-white/5 transition-colors"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>

          {/* Notification list */}
          <div className="flex-1 overflow-y-auto">
            {notifications.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-10 text-zinc-500">
                <Bell className="w-8 h-8 mb-2 opacity-50" />
                <p className="text-sm">No notifications</p>
              </div>
            ) : (
              notifications.map((item) => {
                const isUnread = !item.read_at;
                return (
                  <div
                    key={item.id}
                    className={clsx(
                      'flex items-start gap-3 px-4 py-3 border-b border-white/5 cursor-pointer hover:bg-white/5 transition-colors group',
                      isUnread && 'bg-violet-500/5',
                    )}
                    onClick={() => handleNotificationClick(item)}
                  >
                    {/* Unread indicator */}
                    <div className="w-2 pt-1.5 flex-shrink-0">
                      {isUnread && (
                        <div className="w-2 h-2 rounded-full bg-violet-500" />
                      )}
                    </div>

                    {/* Content */}
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2 mb-0.5">
                        <span className={clsx('text-[10px] font-medium uppercase', CATEGORY_COLORS[item.category])}>
                          {item.category}
                        </span>
                        <span className="text-[10px] text-zinc-500">
                          {formatRelativeTime(item.created_at)}
                        </span>
                      </div>
                      <p className={clsx('text-xs truncate', isUnread ? 'text-white font-medium' : 'text-zinc-300')}>
                        {item.title}
                      </p>
                      <p className="text-[11px] text-zinc-500 truncate">{item.body}</p>
                    </div>

                    {/* Delete button */}
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        deleteNotification(item.id);
                      }}
                      className="text-zinc-600 hover:text-red-400 opacity-0 group-hover:opacity-100 transition-opacity p-1"
                      title="Delete"
                    >
                      <Trash2 className="w-3 h-3" />
                    </button>
                  </div>
                );
              })
            )}
          </div>

          {/* Footer */}
          {notifications.length > 0 && onViewAll && (
            <div className="border-t border-white/10 px-4 py-2">
              <button
                onClick={() => {
                  setIsOpen(false);
                  onViewAll();
                }}
                className="w-full text-center text-xs text-violet-400 hover:text-violet-300 py-1.5 rounded hover:bg-white/5 transition-colors"
              >
                View all notifications
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
