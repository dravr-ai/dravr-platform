// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingMessagingChannelScreen — the messaging-app picker step
// ABOUTME: Verifies channels render with the recommended badge and select/skip drive the shared hook

import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react-native';
import { OnboardingMessagingChannelScreen } from '../OnboardingMessagingChannelScreen';
import { useMessagingOnboarding } from '../../../hooks/useMessagingOnboarding';

jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'u1', display_name: 'Jean' } }),
}));
jest.mock('../../../hooks/useMessagingOnboarding', () => ({
  useMessagingOnboarding: jest.fn(),
}));

const chooseChannel = jest.fn();
const skipMessaging = jest.fn();

const baseState = {
  availableChannels: [
    { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true },
    { channel: 'slack', display_name: 'Slack', method: 'oauth', recommended: false },
  ],
  availableCount: 2,
  chosenChannel: null,
  channelChosen: false,
  channelDone: false,
  configureDone: false,
  loading: false,
  chooseChannel,
  completeConfigure: jest.fn(),
  skipMessaging,
};

describe('OnboardingMessagingChannelScreen', () => {
  beforeEach(() => {
    chooseChannel.mockClear();
    skipMessaging.mockClear();
    (useMessagingOnboarding as jest.Mock).mockReturnValue(baseState);
  });

  it('renders each channel and flags the recommended one', () => {
    render(<OnboardingMessagingChannelScreen />);
    expect(screen.getByText('Telegram')).toBeTruthy();
    expect(screen.getByText('Slack')).toBeTruthy();
    expect(screen.getByText('Recommended')).toBeTruthy();
  });

  it('chooses a channel on tap', () => {
    render(<OnboardingMessagingChannelScreen />);
    fireEvent.press(screen.getByRole('button', { name: 'Telegram' }));
    expect(chooseChannel).toHaveBeenCalledWith('telegram');
  });

  it('skips messaging via "Skip for now"', () => {
    render(<OnboardingMessagingChannelScreen />);
    fireEvent.press(screen.getByText('Skip for now'));
    expect(skipMessaging).toHaveBeenCalled();
  });
});
