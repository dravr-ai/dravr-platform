// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Full-page notification center panel with category filters and actions
// ABOUTME: Renders as a tab in the Dashboard with feed, mark-all-read, and pagination

import { useState, useCallback } from 'react';
import { clsx } from 'clsx';
import { CheckCheck, Trash2 } from 'lucide-react';
import { TabHeader } from '../ui/TabHeader';
import {
  useNotificationFeed,
  useNotificationActions,
} from '../../hooks/useNotifications';
import {
  NOTIFICATION_CATEGORY_META,
  NOTIFICATION_CATEGORIES,
  formatNotificationTime,
  formatCollapsedCount,
  webNotificationRoute,
} from '@pierre/shared-constants';
import type { NotificationCategory, NotificationItem, NotificationAction } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';

interface NotificationsPanelProps {
  /** Callback when a notification with route data is clicked */
  onNavigate?: (route: string) => void;
}

export default function NotificationsPanel({ onNavigate }: NotificationsPanelProps) {
  const { t } = useTranslation();
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
      // Backend triggers (dravr-commere) emit `data.screen` as the
      // routing hint; the legacy `data.route` key was never wired on
      // the server side, so reading it left every Recovery / activity
      // notification stranded with no destination (web QA 2026-05-09).
      // Resolved from the server's own screen vocabulary against the shared
      // surface registry, so this panel, the mobile centre and any future
      // surface land in the same place. Coach messages carry the conversation
      // id on `data.id` and resolve to `chat/<id>`, which opens the thread
      // rather than the empty picker.
      const data = item.data as Record<string, unknown> | undefined;
      const route = webNotificationRoute(data);
      if (route && onNavigate) {
        onNavigate(route);
      }
    },
    [markAsRead, onNavigate],
  );

  const handleActionClick = useCallback(
    (item: NotificationItem, action: NotificationAction) => {
      if (!item.read_at) {
        markAsRead(item.id);
      }
      const data = item.data as Record<string, unknown> | undefined;
      const route = webNotificationRoute(data, action.id);
      if (route && onNavigate) {
        onNavigate(route);
      }
    },
    [markAsRead, onNavigate],
  );

  /** Category filter list: 'all' + each category from shared constants */
  const categoryFilters = [
    { key: 'all' as const, label: t('discover.filterAll') },
    ...NOTIFICATION_CATEGORIES.map((cat) => ({
      key: cat,
      label: t(NOTIFICATION_CATEGORY_META[cat].labelKey),
    })),
  ];

  return (
    <div className="h-full flex flex-col">
      <TabHeader
        title={t('shell.navNotifications')}
        description={
          <>
            {unreadCount > 0 ? `${unreadCount} unread` : t('shell.notificationsCaughtUp')}
            {total > 0 && ` · ${total} total`}
          </>
        }
        actions={
          unreadCount > 0 ? (
            <button
              onClick={() => markAllAsRead()}
              disabled={isMarkingAllRead}
              className="btn-base btn-tertiary touch-target gap-1.5 text-sm disabled:opacity-50"
            >
              <CheckCheck className="w-4 h-4" aria-hidden="true" />
              {t('shell.notificationMarkAllRead')}
            </button>
          ) : null
        }
      />

      {/* Categories as text tabs — every category the feed knows, no icons, scrolling on a narrow screen. */}
      <div className="flex gap-5 border-b ghost-border px-6 overflow-x-auto">
        {categoryFilters.map(({ key, label }) => {
          const isActive = selectedCategory === key;
          return (
            <button
              key={key}
              onClick={() => setSelectedCategory(key)}
              className={clsx(
                '-mb-px flex touch-target items-center justify-center whitespace-nowrap border-b-2 pt-1 text-sm font-medium transition-colors',
                isActive
                  ? 'border-primary text-on-surface'
                  : 'border-transparent text-on-surface-variant hover:text-on-surface',
              )}
            >
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
          // One sentence where the rows would be, and the second in the caption size.
          <div className="px-4 py-3 sm:px-6">
            <p className="text-sm text-on-surface-variant">{t('shell.notificationsEmpty')}</p>
            <p className="mt-0.5 text-xs text-outline">
              {selectedCategory === 'all'
                ? t('frag.allCaughtUp')
                : t('shell.notificationsEmptyCategory', { category: selectedCategory })}
            </p>
          </div>
        ) : (
          <div>
            {notifications.map((item) => {
              const isUnread = !item.read_at;
              const meta = NOTIFICATION_CATEGORY_META[item.category];
              const collapsedLabel = formatCollapsedCount(item.collapsed_count);

              return (
                <div
                  key={item.id}
                  className="group flex min-h-[48px] cursor-pointer items-start gap-3 border-t ghost-border-faint px-4 py-2.5 transition-colors first:border-t-0 hover:bg-surface-container-low/60 sm:gap-4 sm:px-6"
                  onClick={() => handleNotificationClick(item)}
                >
                  {/* Unread indicator — the primary dot, the same mark the chat list uses */}
                  <div className="w-2 pt-2 flex-shrink-0">
                    {isUnread && <div className="w-2 h-2 rounded-full bg-primary" />}
                  </div>

                  {/* Image thumbnail */}
                  {item.image_url && (
                    <img
                      src={item.image_url}
                      alt=""
                      className="w-10 h-10 rounded-lg object-cover flex-shrink-0 mt-0.5"
                    />
                  )}

                  {/* Category — its pillar as a dot beside the word, never a coloured chip */}
                  <div className="inline-flex flex-shrink-0 items-center gap-1.5 pt-0.5 text-xs text-on-surface-variant whitespace-nowrap">
                    <span aria-hidden="true" className="h-2 w-2 rounded-full" style={{ backgroundColor: meta.color }} />
                    {t(meta.labelKey)}
                  </div>

                  {/* Content */}
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 min-w-0">
                      <p className={clsx('text-sm truncate', isUnread ? 'text-on-surface font-medium' : 'text-on-surface')}>
                        {item.title}
                      </p>
                      {collapsedLabel && (
                        <span className="whitespace-nowrap font-mono text-xs text-outline">{collapsedLabel}</span>
                      )}
                    </div>
                    <p className="text-xs text-outline mt-0.5 line-clamp-2">{item.body}</p>

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
                            className="text-xs font-medium px-1 py-1 text-primary hover:text-primary-hover transition-colors"
                          >
                            {action.title}
                          </button>
                        ))}
                      </div>
                    )}
                  </div>

                  {/* Time and actions */}
                  <div className="flex items-center gap-2 flex-shrink-0">
                    <span className="text-xs text-outline">{formatNotificationTime(item.created_at, t)}</span>
                    {/* Always visible on touch (coarse pointers have no :hover);
                        hover-reveal retained on >=sm pointer-fine devices. 44x44
                        hit area so it's tappable, not just hoverable. */}
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        deleteNotification(item.id);
                      }}
                      className="flex h-7 w-7 flex-shrink-0 items-center justify-center rounded text-on-surface-variant opacity-100 transition-opacity hover:bg-surface-container-low hover:text-error sm:opacity-0 sm:group-hover:opacity-100 touch-target"
                      aria-label={t('shell.notificationDelete')}
                      title={t('common.delete')}
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
