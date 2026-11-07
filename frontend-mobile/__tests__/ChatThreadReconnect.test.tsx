// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A reply's reconnect in a chat thread follows its card's connect plan: the authorization by its provider's name, or the sign-in here
// ABOUTME: Pins that no `sciotte_*` key reaches the OAuth start, that Garmin reads the sciotte_garmin card, and that a sign-in re-reads the statuses

import React from 'react';
import { fireEvent, render, screen } from '@testing-library/react-native';
import type { ExtendedProviderStatus } from '@pierre/shared-types';
import { ChatThread, type ChatThreadProps } from '../src/screens/chat/ChatThread';

jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn() }));
// The slug the reply's reconnect names; set per test.
let mockSlug = 'garmin';
// The transcript, reduced to a reply asking to reconnect `mockSlug`.
jest.mock('../src/screens/chat/MessageList', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    MessageList: ({ onReconnectProvider }: { onReconnectProvider: (provider: string) => void }) =>
      React.createElement(Text, { onPress: () => onReconnectProvider(mockSlug) }, 'reply:reconnect'),
  };
});
// The sign-in window, reduced to its target, its notice, and its success.
jest.mock('../src/components/SciotteLoginModal', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    SciotteLoginModal: ({
      visible,
      target,
      consentRequired,
      onConnected,
      onOAuthRequired,
    }: {
      visible: boolean;
      target: string;
      consentRequired?: boolean;
      onConnected: () => void;
      onOAuthRequired?: () => void;
    }) =>
      visible
        ? React.createElement(
            React.Fragment,
            null,
            React.createElement(
              Text,
              { onPress: onConnected, testID: 'sciotte-modal' },
              `sciotte-modal:${target}${consentRequired ? ':consent' : ''}`,
            ),
            // The server's oauth_required refusal: the window stays open and says why.
            React.createElement(
              Text,
              { onPress: () => onOAuthRequired?.(), testID: 'sciotte-modal-oauth-required' },
              'refuse:oauth_required',
            ),
          )
        : null,
  };
});
jest.mock('../src/screens/chat/ChatInputBar', () => ({ ChatInputBar: () => null }));
jest.mock('../src/screens/chat/UsageWarningBanner', () => ({ UsageWarningBanner: () => null }));
jest.mock('../src/screens/chat/VerdictSheet', () => ({ VerdictSheet: () => null }));
jest.mock('../src/screens/chat/useChatVoiceInput', () => ({ useChatVoiceInput: () => ({}) }));
jest.mock('../src/components/ProviderNotice', () => ({ ProviderNoticeSheet: () => null }));
jest.mock('../src/components/OAuthCredentialsSection', () => ({ OAuthCredentialsSection: () => null }));

function card(overrides: Partial<ExtendedProviderStatus> & { provider: string }): ExtendedProviderStatus {
  return {
    display_name: overrides.provider,
    description: '',
    requires_oauth: false,
    connected: true,
    needs_reauth: true,
    capabilities: ['activities'],
    consent_required: false,
    own_app_required: false,
    ...overrides,
  } as ExtendedProviderStatus;
}

const COROS = card({ provider: 'sciotte_coros', display_name: 'COROS', connected_backend: 'sciotte_coros', consent_required: true });
const garmin = (recommended_backend: 'oauth' | 'mirror', connected_backend = 'garmin') =>
  card({ provider: 'sciotte_garmin', display_name: 'Garmin', connected_backend, recommended_backend });
const strava = (connected_backend: string, recommended_backend: 'oauth' | 'mirror') =>
  card({ provider: 'sciotte', display_name: 'Strava', connected_backend, recommended_backend });

function providerStatus(connectedProviders: ExtendedProviderStatus[]): ChatThreadProps['providerStatus'] {
  return {
    connectedProviders,
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

describe('ChatThread — a reply reconnects the way its card connects', () => {
  it('opens the COROS sign-in with its notice, never an OAuth start', () => {
    mockSlug = 'sciotte_coros';
    const status = providerStatus([COROS]);
    renderThread(status);

    fireEvent.press(screen.getByText('reply:reconnect'));

    expect(screen.getByText('sciotte-modal:coros:consent')).toBeTruthy();
    expect(status.handleConnectProvider).not.toHaveBeenCalled();
  });

  it('starts the Garmin authorization by its name, reading the sciotte_garmin card', () => {
    mockSlug = 'garmin';
    const status = providerStatus([garmin('oauth')]);
    renderThread(status);

    fireEvent.press(screen.getByText('reply:reconnect'));

    expect(status.handleConnectProvider).toHaveBeenCalledWith('garmin');
    expect(screen.queryByTestId('sciotte-modal')).toBeNull();
  });

  it('signs in to Garmin while its card has no seat for the authorization', () => {
    mockSlug = 'garmin';
    const status = providerStatus([garmin('mirror', 'sciotte_garmin')]);
    renderThread(status);

    fireEvent.press(screen.getByText('reply:reconnect'));

    expect(screen.getByText('sciotte-modal:garmin')).toBeTruthy();
    expect(status.handleConnectProvider).not.toHaveBeenCalled();
  });

  it('starts the Strava authorization for a Strava card served by its grant', () => {
    mockSlug = 'sciotte';
    const status = providerStatus([strava('strava', 'oauth')]);
    renderThread(status);

    fireEvent.press(screen.getByText('reply:reconnect'));

    expect(status.handleConnectProvider).toHaveBeenCalledWith('strava');
  });

  it('signs in to Strava again for a Strava card served by its sign-in', () => {
    mockSlug = 'sciotte';
    const status = providerStatus([strava('sciotte', 'oauth')]);
    renderThread(status);

    fireEvent.press(screen.getByText('reply:reconnect'));

    expect(screen.getByText('sciotte-modal:strava')).toBeTruthy();
    expect(status.handleConnectProvider).not.toHaveBeenCalled();
  });

  it('signs in for a sign-in backend before any card is read, never starting its OAuth', () => {
    mockSlug = 'sciotte_trainingpeaks';
    const status = providerStatus([]);
    renderThread(status);

    fireEvent.press(screen.getByText('reply:reconnect'));

    expect(screen.getByText('sciotte-modal:trainingpeaks')).toBeTruthy();
    expect(status.handleConnectProvider).not.toHaveBeenCalled();
  });

  it('keeps the window open on an oauth_required refusal and re-reads the statuses, whose plan then authorizes', () => {
    mockSlug = 'garmin';
    const status = providerStatus([garmin('mirror', 'sciotte_garmin')]);
    renderThread(status);

    fireEvent.press(screen.getByText('reply:reconnect'));
    fireEvent.press(screen.getByTestId('sciotte-modal-oauth-required'));

    expect(status.refreshAfterLink).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('sciotte-modal')).toBeTruthy();
  });

  it('re-reads the statuses once the sign-in connects', () => {
    mockSlug = 'sciotte_coros';
    const status = providerStatus([COROS]);
    renderThread(status);

    fireEvent.press(screen.getByText('reply:reconnect'));
    fireEvent.press(screen.getByTestId('sciotte-modal'));

    expect(status.refreshAfterLink).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('sciotte-modal')).toBeNull();
  });
});
