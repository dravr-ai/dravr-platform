// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's plan thread on the phone — the open draft drawn whenever no card in the thread shows it, and the one-tap start
// ABOUTME: Also pins that a composer whose thread is not yet read shows no placeholder, never the athlete's words

import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react-native';
import type { Message, ReplyBlock, WorkoutPlan } from '@pierre/shared-types';
import type { PlanThreadDraft } from '@pierre/ui-logic';
import { ChatThread, type ChatThreadProps } from '../src/screens/chat/ChatThread';

jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn() }));
// The transcript, reduced to the plan thread's slots: the draft and the start.
jest.mock('../src/screens/chat/MessageList', () => {
  const React = require('react');
  const { Text, View } = require('react-native');
  return {
    MessageList: ({
      planDraft,
      planStarter,
    }: {
      planDraft?: React.ReactNode;
      planStarter?: { label: string; onPress: () => void; disabled: boolean } | null;
    }) =>
      React.createElement(
        View,
        null,
        planDraft ?? null,
        planStarter
          ? React.createElement(
              Text,
              { testID: 'starter', onPress: planStarter.disabled ? undefined : planStarter.onPress },
              planStarter.label,
            )
          : null,
      ),
  };
});
// The composer, reduced to the placeholder it is handed.
jest.mock('../src/screens/chat/ChatInputBar', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    ChatInputBar: ({ placeholder }: { placeholder?: string }) =>
      React.createElement(Text, { testID: 'composer-placeholder' }, placeholder === undefined ? '<default>' : `[${placeholder}]`),
  };
});
jest.mock('../src/screens/chat/PlanThreadOpenDraft', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    __esModule: true,
    default: ({ open }: { open: { draft: { draft_id: string } } }) =>
      React.createElement(Text, { testID: 'open-draft' }, open.draft.draft_id),
  };
});
jest.mock('../src/screens/chat/UsageWarningBanner', () => ({ UsageWarningBanner: () => null }));
jest.mock('../src/screens/chat/VerdictSheet', () => ({ VerdictSheet: () => null }));
jest.mock('../src/screens/chat/useChatVoiceInput', () => ({ useChatVoiceInput: () => ({}) }));
jest.mock('../src/components/ProviderNotice', () => ({ ProviderNoticeSheet: () => null }));
jest.mock('../src/components/OAuthCredentialsSection', () => ({ OAuthCredentialsSection: () => null }));

const SUBJECT = { user_id: 'user-chloe', display_name: 'Chloé Tremblay' };
const DRAFT = { draft_id: 'draft-7', updated_at: '2026-11-02T14:05:00Z', conflicts: [], season_relaid: false };
const OPEN: PlanThreadDraft = {
  open: { subject: SUBJECT, plan: { title: 'Base block' } as unknown as WorkoutPlan, draft: DRAFT },
  noDraft: false,
};

function row(id: string): Message {
  return { id, role: 'assistant', content: 'About her week.', created_at: '2026-11-02T10:00:00Z' };
}

function cardFor(draftId: string): ReplyBlock[] {
  return [
    {
      type: 'workout_plan',
      plan: { plan_window: { start: '2026-11-03', end: '2026-11-09' } },
      subject: SUBJECT,
      draft: { ...DRAFT, draft_id: draftId },
    } as unknown as ReplyBlock,
  ];
}

interface Setup {
  planDraftRead?: PlanThreadDraft;
  messages?: Message[];
  messageBlocks?: Record<string, ReplyBlock[]>;
  turnInFlight?: boolean;
  composerPending?: boolean;
  planSubjectName?: string | null;
  sendText?: ChatThreadProps['sendText'];
}

function renderThread({
  planDraftRead,
  messages = [],
  messageBlocks = {},
  turnInFlight = false,
  composerPending = false,
  planSubjectName = 'Chloé Tremblay',
  sendText = jest.fn().mockResolvedValue(undefined),
}: Setup) {
  // Only what the thread reads itself; every child that reads more is mocked.
  const messagesHook = {
    scrollToBottom: jest.fn(),
    quotaNotice: null,
    verdicts: [],
    verdictsLoading: false,
    refreshVerdicts: jest.fn(),
    messages,
    messageBlocks,
    turnInFlight,
  } as unknown as ChatThreadProps['messagesHook'];
  const usageStatus = { applyNotice: jest.fn(), sendDisabled: false } as unknown as ChatThreadProps['usageStatus'];
  const providerStatus = {
    connectedProviders: [],
    setNeedsCredentialsProvider: jest.fn(),
    loadProviderStatus: jest.fn(),
    needsCredentialsProvider: null,
  } as unknown as ChatThreadProps['providerStatus'];
  render(
    <ChatThread
      conversationId="conv-plan"
      messagesHook={messagesHook}
      usageStatus={usageStatus}
      providerStatus={providerStatus}
      isLoading={false}
      inputText=""
      onChangeInputText={jest.fn()}
      inputRef={React.createRef()}
      sendText={sendText}
      planSubjectName={planSubjectName}
      planDraftRead={planDraftRead}
      composerPending={composerPending}
    />,
  );
  return { sendText };
}

describe('ChatThread — the coach plan thread', () => {
  it('draws the open draft under a thread with messages none of which shows it', () => {
    renderThread({ planDraftRead: OPEN, messages: [row('m1'), row('m2')] });
    expect(screen.getByTestId('open-draft')).toHaveTextContent('draft-7');
  });

  it('draws it once when no card shows it, and not again when a row already does', () => {
    renderThread({ planDraftRead: OPEN, messages: [row('m1')], messageBlocks: { m1: cardFor('draft-7') } });
    expect(screen.queryByTestId('open-draft')).toBeNull();
  });

  it('is not hidden by a card left from a draft since discarded', () => {
    renderThread({ planDraftRead: OPEN, messages: [row('m1')], messageBlocks: { m1: cardFor('draft-old') } });
    expect(screen.getByTestId('open-draft')).toHaveTextContent('draft-7');
  });

  it('offers no start while a draft is open or the read is out', () => {
    renderThread({ planDraftRead: OPEN });
    expect(screen.queryByTestId('starter')).toBeNull();
  });

  it("sends the plan request in one tap when the coach holds no draft, under the athlete's name", () => {
    const { sendText } = renderThread({ planDraftRead: { open: null, noDraft: true } });
    const starter = screen.getByTestId('starter');
    expect(starter).toHaveTextContent(/Chloé Tremblay/);
    fireEvent.press(starter);
    expect(sendText).toHaveBeenCalledTimes(1);
    const [text] = (sendText as jest.Mock).mock.calls[0] as [string];
    expect(text).toMatch(/Chloé Tremblay/);
    expect(text).not.toBe(String(starter.props.children));
  });

  it('holds the start while a turn is running', () => {
    const { sendText } = renderThread({ planDraftRead: { open: null, noDraft: true }, turnInFlight: true });
    fireEvent.press(screen.getByTestId('starter'));
    expect(sendText).not.toHaveBeenCalled();
  });
});

describe('ChatThread — a composer whose thread is not yet read', () => {
  it('shows no placeholder at all, never the athlete one', () => {
    renderThread({ planSubjectName: null, composerPending: true });
    expect(screen.getByTestId('composer-placeholder')).toHaveTextContent('[]');
  });

  it("keeps the app's own words once it is known to be an athlete thread", () => {
    renderThread({ planSubjectName: null, composerPending: false });
    expect(screen.getByTestId('composer-placeholder')).toHaveTextContent('<default>');
  });
});
