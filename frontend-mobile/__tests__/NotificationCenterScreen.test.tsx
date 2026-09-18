// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the notification center — the unread dot, a category's own hue, mono time, day grouping, the EmptyState and delete by swipe or long-press
// ABOUTME: Mocks the notification hooks and the platform menu; pins the noCategoryNotifications bug fix (a translated label, never the raw category key)

import React from 'react';
import { ActionSheetIOS, Alert } from 'react-native';
import { render, fireEvent, waitFor, within } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import { dayLabelFor } from '@pierre/chat-utils';
import {
  NOTIFICATION_CATEGORIES,
  NOTIFICATION_CATEGORY_COLORS,
} from '@pierre/shared-constants';
import type { NotificationItem } from '@pierre/shared-types';

jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({ push: jest.fn(), back: jest.fn(), replace: jest.fn(), canGoBack: () => true }),
  }),
);

const mockUseNotificationFeed = jest.fn();
const mockUseUnreadCount = jest.fn();
const mockMarkAsRead = jest.fn();
const mockMarkAllAsRead = jest.fn();
const mockDeleteNotification = jest.fn();

jest.mock('../src/hooks/useNotifications', () => ({
  useNotificationFeed: (...args: unknown[]) => mockUseNotificationFeed(...args),
  useUnreadCount: () => mockUseUnreadCount(),
  useNotificationActions: () => ({
    markAsRead: mockMarkAsRead,
    markAllAsRead: mockMarkAllAsRead,
    deleteNotification: mockDeleteNotification,
    isMarkingRead: false,
    isMarkingAllRead: false,
    isDeleting: false,
  }),
}));

import { NotificationCenterScreen } from '../src/screens/notifications/NotificationCenterScreen';

function createNotification(overrides: Partial<NotificationItem> = {}): NotificationItem {
  return {
    id: 'notif-1',
    category: 'training',
    notification_type: 'session_ready',
    title: 'Thursday session ready',
    body: '5 x 4 min in Z5 — Camille put it on the calendar.',
    data: null,
    image_url: null,
    read_at: null,
    delivered_at: null,
    opened_at: null,
    created_at: new Date().toISOString(),
    ...overrides,
  };
}

/** A loaded feed, unread count derived the same way the real hook derives it. */
function loadedFeed(notifications: NotificationItem[]) {
  return {
    notifications,
    total: notifications.length,
    unreadCount: notifications.filter((n) => !n.read_at).length,
    isLoading: false,
    isRefetching: false,
    isError: false,
    error: null,
    refetch: jest.fn(),
    invalidate: jest.fn(),
  };
}

function renderScreen(): ReturnType<typeof render> {
  return render(<NotificationCenterScreen />);
}

/** The rows the platform sheet last offered, and a way to pick one by index. */
function presentedSheet() {
  const spy = ActionSheetIOS.showActionSheetWithOptions as unknown as jest.Mock;
  expect(spy).toHaveBeenCalled();
  const [, callback] = spy.mock.calls[spy.mock.calls.length - 1] as [unknown, (index: number) => void];
  return { pick: callback };
}

/** An `Alert.alert` that taps the destructive button the way a confirming athlete would. */
function confirmDestructiveAlerts(): jest.SpyInstance {
  return jest.spyOn(Alert, 'alert').mockImplementation((_title, _msg, buttons) => {
    const confirm = (buttons ?? []).find((b) => b.style === 'destructive');
    confirm?.onPress?.();
  });
}

