// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the notification center — the unread dot, a category's own hue, mono time, day grouping, the EmptyState and delete by swipe or long-press
// ABOUTME: Mocks the notification hooks and the platform menu; pins the noCategoryNotifications bug fix (a translated label, never the raw category key)

import React from 'react';
import { ActionSheetIOS, Alert } from 'react-native';
import { render, fireEvent, waitFor, within } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import { dayLabelFor } from '@pierre/chat-utils';
import { NOTIFICATION_CATEGORY_META } from '@pierre/shared-constants';
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
    expect(trainingLabel.props.style).toEqual(
      expect.objectContaining({ color: NOTIFICATION_CATEGORY_META.training.color }),
    );
    expect(recoveryLabel.props.style).toEqual(
      expect.objectContaining({ color: NOTIFICATION_CATEGORY_META.recovery.color }),
    );
    expect(NOTIFICATION_CATEGORY_META.training.color).not.toEqual(
      NOTIFICATION_CATEGORY_META.recovery.color,
    );
  });

  it('prints the row time in mono, tabular figures', async () => {
    mockUseNotificationFeed.mockReturnValue(loadedFeed([createNotification()]));
    const { getByTestId } = renderScreen();

    await waitFor(() => expect(getByTestId('notification-time')).toBeTruthy());
    const time = getByTestId('notification-time');
    expect(time.props.className).toContain('font-mono');
    expect(time.props.className).toContain('tabular-nums');
  });

  it('groups the feed by day — today, yesterday, and an older day spelled out', async () => {
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
