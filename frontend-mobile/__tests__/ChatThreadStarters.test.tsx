// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A starter tapped in the phone's empty thread is sent at once as its postback, under its label (carnet#829)
// ABOUTME: Pins what the thread hands the host's one send, which opens the thread with the athlete speaking first

import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react-native';
import type { UseCaseStarter } from '@pierre/shared-types';
import { ChatThread, type ChatThreadProps } from '../src/screens/chat/ChatThread';

jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn() }));
// The transcript, reduced to the empty thread's starters as links.
jest.mock('../src/screens/chat/MessageList', () => {
  const React = require('react');
  const { Text, View } = require('react-native');
  return {
    MessageList: ({
      starters,
      onStarter,
    }: {
      starters?: { id: string; label: string }[];
      onStarter?: (starter: unknown) => void;
    }) =>
      React.createElement(
        View,
        null,
        (starters ?? []).map((starter) =>
          React.createElement(Text, { key: starter.id, onPress: () => onStarter?.(starter) }, starter.label),
        ),
      ),
  };
});
jest.mock('../src/screens/chat/ChatInputBar', () => ({ ChatInputBar: () => null }));
jest.mock('../src/screens/chat/UsageWarningBanner', () => ({ UsageWarningBanner: () => null }));
jest.mock('../src/screens/chat/VerdictSheet', () => ({ VerdictSheet: () => null }));
jest.mock('../src/screens/chat/useChatVoiceInput', () => ({ useChatVoiceInput: () => ({}) }));
jest.mock('../src/components/ProviderNotice', () => ({ ProviderNoticeSheet: () => null }));
jest.mock('../src/components/OAuthCredentialsSection', () => ({ OAuthCredentialsSection: () => null }));

const STARTERS: UseCaseStarter[] = [
  { id: 'connect', label: 'Connect my watch', postback: 'uc:0:connect' },
  { id: 'about_me_more', label: 'Finish my profile', postback: 'uc:1:about_me_more' },
];

function renderThread(sendText: ChatThreadProps['sendText'], starters?: UseCaseStarter[]) {
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
  const providerStatus = {
    connectedProviders: [],
    setNeedsCredentialsProvider: jest.fn(),
    loadProviderStatus: jest.fn(),
    needsCredentialsProvider: null,
  } as unknown as ChatThreadProps['providerStatus'];
  return render(
    <ChatThread
      conversationId={null}
      messagesHook={messagesHook}
      usageStatus={usageStatus}
      providerStatus={providerStatus}
      isLoading={false}
      inputText=""
      onChangeInputText={jest.fn()}
      inputRef={React.createRef()}
      sendText={sendText}
      starters={starters}
    />,
  );
}

describe('ChatThread — the empty thread\'s starters', () => {
  it('sends a tapped starter as its postback, with its label on the athlete\'s bubble', () => {
    const sendText = jest.fn().mockResolvedValue(undefined);
    renderThread(sendText, STARTERS);

    fireEvent.press(screen.getByText('Finish my profile'));

    expect(sendText).toHaveBeenCalledTimes(1);
    expect(sendText).toHaveBeenCalledWith('uc:1:about_me_more', { display: 'Finish my profile' });
  });

  it('draws none when the host has none to offer', () => {
    renderThread(jest.fn(), undefined);
    expect(screen.queryByText('Connect my watch')).toBeNull();
  });
});