describe('NotificationCenterScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseUnreadCount.mockReturnValue({ unreadCount: 0, isLoading: false });
  });

  afterEach(() => {
    jest.restoreAllMocks();
    jest.useRealTimers();
  });

  it('shows the unread dot only on an unread row', async () => {
    mockUseNotificationFeed.mockReturnValue(
      loadedFeed([
        createNotification({ id: 'unread-1', read_at: null }),
        createNotification({ id: 'read-1', read_at: new Date().toISOString() }),
      ]),
    );
    const { getByTestId } = renderScreen();

    await waitFor(() => expect(getByTestId('notification-row-unread-1')).toBeTruthy());

    expect(
      within(getByTestId('notification-row-unread-1')).getByTestId('notification-unread-dot'),
    ).toBeTruthy();
    expect(
      within(getByTestId('notification-row-read-1')).queryByTestId('notification-unread-dot'),
    ).toBeNull();
  });

  it("colors each category's word with its own hue, not a shared ink", async () => {
    mockUseNotificationFeed.mockReturnValue(
      loadedFeed([
        createNotification({ id: 'n-training', category: 'training' }),
        createNotification({ id: 'n-recovery', category: 'recovery' }),
      ]),
    );
    const { getByTestId } = renderScreen();
    await waitFor(() => expect(getByTestId('notification-row-n-training')).toBeTruthy());

    const trainingLabel = within(getByTestId('notification-row-n-training')).getByTestId(
      'notification-category',
    );
    const recoveryLabel = within(getByTestId('notification-row-n-recovery')).getByTestId(
      'notification-category',
    );
    // Scheme-agnostic on purpose: the row takes its hue from the athlete's
    // active scheme, so the assertion is that both words come from the SAME
    // scheme's map and are different hues within it — not that either matches
    // one half the harness happens to render.
    const trainingColor = (trainingLabel.props.style as { color: string }).color;
    const recoveryColor = (recoveryLabel.props.style as { color: string }).color;
    const scheme = (['light', 'dark'] as const).find(
      (candidate) => NOTIFICATION_CATEGORY_COLORS[candidate].training === trainingColor,
    );
    expect(scheme).toBeDefined();
    expect(recoveryColor).toBe(NOTIFICATION_CATEGORY_COLORS[scheme!].recovery);
    expect(trainingColor).not.toEqual(recoveryColor);
  });

  // `--color-surface` from global.css, which `bg-background-primary` resolves
  // to and the category word sits on.
  const SURFACE = { light: '#f7f6f2', dark: '#11130f' } as const;

  /** WCAG 2.1 relative luminance. */
  function luminance(hex: string): number {
    const channels = [1, 3, 5].map((at) => parseInt(hex.slice(at, at + 2), 16) / 255);
    const linear = channels.map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
    return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
  }

  function contrast(a: string, b: string): number {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  }

  // The category word is text, so it owes 4.5:1 against the surface it sits
  // on. One flat hex per category could not pay that in both schemes: the
  // near-black forest greens landed at 1.13:1 (coach) and 1.50:1 (ai) on the
  // dark canvas, where the label was the canvas.
  it.each(['light', 'dark'] as const)(
    'paints every category word at 4.5:1 or better on the %s surface',
    (scheme) => {
      const failures = NOTIFICATION_CATEGORIES.filter(
        (category) => contrast(NOTIFICATION_CATEGORY_COLORS[scheme][category], SURFACE[scheme]) < 4.5,
      );
      expect(failures).toEqual([]);
      expect(NOTIFICATION_CATEGORIES.length).toBe(7);
    },
  );

  it('prints the row time in mono, tabular figures', async () => {
    mockUseNotificationFeed.mockReturnValue(loadedFeed([createNotification()]));
    const { getByTestId } = renderScreen();

    await waitFor(() => expect(getByTestId('notification-time')).toBeTruthy());
    const time = getByTestId('notification-time');
    expect(time.props.className).toContain('font-mono');
    expect(time.props.className).toContain('tabular-nums');
  });

  it('groups the feed by day — today, yesterday, and an older day spelled out', async () => {
    // A fixed noon-UTC "now" — never near a local-midnight boundary — so the
    // 26h/72h offsets below land on the intended calendar day regardless of
    // the host's timezone or the real wall-clock time the suite runs at.
    // Un-mocked, this test read the CI runner's real Date.now(): whenever
    // that happened to fall within ~02:00 of UTC midnight, "26 hours ago"
    // crossed two calendar-day boundaries instead of one and "yesterday"
    // silently became a dated label, failing only in that window.
    jest.useFakeTimers();
    jest.setSystemTime(new Date('2026-06-15T12:00:00.000Z'));

    const today = createNotification({ id: 'today-1', created_at: new Date().toISOString() });
    const yesterdayIso = new Date(Date.now() - 26 * 3_600_000).toISOString();
    const yesterday = createNotification({ id: 'yesterday-1', created_at: yesterdayIso });
    const olderIso = new Date(Date.now() - 3 * 86_400_000).toISOString();
    const older = createNotification({ id: 'older-1', created_at: olderIso });

    mockUseNotificationFeed.mockReturnValue(loadedFeed([today, yesterday, older]));
    const { getByText, getAllByTestId } = renderScreen();

    const olderDayLabel = dayLabelFor(olderIso, 'en');
    const expectedOlderLabel = olderDayLabel.kind === 'date' ? olderDayLabel.label : '';
    await waitFor(() => {
      expect(getByText(i18n.t('chat.dayToday'))).toBeTruthy();
    });
    expect(getByText(i18n.t('chat.dayYesterday'))).toBeTruthy();
    expect(getByText(expectedOlderLabel)).toBeTruthy();
    expect(getAllByTestId('notification-day-header')).toHaveLength(3);
  });

  it('renders the empty state through ui/EmptyState, with the category label translated — not the raw enum key', async () => {
    mockUseNotificationFeed.mockReturnValue(loadedFeed([]));
    const { getByTestId, rerender } = renderScreen();

    await waitFor(() => expect(getByTestId('notifications-empty')).toBeTruthy());
    expect(getByTestId('notifications-empty')).toHaveTextContent(
      `${i18n.t('app.noNotificationsYet')} ${i18n.t('app.allCaughtUp')}`,
    );

    // The bug this replaces interpolated the raw enum ("training") into the
    // sentence; the fix passes the same translated word the row's own
    // category label and the filter tab already read.
    mockUseNotificationFeed.mockReturnValue(loadedFeed([]));
    rerender(<NotificationCenterScreen />);
    fireEvent.press(getByTestId('filter-training'));

    await waitFor(() => {
      expect(getByTestId('notifications-empty')).toHaveTextContent(
        `${i18n.t('app.noNotificationsYet')} ${i18n.t('app.noCategoryNotifications', {
          category: i18n.t('notifPrefs.catTraining'),
        })}`,
      );
    });
    expect(getByTestId('notifications-empty')).not.toHaveTextContent(/\btraining\b/);
  });

  it("shows 'mark all read' as an ink header action only while something is unread", async () => {
    mockUseNotificationFeed.mockReturnValue(loadedFeed([createNotification({ read_at: null })]));
    mockUseUnreadCount.mockReturnValue({ unreadCount: 1, isLoading: false });
    const { getByTestId, queryByTestId, rerender } = renderScreen();

    await waitFor(() => expect(getByTestId('mark-all-read')).toBeTruthy());
    fireEvent.press(getByTestId('mark-all-read'));
    expect(mockMarkAllAsRead).toHaveBeenCalled();

    mockUseUnreadCount.mockReturnValue({ unreadCount: 0, isLoading: false });
    rerender(<NotificationCenterScreen />);
    await waitFor(() => expect(queryByTestId('mark-all-read')).toBeNull());
  });

  it('deletes a notification from the swipe action and from the long-press menu, through the same confirm', async () => {
    mockUseNotificationFeed.mockReturnValue(loadedFeed([createNotification({ id: 'notif-1' })]));
    jest.spyOn(ActionSheetIOS, 'showActionSheetWithOptions').mockImplementation(() => undefined);
    confirmDestructiveAlerts();

    const { getByTestId } = renderScreen();
    await waitFor(() =>
      expect(getByTestId('notification-row-notif-1-swipe-action-delete')).toBeTruthy(),
    );

    fireEvent.press(getByTestId('notification-row-notif-1-swipe-action-delete'));
    await waitFor(() => expect(mockDeleteNotification).toHaveBeenCalledWith('notif-1'));
    expect(Alert.alert).toHaveBeenCalledWith(
      i18n.t('shell.notificationDelete'),
      undefined,
      expect.any(Array),
    );

    mockDeleteNotification.mockClear();
    fireEvent(getByTestId('notification-row-notif-1'), 'longPress');
    presentedSheet().pick(0);

    await waitFor(() => expect(mockDeleteNotification).toHaveBeenCalledWith('notif-1'));
  });
});
