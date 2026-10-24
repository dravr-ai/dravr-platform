// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the thread's header title view — its subtitle line: group, coach handle, or provider status
// ABOUTME: The provider line is the one the phone never rendered, so a dead session read as a quiet coach

import React from 'react';
import { render, screen } from '@testing-library/react-native';
import type { Conversation } from '../../../types';
import { ChatHeaderTitle, threadTitle } from '../ChatHeaderTitle';

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
  agentTitle: string | null = null,
) {
  return render(
    <ChatHeaderTitle
      currentConversation={currentConversation}
      providerStatus={providerStatus}
      onTitlePress={jest.fn()}
      agentTitle={agentTitle}
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
  it('yields to the group name, and draws the room avatar as a square', () => {
    renderHeader(
      conversation({ group_id: 'g-1', group_name: 'Sunday Long Run' }),
      'No provider connected',
    );

    expect(screen.getByTestId('chat-header-group')).toHaveTextContent('Sunday Long Run');
    expect(screen.queryByTestId('chat-header-provider-status')).toBeNull();
    expect(
      screen.getByTestId('chat-header-avatar', { includeHiddenElements: true }).props.style,
    ).toEqual(expect.objectContaining({ borderRadius: 12 }));
  });

  it('draws a 1:1 thread avatar as a circle and prints the stored title', () => {
    renderHeader(conversation({ title: 'Trail Coach', agent_handle: 'trail' }), null);

    expect(screen.getByTestId('chat-title')).toHaveTextContent('Trail Coach');
    expect(
      screen.getByTestId('chat-header-avatar', { includeHiddenElements: true }).props.style,
    ).toEqual(expect.objectContaining({ borderRadius: 16 }));
  });

  it("names the athlete on a coach's plan thread, over the agent it runs on", () => {
    renderHeader(
      conversation({
        title: 'Alice Runner',
        agent_handle: 'marathon-coach',
        subject_user_id: 'user-alice',
        subject_group_id: 'group-1',
        subject_display_name: 'Alice Runner',
      }),
      'No provider connected',
    );

    // The translator here answers with the key; the line is the plan-thread subtitle.
    expect(screen.getByTestId('chat-header-subject')).toHaveTextContent('humanCoach.planThread.subtitle');
    expect(screen.queryByTestId('chat-header-handle')).toBeNull();
    expect(screen.queryByTestId('chat-header-provider-status')).toBeNull();
    // A person, so a circle.
    expect(
      screen.getByTestId('chat-header-avatar', { includeHiddenElements: true }).props.style,
    ).toEqual(expect.objectContaining({ borderRadius: 16 }));
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

/**
 * carnet#1146: the server stamps a rebound personal thread with the agent's
 * canonical English title. Home's conversation is named after its agent in the
 * athlete's language instead; a rename shows only in the list.
 */
describe('ChatHeaderTitle title', () => {
  const fuelling = conversation({
    title: 'Fuelling Agent',
    agent_id: 'agent-fuel',
    agent_handle: 'fuelling-agent',
    personal: true,
  });

  it("names the personal thread after its agent as the athlete's agent list spells it", () => {
    renderHeader(fuelling, null, 'Agent Ravitaillement');

    expect(screen.getByTestId('chat-title')).toHaveTextContent('Agent Ravitaillement');
    expect(screen.getByTestId('chat-header-handle')).toHaveTextContent('@fuelling-agent');
  });

  it('keeps the stored title while the agent list has not answered', () => {
    renderHeader(fuelling, null, null);

    expect(screen.getByTestId('chat-title')).toHaveTextContent('Fuelling Agent');
  });

  it('keeps the stored title of a thread that is not the personal one', () => {
    renderHeader(conversation({ title: 'Renamed run', agent_id: 'agent-fuel' }), null, 'Agent Ravitaillement');

    expect(screen.getByTestId('chat-title')).toHaveTextContent('Renamed run');
  });

  it('names the verdict sheet the same way the header does', () => {
    const t = (key: string) => key;
    expect(threadTitle(fuelling, t, 'Agent Ravitaillement')).toBe('Agent Ravitaillement');
    expect(threadTitle(conversation({ title: '  ', personal: true }), t, null)).toBe('app.untitledChat');
  });
});
