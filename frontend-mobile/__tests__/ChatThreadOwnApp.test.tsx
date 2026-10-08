// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A reply's connect in a chat thread asks for the athlete's own OAuth app while no app of the server can authorize it
// ABOUTME: Pins that gate and the status re-read on closing the credentials sheet, so a saved app does not reopen it

import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react-native';
import type { ExtendedProviderStatus } from '@pierre/shared-types';
import { ChatThread, type ChatThreadProps } from '../src/screens/chat/ChatThread';

jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn() }));
// The transcript, reduced to a reply asking to connect WHOOP.
jest.mock('../src/screens/chat/MessageList', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    MessageList: ({ onReconnectProvider }: { onReconnectProvider: (provider: string) => void }) =>
      React.createElement(Text, { onPress: () => onReconnectProvider('whoop') }, 'reply:connect-whoop'),
  };
});
jest.mock('../src/screens/chat/ChatInputBar', () => ({ ChatInputBar: () => null }));
jest.mock('../src/screens/chat/ChatProgressStrip', () => ({ ChatProgressStrip: () => null }));
jest.mock('../src/screens/chat/UsageWarningBanner', () => ({ UsageWarningBanner: () => null }));
jest.mock('../src/screens/chat/VerdictSheet', () => ({ VerdictSheet: () => null }));
jest.mock('../src/screens/chat/useChatVoiceInput', () => ({ useChatVoiceInput: () => ({}) }));
jest.mock('../src/components/ProviderNotice', () => ({ ProviderNoticeSheet: () => null }));
jest.mock('../src/components/OAuthCredentialsSection', () => ({ OAuthCredentialsSection: () => null }));

const whoop: ExtendedProviderStatus = {
  provider: 'whoop',
  display_name: 'WHOOP',
  description: '',
  requires_oauth: true,
  connected: false,
  needs_reauth: false,
  capabilities: ['sleep', 'recovery'],
  consent_required: false,
  own_app_required: true,
  oauth_callback_url: 'https://app.dravr.ai/api/oauth/callback/whoop',
};

function providerStatus(
  overrides: Partial<ChatThreadProps['providerStatus']>,
): ChatThreadProps['providerStatus'] {
  return {
    connectedProviders: [whoop],
    providersLoaded: true,
    selectedProvider: null,
    connectingProvider: null,
    needsCredentialsProvider: null,
    error: null,
    loadProviderStatus: jest.fn().mockResolvedValue(undefined),
    hasConnectedProvider: () => false,
    setSelectedProvider: jest.fn(),
    setNeedsCredentialsProvider: jest.fn(),
    handleConnectProvider: jest.fn().mockResolvedValue(undefined),
    getCachedConnectedProvider: () => undefined,
    ...overrides,
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

describe('ChatThread — an app of the athlete\'s own', () => {
  it('opens the credentials sheet instead of an OAuth start while no app of the server can authorize it', () => {
    const status = providerStatus({});
    renderThread(status);

    fireEvent.press(screen.getByText('reply:connect-whoop'));

    expect(status.setNeedsCredentialsProvider).toHaveBeenCalledWith('whoop');
    expect(status.handleConnectProvider).not.toHaveBeenCalled();
  });

  it('starts the OAuth flow while an app of the server can authorize it', () => {
    const status = providerStatus({ connectedProviders: [{ ...whoop, own_app_required: false }] });
    renderThread(status);

    fireEvent.press(screen.getByText('reply:connect-whoop'));

    expect(status.handleConnectProvider).toHaveBeenCalledWith('whoop');
    expect(status.setNeedsCredentialsProvider).not.toHaveBeenCalled();
  });

  it('re-reads the provider statuses when the credentials sheet closes', () => {
    const status = providerStatus({ needsCredentialsProvider: 'whoop' });
    renderThread(status);

    fireEvent.press(screen.getByText('Close'));

    expect(status.setNeedsCredentialsProvider).toHaveBeenCalledWith(null);
    expect(status.loadProviderStatus).toHaveBeenCalledTimes(1);
  });
});
