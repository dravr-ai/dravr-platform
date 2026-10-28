// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The notification bell is a button named for what it does and how many are unread
// ABOUTME: Unlabeled, VoiceOver would announce only the badge's digit — "1" — on every header that carries the bell

import React from 'react';
import { act, render, screen } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import { NotificationBellButton } from '../NotificationBellButton';

const unread = { count: 0 };
jest.mock('../../../hooks/useNotifications', () => ({
  useUnreadCount: () => ({ unreadCount: unread.count }),
}));

describe('NotificationBellButton', () => {
  it('names the unread count when there is one', () => {
    unread.count = 3;
    render(<NotificationBellButton />);
    const bell = screen.getByTestId('notification-bell');
    expect(bell.props.accessibilityLabel).toBe('Notifications, 3 unread');
    expect(bell.props.accessibilityRole).toBe('button');
    expect(screen.getByLabelText('Notifications, 3 unread')).toBeTruthy();
  });

  // carnet#1212: the label had one key for every count, so VoiceOver read
  // « Notifications, 1 non lues » to a French athlete.
  it('names one unread in the French singular and several in the plural', async () => {
    await act(async () => {
      await i18n.changeLanguage('fr');
    });
    try {
      unread.count = 1;
      const { unmount } = render(<NotificationBellButton />);
      expect(screen.getByTestId('notification-bell').props.accessibilityLabel).toBe('Notifications, 1 non lue');
      unmount();
      unread.count = 2;
      render(<NotificationBellButton />);
      expect(screen.getByTestId('notification-bell').props.accessibilityLabel).toBe('Notifications, 2 non lues');
    } finally {
      await act(async () => {
        await i18n.changeLanguage('en');
      });
    }
  });

  it('is named Notifications when nothing is unread', () => {
    unread.count = 0;
    render(<NotificationBellButton />);
    expect(screen.getByTestId('notification-bell').props.accessibilityLabel).toBe('Notifications');
  });
});
