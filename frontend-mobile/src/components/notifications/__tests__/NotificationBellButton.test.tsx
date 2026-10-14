// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The notification bell is a button named for what it does and how many are unread
// ABOUTME: Unlabeled, VoiceOver would announce only the badge's digit — "1" — on every header that carries the bell

import React from 'react';
import { render, screen } from '@testing-library/react-native';
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

  it('is named Notifications when nothing is unread', () => {
    unread.count = 0;
    render(<NotificationBellButton />);
    expect(screen.getByTestId('notification-bell').props.accessibilityLabel).toBe('Notifications');
  });
});
