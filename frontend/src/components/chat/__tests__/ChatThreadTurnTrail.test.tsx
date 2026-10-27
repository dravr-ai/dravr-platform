// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A long turn in the web thread — its live step trail, the line it folds into, a reload mid-turn and the tab's mark
// ABOUTME: Driven through the real hook and ChatThread with the API mocked, so the trail, the restore poll and the stop meet the transcript

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import { AWAITING_REPLY_POLL_MS, TURN_STEP_KEYS } from '@pierre/chat-utils';
import type { Message, TurnProgress } from '@pierre/shared-types';
import ChatThread from '../ChatThread';
import { useChatThread, type OpenThread } from '../useChatThread';
import type { PendingComposerAction } from '../types';
import { ToastProvider } from '../../ui';

const CONVERSATION_ID = 'conv-1';

const getConversations = vi.fn();
const getConversationMessages = vi.fn();
const sendTurn = vi.fn();
const stopTurn = vi.fn();

vi.mock('../../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    getConversationMessages: (...a: unknown[]) => getConversationMessages(...a),
    getConversationVerdicts: vi.fn().mockResolvedValue({ verdicts: [] }),
    sendTurn: (...a: unknown[]) => sendTurn(...a),
    stopTurn: (...a: unknown[]) => stopTurn(...a),
    markConversationRead: vi.fn().mockResolvedValue(undefined),
  },
  coachesApi: { list: vi.fn().mockResolvedValue({ agents: [] }) },
  providersApi: {
    getProvidersStatus: vi.fn().mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] }),
  },
  groupsApi: { getGroup: vi.fn(), readRoom: vi.fn() },
}));

vi.mock('../../../services/analytics', () => ({ track: vi.fn() }));
vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { id: 'viewer-1', role: 'user' } }),
}));
vi.mock('../../../hooks/useUsageStatus', () => ({
  useUsageStatus: () => ({
    level: 'none',
    sendDisabled: false,
    text: null,
    invalidate: vi.fn(),
    applyNotice: vi.fn(),
  }),
}));

const EARLIER: Message[] = [
  { id: 'm1', role: 'user', content: 'Hi', created_at: '2026-09-30T08:00:00Z' },
  { id: 'm2', role: 'assistant', content: 'Hi, ready when you are.', created_at: '2026-09-30T08:00:04Z' },
];
const QUESTION: Message = { id: 'q1', role: 'user', content: 'Build Léa’s plan', created_at: '2026-09-30T08:01:00Z' };
const REPLY: Message = { id: 'r1', role: 'assistant', content: 'Léa’s plan is saved.', created_at: '2026-09-30T08:09:00Z' };

type TurnOptions = {
  onDone?: (turn: unknown) => void;
  onProgress?: (progress: TurnProgress) => void;
  onDelta?: (delta: string) => void;
  onRetract?: () => void;
};

/** A turn that stays in flight until the test ends it, with its progress sink exposed. */
function heldTurn() {
  let options: TurnOptions = {};
  let resolveTurn: () => void = () => undefined;
  sendTurn.mockImplementationOnce(
    (_id: string, _content: string, turnOptions: TurnOptions) =>
      new Promise<void>(resolve => {
        options = turnOptions;
        resolveTurn = resolve;
      }),
  );
  return {
    progress: (frame: TurnProgress) => options.onProgress?.(frame),
    delta: (text: string) => options.onDelta?.(text),
    retract: () => options.onRetract?.(),
    finish: (turn: unknown) => {
      options.onDone?.(turn);
      resolveTurn();
    },
  };
}

function turnEnding(content: string, toolsCalled: string[]) {
  return {
    assistant: {
      message: { id: 'm4', role: 'assistant', content, created_at: '2026-09-30T08:01:00Z' },
      blocks: [{ type: 'prose', text: content }],
      finish_reason: 'stop',
    },
    telemetry: { model: 'test-model', execution_time_ms: 10, tools_called: toolsCalled, tool_calls_count: toolsCalled.length },
  };
}

