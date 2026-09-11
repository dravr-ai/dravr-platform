// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the thread's header title view — its subtitle line: group, coach handle, or provider status
// ABOUTME: The provider line is the one the phone never rendered, so a dead session read as a quiet coach

import React from 'react';
import { render, screen } from '@testing-library/react-native';
import type { Conversation } from '../../../types';
import { ChatHeaderTitle } from '../ChatHeaderTitle';

jest.mock('@pierre/i18n', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

function conversation(overrides: Partial<Conversation> = {}): Conversation {
  return {
    id: 'conv-1',
    title: 'Tuesday intervals',
    group_id: null,
    group_name: null,
    agent_handle: null,
    ...overrides,
  } as Conversation;
}

function renderHeader(
  currentConversation: Conversation | null,
  providerStatus: string | null,
) {
  return render(
    <ChatHeaderTitle
      currentConversation={currentConversation}
      providerStatus={providerStatus}
      onTitlePress={jest.fn()}
    />,
  );
}

describe('ChatHeaderTitle subtitle', () => {
  /**
   * carnet#231: web has reported this since the single-source sweep, the phone
   * reported nothing. An athlete whose Strava session died saw an ordinary
   * header and a coach that had stopped citing their activities.
   */
  it('reports the provider status on a thread with no group and no handle', () => {
    renderHeader(conversation(), 'No provider connected');

    expect(screen.getByTestId('chat-header-provider-status')).toHaveTextContent(
      'No provider connected',
    );
  });

  it('says nothing while the provider status is still in flight', () => {
    renderHeader(conversation(), null);

    expect(screen.queryByTestId('chat-header-provider-status')).toBeNull();
  });

  /**
   * The provider line is a fallback, not an addition: a thread that already
   * names its group or its coach keeps saying that, exactly as web does.
   */
  it('yields to the group name', () => {
    renderHeader(
      conversation({ group_id: 'g-1', group_name: 'Sunday Long Run' }),
      'No provider connected',
    );

    expect(screen.getByTestId('chat-header-group')).toHaveTextContent('Sunday Long Run');
    expect(screen.queryByTestId('chat-header-provider-status')).toBeNull();
  });

  it('yields to the coach handle', () => {
    renderHeader(conversation({ agent_handle: 'trail' }), 'No provider connected');

    expect(screen.getByTestId('chat-header-handle')).toHaveTextContent('@trail');
    expect(screen.queryByTestId('chat-header-provider-status')).toBeNull();
  });

  /**
   * Before a thread exists the header names what the athlete is about to start,
   * and the line still applies — this is exactly where web puts it too, on the
   * empty state it renders instead of a thread header.
   */
  it('still reports the provider status before a thread exists', () => {
    renderHeader(null, 'No provider connected');

    expect(screen.getByTestId('chat-header-provider-status')).toHaveTextContent(
      'No provider connected',
    );
  });
});
