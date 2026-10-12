// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A reply's reconnect for Intervals.icu goes back the way the card was linked, never through an own-app setup
// ABOUTME: Pins the key sheet for a key-linked coach, the authorization for an authorized athlete, and the status re-read

import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react-native';
import type { ExtendedProviderStatus } from '@pierre/shared-types';
import { ChatThread, type ChatThreadProps } from '../src/screens/chat/ChatThread';

jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn() }));
// The transcript, reduced to a reply asking to reconnect Intervals.icu.
jest.mock('../src/screens/chat/MessageList', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    MessageList: ({ onReconnectProvider }: { onReconnectProvider: (provider: string) => void }) =>
      React.createElement(Text, { onPress: () => onReconnectProvider('intervals_icu') }, 'reply:reconnect-intervals'),
  };
});
jest.mock('../src/screens/chat/ChatInputBar', () => ({ ChatInputBar: () => null }));
jest.mock('../src/screens/chat/ChatProgressStrip', () => ({ ChatProgressStrip: () => null }));
jest.mock('../src/screens/chat/UsageWarningBanner', () => ({ UsageWarningBanner: () => null }));
jest.mock('../src/screens/chat/VerdictSheet', () => ({ VerdictSheet: () => null }));
jest.mock('../src/screens/chat/useChatVoiceInput', () => ({ useChatVoiceInput: () => ({}) }));
jest.mock('../src/components/ProviderNotice', () => ({ ProviderNoticeSheet: () => null }));
jest.mock('../src/components/OAuthCredentialsSection', () => ({ OAuthCredentialsSection: () => null }));
// The Intervals.icu sheet, reduced to which path it opened on, the control
// that starts the authorization when it was handed one, and a finished link.
jest.mock('../src/components/IntervalsIcuLinkModal', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    IntervalsIcuLinkModal: ({
      visible,
      onAuthorize,
      onConnected,
    }: {
      visible: boolean;
      onAuthorize?: () => void;
      onConnected: () => void;
    }) =>
      visible
        ? React.createElement(
            React.Fragment,
            null,
            React.createElement(
              Text,
              { onPress: onAuthorize, testID: 'intervals-sheet' },
              onAuthorize ? 'intervals-sheet:authorize' : 'intervals-sheet:key',
            ),
            React.createElement(Text, { onPress: onConnected }, 'intervals-linked'),
          )
        : null,
  };
});

function intervalsCard(overrides: Partial<ExtendedProviderStatus>): ExtendedProviderStatus {
  return {
    provider: 'intervals_icu',
    display_name: 'Intervals.icu',
    description: '',
    requires_oauth: true,
    connected: true,
    needs_reauth: true,
    capabilities: ['activities', 'health'],
    consent_required: false,
    own_app_required: false,
    oauth_callback_url: 'https://app.dravr.ai/api/oauth/callback/intervals_icu',
    ...overrides,
  };
}

function providerStatus(card: ExtendedProviderStatus): ChatThreadProps['providerStatus'] {
  return {
    connectedProviders: [card],
    providersLoaded: true,
    selectedProvider: null,
    connectingProvider: null,
    needsCredentialsProvider: null,
    error: null,
    loadProviderStatus: jest.fn().mockResolvedValue(undefined),
    hasConnectedProvider: () => true,
    setSelectedProvider: jest.fn(),
    setNeedsCredentialsProvider: jest.fn(),
    handleConnectProvider: jest.fn().mockResolvedValue(undefined),
    getCachedConnectedProvider: () => undefined,
    refreshAfterLink: jest.fn().mockResolvedValue(undefined),
  };
}

function renderThread(status: ChatThreadProps['providerStatus']) {
  // Only what the thread reads itself; every child that reads more is mocked.
  const messagesHook = {
    scrollToBottom: jest.fn(),
    quotaNotice: null,
    verdicts: [],
    verdictsLoading: false,
    refreshVerdicts: jest.fn(),
    messages: [],
  } as unknown as ChatThreadProps['messagesHook'];
  const usageStatus = { applyNotice: jest.fn() } as unknown as ChatThreadProps['usageStatus'];
  return render(
    <ChatThread
      conversationId="c1"
      messagesHook={messagesHook}
      usageStatus={usageStatus}
      providerStatus={status}
      isLoading={false}
      inputText=""
      onChangeInputText={jest.fn()}
      inputRef={React.createRef()}
      sendText={jest.fn()}
    />,
  );
}

describe('ChatThread — reconnecting Intervals.icu', () => {
  it('reopens the key sheet for a key-linked coach, never the authorization', () => {
    const status = providerStatus(intervalsCard({ connection_type: 'manual' }));
    renderThread(status);

    fireEvent.press(screen.getByText('reply:reconnect-intervals'));

    expect(screen.getByText('intervals-sheet:key')).toBeTruthy();
    expect(status.handleConnectProvider).not.toHaveBeenCalled();
    expect(status.setNeedsCredentialsProvider).not.toHaveBeenCalled();
  });

  it('re-reads the statuses once the key is linked again', () => {
    const status = providerStatus(intervalsCard({ connection_type: 'manual' }));
    renderThread(status);

    fireEvent.press(screen.getByText('reply:reconnect-intervals'));
    fireEvent.press(screen.getByText('intervals-linked'));

    expect(status.refreshAfterLink).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('intervals-sheet')).toBeNull();
  });

  it('starts the authorization again for an athlete who authorized', () => {
    const status = providerStatus(intervalsCard({ connection_type: 'oauth' }));
    renderThread(status);

    fireEvent.press(screen.getByText('reply:reconnect-intervals'));

    expect(status.handleConnectProvider).toHaveBeenCalledWith('intervals_icu');
    expect(screen.queryByTestId('intervals-sheet')).toBeNull();
  });

  it('opens the key sheet, never an own-app setup, while no client resolves', () => {
    const status = providerStatus(intervalsCard({ connection_type: 'oauth', own_app_required: true }));
    renderThread(status);

    fireEvent.press(screen.getByText('reply:reconnect-intervals'));

    expect(screen.getByText('intervals-sheet:key')).toBeTruthy();
    expect(status.setNeedsCredentialsProvider).not.toHaveBeenCalled();
    expect(status.handleConnectProvider).not.toHaveBeenCalled();
  });
});
