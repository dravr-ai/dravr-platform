// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The bell at the top right of Home, Groups and Discover — the unread count, and the notifications in a sheet
// ABOUTME: Where an athlete looks for notifications; the rail no longer carries them (carnet#820)

import { lazy, Suspense, useState } from 'react';
import { Bell } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import { badgeLabel } from '@pierre/shared-constants';
import { Sheet } from '../ui/Sheet';
import { useUnreadCount } from '../../hooks/useNotifications';
import { useIsMobile } from '../../hooks/useBreakpoint';

const NotificationsPanel = lazy(() => import('./NotificationsPanel'));

interface NotificationBellProps {
  /** Dashboard route navigator: a notification that opens somewhere closes the sheet and goes there. */
  onNavigate?: (route: string) => void;
}

/**
 * A bell in a page header. The badge is the unread count the rail used to
 * carry; a press opens the notifications as a sheet over the page — from the
 * bottom on a phone, from the right on a wider screen — so reading them
 * never leaves the surface the athlete is on.
 */
export function NotificationBell({ onNavigate }: NotificationBellProps) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const { unreadCount } = useUnreadCount();
  const [open, setOpen] = useState(false);
  const label =
    unreadCount > 0 ? t('shell.notificationsBellUnread', { count: unreadCount }) : t('nav.notifications');

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        data-testid="notification-bell"
        aria-haspopup="dialog"
        aria-label={label}
        title={label}
        className="relative flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-on-surface-variant transition-colors hover:bg-surface-container-low hover:text-on-surface focus-ring touch-target"
      >
        <Bell className="h-[18px] w-[18px]" aria-hidden="true" />
        {unreadCount > 0 && (
          <span
            aria-hidden="true"
            data-testid="notification-bell-badge"
            className="absolute -right-1 -top-1 flex h-[18px] min-w-[18px] items-center justify-center rounded-full bg-primary px-1 text-xs font-semibold text-on-primary ring-2 ring-surface"
          >
            {badgeLabel(unreadCount)}
          </span>
        )}
      </button>
      {open && (
        <Sheet
          side={isMobile ? 'bottom' : 'right'}
          title={t('nav.notifications')}
          onClose={() => setOpen(false)}
          data-testid="notification-sheet"
        >
          <Suspense
            fallback={
              <div className="flex justify-center py-8" role="status" aria-label={t('common.loading')}>
                <div className="pierre-spinner" />
              </div>
            }
          >
            <NotificationsPanel
              embedded
              onNavigate={(route) => {
                setOpen(false);
                onNavigate?.(route);
              }}
            />
          </Suspense>
        </Sheet>
      )}
    </>
  );
}
