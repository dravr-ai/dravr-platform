// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile ChatAppPicker — the chat-app choice the chat-app sheet opens on
// ABOUTME: Verifies channels render with the recommended badge, a tap chooses once, and a phone is never told to scan

import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react-native';
import { ChatAppPicker } from '../ChatAppPicker';

jest.mock('expo-device', () => ({
  deviceType: 1, // Device.DeviceType.PHONE
  DeviceType: { UNKNOWN: 0, PHONE: 1, TABLET: 2, DESKTOP: 3, TV: 4 },
}));

const CHANNELS = [
  { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true },
  { channel: 'slack', display_name: 'Slack', method: 'oauth', recommended: false },
];

describe('ChatAppPicker', () => {
  it('renders each channel and flags the recommended one', () => {
    render(<ChatAppPicker channels={CHANNELS} onSelect={jest.fn()} />);
    expect(screen.getByText('Telegram')).toBeTruthy();
    expect(screen.getByText('Slack')).toBeTruthy();
    expect(screen.getByText('Recommended')).toBeTruthy();
  });

  it('chooses a channel on tap, once', () => {
    const onSelect = jest.fn();
    render(<ChatAppPicker channels={CHANNELS} onSelect={onSelect} />);
    fireEvent.press(screen.getByRole('button', { name: 'Telegram' }));
    fireEvent.press(screen.getByRole('button', { name: 'Slack' }));
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith('telegram');
  });

  it('never offers a phone a QR to scan from the screen it is', () => {
    render(<ChatAppPicker channels={CHANNELS} onSelect={jest.fn()} />);
    expect(screen.queryByText(/QR/)).toBeNull();
  });
});