function Host({ action }: { action?: PendingComposerAction }) {
  const [conversationId, setConversationId] = useState<string | null>(CONVERSATION_ID);
  const [pending, setPending] = useState<PendingComposerAction | null>(action ?? null);
  const thread = useChatThread({
    conversationId,
    onSelectConversation: setConversationId,
    openThread: vi.fn<OpenThread>(),
    pendingComposerAction: pending,
    onPendingComposerActionConsumed: () => setPending(null),
  });
  return <ChatThread thread={thread} conversationTitle={thread.title} followOnOpen focusComposerOnOpen={false} />;
}

function renderThread(action?: PendingComposerAction) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <Host action={action} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

function composer(): HTMLTextAreaElement {
  const field = document.querySelector<HTMLTextAreaElement>('textarea[data-composer="true"]');
  if (!field) throw new Error('no composer on screen');
  return field;
}

function setVisibility(state: DocumentVisibilityState) {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
  document.dispatchEvent(new Event('visibilitychange'));
}

describe('a long turn in the web thread', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Plan for Léa', agent_id: null }],
      total: 1,
    });
    getConversationMessages.mockResolvedValue({ messages: EARLIER, awaiting_reply: null });
  });

  afterEach(() => {
    vi.useRealTimers();
    setVisibility('visible');
  });

  it('lists finished steps with a check, names the current one, and never shows a tool’s own name', async () => {
    const turn = heldTurn();
    const user = userEvent.setup();
    renderThread();

    await screen.findByText('Hi, ready when you are.');
    await user.type(composer(), 'Build Léa’s plan');
    await user.click(screen.getByRole('button', { name: i18n.t('chat.sendMessageAria') }));

    const current = await screen.findByTestId('turn-current-step');
    expect(current).toHaveTextContent(i18n.t(TURN_STEP_KEYS.thinking));

    act(() => turn.progress({ kind: 'tool', id: 'c1', title: 'get_group_member_activities', status: 'InProgress' }));
    expect(screen.getByTestId('turn-current-step')).toHaveTextContent(i18n.t(TURN_STEP_KEYS.readMemberActivities));

    act(() => turn.progress({ kind: 'tool', id: 'c1', title: 'get_group_member_activities', status: 'Completed' }));
    // Between tools the line is the agent analysing, not a reset to the first line.
    expect(screen.getByTestId('turn-current-step')).toHaveTextContent(i18n.t(TURN_STEP_KEYS.analysing));
    expect(screen.getAllByTestId('turn-step-done')).toHaveLength(1);

    act(() => turn.progress({ kind: 'tool', id: 'c2', title: 'save_training_plan', status: 'InProgress' }));
    expect(screen.getByTestId('turn-current-step')).toHaveTextContent(i18n.t(TURN_STEP_KEYS.savePlan));
    expect(screen.queryByText(/get_group_member_activities|save_training_plan/)).not.toBeInTheDocument();
    // The stop button stands for the whole turn.
    expect(screen.getByTestId('stop-turn-button')).toBeInTheDocument();

    await act(async () => {
      turn.finish(turnEnding('Léa’s plan is saved.', ['get_group_member_activities', 'save_training_plan']));
    });
    await screen.findByText('Léa’s plan is saved.');
    expect(screen.queryByTestId('turn-indicator')).not.toBeInTheDocument();

    // The trail folds into one line above the reply, and opens into its steps.
    const folded = screen.getByTestId('turn-trail-folded');
    const toggle = within(folded).getByRole('button');
    expect(toggle).toHaveTextContent(/2/);
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await user.click(toggle);
    const steps = within(folded).getByTestId('turn-trail-steps');
    expect(within(steps).getAllByRole('listitem')).toHaveLength(2);
    expect(steps).toHaveTextContent(i18n.t(TURN_STEP_KEYS.readMemberActivities));
    expect(steps).toHaveTextContent(i18n.t(TURN_STEP_KEYS.savePlan));
  });

  it('empties the streaming bubble back to thinking when its words are taken back', async () => {
    const turn = heldTurn();
    const user = userEvent.setup();
    renderThread();

    await screen.findByText('Hi, ready when you are.');
    await user.type(composer(), 'How was my week?');
    await user.click(screen.getByRole('button', { name: i18n.t('chat.sendMessageAria') }));
    await screen.findByTestId('turn-current-step');

    act(() => {
      turn.delta('Let me look ');
      turn.delta('at your runs.');
    });
    expect(await screen.findByText('Let me look at your runs.')).toBeInTheDocument();

    // A tool call followed those words: they are not the reply.
    act(() => turn.retract());
    expect(screen.queryByText(/Let me look/)).not.toBeInTheDocument();
    expect(screen.getByTestId('turn-current-step')).toHaveTextContent(i18n.t(TURN_STEP_KEYS.thinking));

    // The reply streams into an empty bubble, never glued to what was taken back.
    act(() => turn.delta('Three easy runs this week.'));
    expect(await screen.findByText('Three easy runs this week.')).toBeInTheDocument();
    expect(screen.queryByText(/Let me look/)).not.toBeInTheDocument();

    await act(async () => {
      turn.finish(turnEnding('Three easy runs this week.', ['get_activities']));
    });
    await waitFor(() => expect(screen.queryByTestId('turn-indicator')).not.toBeInTheDocument());
    expect(screen.getAllByText('Three easy runs this week.')).toHaveLength(1);
  });

  it('marks a hidden tab when the reply lands, and clears the mark when the tab is shown', async () => {
    document.title = 'Dravr';
    const turn = heldTurn();
    const user = userEvent.setup();
    renderThread();

    await screen.findByText('Hi, ready when you are.');
    await user.type(composer(), 'Build Léa’s plan');
    await user.click(screen.getByRole('button', { name: i18n.t('chat.sendMessageAria') }));

    setVisibility('hidden');
    await act(async () => {
      turn.finish(turnEnding('Léa’s plan is saved.', []));
    });
    await screen.findByText('Léa’s plan is saved.');
    expect(document.title).toBe('(1) Dravr');

    act(() => setVisibility('visible'));
    expect(document.title).toBe('Dravr');
  });

  it('restores the indicator after a reload, polls until the reply lands, then stops polling', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const askedAt = new Date(Date.now() - 60_000).toISOString();
    getConversationMessages.mockResolvedValue({
      messages: [...EARLIER, { ...QUESTION, created_at: askedAt }],
      awaiting_reply: { question_id: QUESTION.id, asked_at: askedAt },
    });
    renderThread();

    const indicator = await screen.findByTestId('turn-indicator');
    // Timed from the question, with the note that leaving is safe; no steps are known.
    expect(within(indicator).getByTestId('turn-elapsed')).toHaveTextContent(/1/);
    expect(within(indicator).getByTestId('turn-safe-to-leave')).toBeInTheDocument();
    expect(within(indicator).queryByTestId('turn-step-done')).not.toBeInTheDocument();
    expect(screen.getByTestId('stop-turn-button')).toBeInTheDocument();

    const callsBefore = getConversationMessages.mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AWAITING_REPLY_POLL_MS + 100);
    });
    expect(getConversationMessages.mock.calls.length).toBeGreaterThan(callsBefore);

    getConversationMessages.mockResolvedValue({
      messages: [...EARLIER, { ...QUESTION, created_at: askedAt }, REPLY],
      awaiting_reply: null,
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AWAITING_REPLY_POLL_MS + 100);
    });
    await screen.findByText(REPLY.content);
    expect(screen.queryByTestId('turn-indicator')).not.toBeInTheDocument();
    expect(screen.queryByTestId('stop-turn-button')).not.toBeInTheDocument();

    const callsAfterReply = getConversationMessages.mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AWAITING_REPLY_POLL_MS * 3);
    });
    expect(getConversationMessages.mock.calls.length).toBe(callsAfterReply);
  });

  it('stops a restored turn on the thread the server is still answering', async () => {
    const askedAt = new Date(Date.now() - 5_000).toISOString();
    getConversationMessages.mockResolvedValue({
      messages: [...EARLIER, { ...QUESTION, created_at: askedAt }],
      awaiting_reply: { question_id: QUESTION.id, asked_at: askedAt },
    });
    stopTurn.mockResolvedValue(true);
    const user = userEvent.setup();
    renderThread();

    await screen.findByTestId('turn-indicator');
    // Under ten seconds: no timer yet, and no note.
    expect(screen.queryByTestId('turn-elapsed')).not.toBeInTheDocument();
    expect(screen.queryByTestId('turn-safe-to-leave')).not.toBeInTheDocument();

    const callsBefore = getConversationMessages.mock.calls.length;
    getConversationMessages.mockResolvedValue({
      messages: [...EARLIER, { ...QUESTION, created_at: askedAt }, { ...REPLY, content: 'Stopped.' }],
      awaiting_reply: null,
    });
    await user.click(screen.getByTestId('stop-turn-button'));
    expect(stopTurn).toHaveBeenCalledWith(CONVERSATION_ID);
    await waitFor(() => expect(getConversationMessages.mock.calls.length).toBeGreaterThan(callsBefore));
    await screen.findByText('Stopped.');
    expect(screen.queryByTestId('turn-indicator')).not.toBeInTheDocument();
    expect(await screen.findByRole('button', { name: i18n.t('chat.sendMessageAria') })).toBeInTheDocument();
  });

  it('ignores a stale awaiting flag once the reply is already in the thread', async () => {
    getConversationMessages.mockResolvedValue({
      messages: [...EARLIER, QUESTION, REPLY],
      awaiting_reply: { question_id: QUESTION.id, asked_at: QUESTION.created_at },
    });
    renderThread();

    await screen.findByText(REPLY.content);
    expect(screen.queryByTestId('turn-indicator')).not.toBeInTheDocument();
  });
  it('holds a deep-link send during a restored turn and sends it once the reply lands', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const askedAt = new Date(Date.now() - 20_000).toISOString();
    getConversationMessages.mockResolvedValue({
      messages: [...EARLIER, { ...QUESTION, created_at: askedAt }],
      awaiting_reply: { question_id: QUESTION.id, asked_at: askedAt },
    });
    sendTurn.mockResolvedValue(undefined);
    renderThread({ kind: 'send', text: '/group join ABC123' });

    await screen.findByTestId('turn-indicator');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AWAITING_REPLY_POLL_MS + 100);
    });
    // Held, not sent and not dropped, while the server is still answering.
    expect(sendTurn).not.toHaveBeenCalled();

    getConversationMessages.mockResolvedValue({
      messages: [...EARLIER, { ...QUESTION, created_at: askedAt }, REPLY],
      awaiting_reply: null,
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AWAITING_REPLY_POLL_MS + 100);
    });
    await screen.findByText(REPLY.content);
    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(1));
    expect(sendTurn.mock.calls[0][0]).toBe(CONVERSATION_ID);
    expect(sendTurn.mock.calls[0][1]).toBe('/group join ABC123');
  });

  it('blocks a typed send during a restored turn the way it does while a turn streams', async () => {
    const askedAt = new Date(Date.now() - 20_000).toISOString();
    getConversationMessages.mockResolvedValue({
      messages: [...EARLIER, { ...QUESTION, created_at: askedAt }],
      awaiting_reply: { question_id: QUESTION.id, asked_at: askedAt },
    });
    const user = userEvent.setup();
    renderThread();

    await screen.findByTestId('turn-indicator');
    // The same composer a streaming turn leaves: the field is disabled and
    // send has become stop.
    expect(composer()).toBeDisabled();
    expect(screen.queryByRole('button', { name: i18n.t('chat.sendMessageAria') })).not.toBeInTheDocument();
    expect(screen.getByTestId('stop-turn-button')).toBeInTheDocument();
    await user.type(composer(), 'One more thing{Enter}');
    expect(composer()).toHaveValue('');
    expect(sendTurn).not.toHaveBeenCalled();
  });
});
