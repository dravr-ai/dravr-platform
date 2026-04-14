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
import {
  NOTIFICATION_CATEGORY_META,
  formatNotificationTime,
  formatCollapsedCount,
} from '@pierre/shared-constants';
import type { NotificationItem, NotificationAction } from '@pierre/shared-types';

interface NotificationBellProps {
  /** Callback when user navigates to full notifications page */
  onViewAll?: () => void;
  /** Callback when a notification with route data is clicked */
  onNavigate?: (route: string) => void;
}

export function NotificationBell({ onViewAll, onNavigate }: NotificationBellProps) {
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
      if (item.data?.route && typeof item.data.route === 'string' && onNavigate) {
        setIsOpen(false);
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
        setIsOpen(false);
        onNavigate(screen);
      }
    },
    [markAsRead, onNavigate],
  );

  return (
    <div className="relative" ref={dropdownRef}>
      {/* Bell button */}
      <button
        onClick={() => setIsOpen(!isOpen)}
        className="relative text-on-surface-variant hover:text-on-surface transition-colors min-w-[44px] min-h-[44px] flex items-center justify-center"
        title="Notifications"
        aria-label="Notifications"
      >
        <Bell className="w-4 h-4" />
        {unreadCount > 0 && (
          <span className="absolute top-1.5 right-1.5 min-w-[16px] h-[16px] rounded-full bg-violet-600 text-[9px] font-bold text-on-surface flex items-center justify-center px-0.5">
            {unreadCount > 99 ? '99+' : unreadCount}
          </span>
        )}
      </button>

      {/* Dropdown panel */}
      {isOpen && (
        <div className="absolute right-0 top-full mt-2 w-[360px] max-h-[480px] bg-surface-container-low border ghost-border rounded-xl shadow-xl z-50 flex flex-col overflow-hidden">
          {/* Header */}
          <div className="flex items-center justify-between px-4 py-3 border-b ghost-border">
            <div className="flex items-center gap-2">
              <Bell className="w-4 h-4 text-violet-400" />
              <span className="text-sm font-semibold text-on-surface">Notifications</span>
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
                  className="text-xs text-violet-400 hover:text-violet-300 px-2 py-1 rounded hover:bg-surface-container-low transition-colors flex items-center gap-1"
                  title="Mark all as read"
                >
                  <CheckCheck className="w-3 h-3" />
                  Read all
                </button>
              )}
              <button
                onClick={() => setIsOpen(false)}
                className="text-outline hover:text-on-surface p-1 rounded hover:bg-surface-container-low transition-colors"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>

          {/* Notification list */}
          <div className="flex-1 overflow-y-auto">
            {notifications.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-10 text-outline">
                <Bell className="w-8 h-8 mb-2 opacity-50" />
                <p className="text-sm">No notifications</p>
              </div>
            ) : (
              notifications.map((item) => {
                const isUnread = !item.read_at;
                const meta = NOTIFICATION_CATEGORY_META[item.category];
                const collapsedLabel = formatCollapsedCount(item.collapsed_count);
                return (
                  <div
                    key={item.id}
                    className={clsx(
                      'flex items-start gap-3 px-4 py-3 border-b ghost-border cursor-pointer hover:bg-surface-container-low transition-colors group',
                      isUnread && 'bg-violet-500/5',
                    )}
                    onClick={() => handleNotificationClick(item)}
                  >
                    {/* Unread indicator */}
                    <div className="w-2 pt-1.5 flex-shrink-0">
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
                        className="w-8 h-8 rounded-md object-cover flex-shrink-0 mt-0.5"
                      />
                    )}

                    {/* Content */}
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2 mb-0.5">
                        <span
                          className="text-[10px] font-medium uppercase"
                          style={{ color: meta.color }}
                        >
                          {meta.label}
                        </span>
                        <span className="text-[10px] text-outline">
                          {formatNotificationTime(item.created_at)}
                        </span>
                        {collapsedLabel && (
                          <span className="text-[10px] text-outline bg-surface-container-low px-1.5 py-0.5 rounded">
                            {collapsedLabel}
                          </span>
                        )}
                      </div>
                      <p className={clsx('text-xs truncate', isUnread ? 'text-on-surface font-medium' : 'text-on-surface')}>
                        {item.title}
                      </p>
                      <p className="text-[11px] text-outline truncate">{item.body}</p>

                      {/* Action buttons */}
                      {item.actions && item.actions.length > 0 && (
                        <div className="flex items-center gap-1.5 mt-1.5">
                          {item.actions.map((action: NotificationAction) => (
                            <button
                              key={action.id}
                              onClick={(e) => {
                                e.stopPropagation();
                                handleActionClick(item, action);
                              }}
                              className="text-[10px] font-medium px-2 py-1 rounded bg-violet-500/15 text-violet-300 hover:bg-violet-500/25 transition-colors"
                            >
                              {action.title}
                            </button>
                          ))}
                        </div>
                      )}
                    </div>

                    {/* Delete button */}
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        deleteNotification(item.id);
                      }}
                      className="text-on-surface-variant hover:text-red-400 opacity-0 group-hover:opacity-100 transition-opacity p-1"
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
            <div className="border-t ghost-border px-4 py-2">
              <button
                onClick={() => {
                  setIsOpen(false);
                  onViewAll();
                }}
                className="w-full text-center text-xs text-violet-400 hover:text-violet-300 py-1.5 rounded hover:bg-surface-container-low transition-colors"
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
