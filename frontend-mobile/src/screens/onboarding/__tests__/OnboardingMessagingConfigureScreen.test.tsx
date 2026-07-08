// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingMessagingConfigureScreen — QR/deep-link configure + poll-to-complete
// ABOUTME: Verifies the QR (WebView) renders for a deep-link channel and the poll auto-advances via completeConfigure

import React from 'react';
import { render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { OnboardingMessagingConfigureScreen } from '../OnboardingMessagingConfigureScreen';
import { useMessagingOnboarding } from '../../../hooks/useMessagingOnboarding';
import { messagingApi } from '../../../services/api';

jest.mock('../../../contexts/AuthContext', () => ({ useAuth: () => ({ user: { id: 'u1' } }) }));
jest.mock('../../../hooks/useMessagingOnboarding', () => ({
  useMessagingOnboarding: jest.fn(),
}));
jest.mock('../../../services/api', () => ({
  messagingApi: { initLink: jest.fn(), listLinks: jest.fn() },
}));
jest.mock('react-native-webview', () => {
  const ReactMock = require('react');
  const { View } = require('react-native');
  return { WebView: (props: { testID?: string }) => ReactMock.createElement(View, { testID: props.testID }) };
});

const completeConfigure = jest.fn();
const skipMessaging = jest.fn();
const initLink = messagingApi.initLink as jest.Mock;
const listLinks = messagingApi.listLinks as jest.Mock;

const state = {
  availableChannels: [
    { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true },
  ],
  availableCount: 1,
  chosenChannel: 'telegram',
  channelChosen: true,
  channelDone: true,
  configureDone: false,
  loading: false,
  chooseChannel: jest.fn(),
  completeConfigure,
  skipMessaging,
};

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <OnboardingMessagingConfigureScreen />
    </QueryClientProvider>,
  );
}

describe('OnboardingMessagingConfigureScreen', () => {
  beforeEach(() => {
    completeConfigure.mockClear();
    skipMessaging.mockClear();
    (useMessagingOnboarding as jest.Mock).mockReturnValue(state);
    initLink.mockReset().mockResolvedValue({
      channel: 'telegram',
      method: 'deep_link',
      code: 'abc',
      linking_url: 'https://t.me/DravrBot?start=abc',
      expires_at: '2030-01-01T00:00:00Z',
      qr_svg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
    });
    listLinks.mockReset().mockResolvedValue([]);
  });

  it('renders the QR (WebView) and the open button for a deep-link channel', async () => {
    renderScreen();
    expect(await screen.findByTestId('messaging-qr')).toBeTruthy();
    expect(screen.getByText('Open Telegram')).toBeTruthy();
  });

  it('auto-advances via completeConfigure once the channel link appears', async () => {
    listLinks.mockResolvedValue([
      { channel: 'telegram', channel_user_id: 'u1', display_name: null, linked_at: 'now' },
    ]);
    renderScreen();
    await waitFor(() => expect(completeConfigure).toHaveBeenCalled());
  });
});
