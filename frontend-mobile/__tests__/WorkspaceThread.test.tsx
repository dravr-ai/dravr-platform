// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete workspace's thread host on the phone — reads its conversation with the room woven in, sends into it, and sends a handed line once read (carnet#1818)
// ABOUTME: The chat screen's ChatThread is a stand-in here; what is pinned is what the host reads, sends and hands it

import React from 'react';
import { act, render, screen, waitFor } from '@testing-library/react-native';
import type { TurnSendOptions } from '@pierre/shared-types';

const mockLoadMessages = jest.fn();
const mockSendTurn = jest.fn();
const mockInvalidateUsage = jest.fn();
const mockMarkRead = jest.fn();
let mockMessages: { id: string; room?: unknown }[] = [];

jest.mock('../src/screens/chat/useMessages', () => ({
  useMessages: () => ({
    messages: mockMessages,
    isSending: false,
    loadMessages: mockLoadMessages,
    sendTurn: mockSendTurn,
  }),
}));
jest.mock('../src/screens/chat/useUsageStatus', () => ({
  useUsageStatus: () => ({ invalidate: mockInvalidateUsage }),
}));
jest.mock('../src/screens/chat/useProviderStatus', () => ({
  useProviderStatus: () => ({ loadProviderStatus: jest.fn().mockResolvedValue(undefined) }),
}));
jest.mock('../src/screens/chat/useMarkConversationRead', () => ({
  useMarkConversationRead: (options: unknown) => mockMarkRead(options),
}));
jest.mock('../src/hooks/useCoachPlan', () => ({
  usePlanThreadDraft: (conversationId: string, enabled: boolean) => ({ open: null, noDraft: enabled && conversationId !== '' }),
}));
jest.mock('../src/services/analytics', () => ({ trackMobile: jest.fn() }));

/** The ChatThread the host hands its conversation to, as the last render passed it. */
let mockThreadProps: {
  conversationId: string;
  isLoading: boolean;
  sendText: (text: string, options?: TurnSendOptions) => Promise<void>;
  planSubjectName?: string | null;
  coachRoomName?: string | null;
  planSurface?: { kind: string } | null;
  planDraftRead?: { noDraft: boolean };
} | null = null;
jest.mock('../src/screens/chat/ChatThread', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    ChatThread: (props: NonNullable<typeof mockThreadProps>) => {
      mockThreadProps = props;
      return React.createElement(Text, { testID: 'chat-thread-stub' }, props.isLoading ? 'loading' : 'read');
    },
  };
});

import { WorkspaceThread } from '../src/screens/coach-home/workspace/WorkspaceThread';

const PLAN_ID = 'conv-plan-lea';
const ROOM_ID = 'conv-room-lea';

beforeEach(() => {
  jest.clearAllMocks();
  mockThreadProps = null;
  mockMessages = [];
  mockLoadMessages.mockResolvedValue(undefined);
  mockSendTurn.mockResolvedValue(undefined);
});

describe('the workspace thread', () => {
  it("reads the room with its group's rows woven in, and is loading until it has", async () => {
    let finish: () => void = () => undefined;
    mockLoadMessages.mockReturnValue(new Promise<void>((resolve) => (finish = resolve)));
    render(<WorkspaceThread conversationId={ROOM_ID} groupId="g-lea" conversationTitle="TRAKA" coachRoomName="TRAKA" />);

    expect(mockLoadMessages).toHaveBeenCalledWith(ROOM_ID, 'g-lea');
    expect(screen.getByTestId('chat-thread-stub')).toHaveTextContent('loading');
    // Nothing is marked read before the transcript is.
    expect(mockMarkRead).toHaveBeenLastCalledWith({ conversationId: null, lastMessageId: null });

    await act(async () => finish());
    expect(screen.getByTestId('chat-thread-stub')).toHaveTextContent('read');
    expect(mockThreadProps?.coachRoomName).toBe('TRAKA');
    expect(mockThreadProps?.planSubjectName).toBeNull();
    expect(mockMarkRead).toHaveBeenLastCalledWith({ conversationId: ROOM_ID, lastMessageId: null });
  });

  it("moves the read marker to the coach's own newest row, never a room row", async () => {
    mockMessages = [{ id: 'own-1' }, { id: 'room-2', room: { author: 'Léa' } }];
    render(<WorkspaceThread conversationId={ROOM_ID} groupId="g-lea" conversationTitle="TRAKA" />);

    await waitFor(() => expect(mockMarkRead).toHaveBeenLastCalledWith({ conversationId: ROOM_ID, lastMessageId: 'own-1' }));
  });

  it('sends a typed line into its own conversation, trimmed, and refreshes the usage after', async () => {
    render(<WorkspaceThread conversationId={PLAN_ID} groupId={null} conversationTitle="Coach Max" planSubjectName="Léa Gagnon" />);
    await waitFor(() => expect(screen.getByTestId('chat-thread-stub')).toHaveTextContent('read'));

    await act(async () => mockThreadProps?.sendText('  Make Thursday easier  ', { origin: 'chip' }));

    expect(mockLoadMessages).toHaveBeenCalledWith(PLAN_ID, null);
    expect(mockSendTurn).toHaveBeenCalledWith(PLAN_ID, 'Make Thursday easier', { origin: 'chip' });
    expect(mockInvalidateUsage).toHaveBeenCalledTimes(1);
    // The plan thread's draft read is made for a thread about an athlete.
    expect(mockThreadProps?.planDraftRead).toEqual({ open: null, noDraft: true });
  });

  it('sends nothing for a blank line', async () => {
    render(<WorkspaceThread conversationId={PLAN_ID} groupId={null} conversationTitle="Coach Max" />);
    await waitFor(() => expect(screen.getByTestId('chat-thread-stub')).toHaveTextContent('read'));

    await act(async () => mockThreadProps?.sendText('   '));

    expect(mockSendTurn).not.toHaveBeenCalled();
  });

  it('sends a line the workspace hands it once, after the thread is read, and says it took it', async () => {
    let finish: () => void = () => undefined;
    mockLoadMessages.mockReturnValue(new Promise<void>((resolve) => (finish = resolve)));
    const consumed = jest.fn();
    const { rerender } = render(
      <WorkspaceThread
        conversationId={PLAN_ID}
        groupId={null}
        conversationTitle="Coach Max"
        planSubjectName="Léa Gagnon"
        planSurface={{ kind: 'workspace', onShowCalendar: jest.fn() }}
        pendingSend="Draft a training plan for Léa Gagnon."
        onPendingSendConsumed={consumed}
      />,
    );
    expect(mockSendTurn).not.toHaveBeenCalled();
    expect(mockThreadProps?.planSurface?.kind).toBe('workspace');

    await act(async () => finish());
    expect(mockSendTurn).toHaveBeenCalledWith(PLAN_ID, 'Draft a training plan for Léa Gagnon.', undefined);
    expect(consumed).toHaveBeenCalledTimes(1);

    // The workspace clears the line it handed: nothing is sent a second time.
    rerender(
      <WorkspaceThread
        conversationId={PLAN_ID}
        groupId={null}
        conversationTitle="Coach Max"
        planSubjectName="Léa Gagnon"
        pendingSend={null}
        onPendingSendConsumed={consumed}
      />,
    );
    expect(mockSendTurn).toHaveBeenCalledTimes(1);
  });
});
