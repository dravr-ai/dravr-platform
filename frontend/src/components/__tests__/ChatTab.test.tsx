// ABOUTME: Tests for ChatTab's assistant author label and coach-form request building
// ABOUTME: Covers the coach-titled assistant bubble and the keep/clear/set tool budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReplyBlock, TurnEnvelope } from '@pierre/shared-types';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ChatTab from '../ChatTab';
import { formDataToCreateRequest, formDataToUpdateRequest } from '../chat/coachForm';
import { ToastProvider } from '../ui';
import { DEFAULT_COACH_FORM_DATA, type CoachFormData } from '../chat';

const CONVERSATION_ID = 'conv-1';
const COACH_ID = 'coach-1';
const COACH_TITLE = 'Marathon Coach';

const getConversations = vi.fn();
const getConversationMessages = vi.fn();
const getConversationVerdicts = vi.fn();
const sendTurn = vi.fn();
const listCoaches = vi.fn();
const getProvidersStatus = vi.fn();
const applyNotice = vi.fn();

vi.mock('../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    getConversationMessages: (...a: unknown[]) => getConversationMessages(...a),
    getConversationVerdicts: (...a: unknown[]) => getConversationVerdicts(...a),
    sendTurn: (...a: unknown[]) => sendTurn(...a),
  },
  coachesApi: { list: (...a: unknown[]) => listCoaches(...a) },
  providersApi: { getProvidersStatus: (...a: unknown[]) => getProvidersStatus(...a) },
  oauthApi: {},
}));

vi.mock('../../services/analytics', () => ({ track: vi.fn() }));
vi.mock('../../hooks/useAuth', () => ({ useAuth: () => ({ token: 'test-token' }) }));
vi.mock('../../hooks/useUsageStatus', () => ({
  useUsageStatus: () => ({
    data: undefined,
    isLoading: false,
    error: null,
    level: 'none',
    sendDisabled: false,
    message: '',
    resetsAt: '',
    triggerCounter: null,
    invalidate: vi.fn(),
    applyNotice: (...a: unknown[]) => applyNotice(...a),
  }),
}));

function renderChatTab() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <ChatTab selectedConversation={CONVERSATION_ID} onSelectConversation={vi.fn()} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe('ChatTab assistant author label', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getProvidersStatus.mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] });
    getConversationVerdicts.mockResolvedValue({ verdicts: [] });
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Sunday long run', coach_id: COACH_ID }],
      total: 1,
    });
    listCoaches.mockResolvedValue({ coaches: [{ id: COACH_ID, title: COACH_TITLE }] });
    getConversationMessages.mockResolvedValue({
      messages: [
        {
          id: 'm1',
          role: 'user',
          content: 'How was my long run?',
          created_at: '2026-08-23T10:00:00Z',
        },
        {
          id: 'm2',
          role: 'assistant',
          content: 'Your aerobic decoupling held under 5%.',
          created_at: '2026-08-23T10:00:05Z',
        },
      ],
    });
  });

  // The assistant avatar's alt text IS the author label (MessageItem renders
  // `alt={assistantLabel ?? 'Dravr'}`), and the conversation header's own icon
  // is decorative with alt="". So an alt-text query resolves to exactly the
  // assistant turn — the header showing the same title cannot satisfy it.
  it('labels the assistant turn with the coach title, not the literal Dravr', async () => {
    renderChatTab();

    const avatar = await screen.findByAltText(COACH_TITLE);
    expect(avatar).toBeInTheDocument();
    expect(screen.queryByAltText('Dravr')).toBeNull();
  });

  it('renders the coach title as the visible author name on the assistant turn', async () => {
    renderChatTab();

    const avatar = await screen.findByAltText(COACH_TITLE);
    // MessageItem: <div flex gap-3><div avatar><img/></div><div body><div>LABEL</div>…
    const turn = avatar.parentElement?.parentElement as HTMLElement;
    expect(within(turn).getByText(COACH_TITLE)).toBeInTheDocument();
    expect(within(turn).getByText('Your aerobic decoupling held under 5%.')).toBeInTheDocument();
  });

  it('falls back to Dravr when the conversation has no coach attached', async () => {
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Sunday long run', coach_id: null }],
      total: 1,
    });

    renderChatTab();

    await waitFor(() => expect(screen.getByAltText('Dravr')).toBeInTheDocument());
    expect(screen.queryByAltText(COACH_TITLE)).toBeNull();
  });
});

describe('coach form → API request tool budget', () => {
  function filledForm(overrides: Partial<CoachFormData> = {}): CoachFormData {
    return {
      ...DEFAULT_COACH_FORM_DATA,
      title: COACH_TITLE,
      system_prompt: 'You are an expert marathon coach.',
      ...overrides,
    };
  }

  it('omits max_tool_iterations from a create request when the field is untouched', () => {
    const request = formDataToCreateRequest(filledForm());

    expect(request.max_tool_iterations).toBeUndefined();
    expect('max_tool_iterations' in request).toBe(false);
    expect(request.title).toBe(COACH_TITLE);
  });

  it('sends max_tool_iterations on a create request when the user entered one', () => {
    const request = formDataToCreateRequest(filledForm({ max_tool_iterations: 25 }));

    expect(request.max_tool_iterations).toBe(25);
  });

  it('omits max_tool_iterations from an update request when the field is untouched', () => {
    const request = formDataToUpdateRequest(filledForm());

    expect(request.max_tool_iterations).toBeUndefined();
    expect('max_tool_iterations' in request).toBe(false);
  });

  it('sends max_tool_iterations on an update request when the user entered one', () => {
    const request = formDataToUpdateRequest(filledForm({ max_tool_iterations: 3 }));

    expect(request.max_tool_iterations).toBe(3);
  });

  it('sends an explicit null on an update request when the user cleared the box', () => {
    const request = formDataToUpdateRequest(filledForm({ max_tool_iterations: null }));

    // The key has to be PRESENT and null. Merely leaving it out is what the
    // untouched state does, and the server preserves an absent field — so an
    // omitted key would leave the coach's existing pin in place forever.
    expect('max_tool_iterations' in request).toBe(true);
    expect(request.max_tool_iterations).toBeNull();
  });

  it('omits a cleared max_tool_iterations from a create request', () => {
    const request = formDataToCreateRequest(filledForm({ max_tool_iterations: null }));

    // A coach that does not exist yet has no pin to clear, so "cleared" and
    // "untouched" are the same request: inherit.
    expect('max_tool_iterations' in request).toBe(false);
  });
});

/** A finished turn, shaped exactly as `turn_response.rs` serializes it. */
function turnEnvelope(): TurnEnvelope {
  return {
    turn_id: 'turn-1',
    user_message: {
      id: 'm3',
      role: 'user',
      content: 'Connect me.',
      created_at: '2026-08-23T10:01:00Z',
    },
    assistant: {
      message: {
        id: 'm4',
        role: 'assistant',
        content: 'Here is the link.',
        created_at: '2026-08-23T10:01:02Z',
      },
      blocks: [],
      finish_reason: 'command',
    },
    conversation_updated_at: '2026-08-23T10:01:02Z',
    telemetry: {
      model: 'gemini-1.5-flash',
      provider_name: 'gemini',
      tool_calls_count: 0,
      tools_called: [],
      execution_time_ms: 120,
    },
  };
}

/** Answer the next send with these blocks, then finish the turn. */
function answerWith(blocks: ReplyBlock[]) {
  sendTurn.mockImplementation(
    async (
      _conversationId: string,
      _content: string,
      options: {
        onBlock?: (block: ReplyBlock) => void;
        onDone?: (turn: TurnEnvelope) => void;
      },
    ) => {
      for (const block of blocks) options.onBlock?.(block);
      options.onDone?.(turnEnvelope());
    },
  );
}

async function send(text: string) {
  const user = userEvent.setup();
  const input = await screen.findByPlaceholderText('Message Dravr...');
  await user.type(input, text);
  await user.click(screen.getByRole('button', { name: 'Send message' }));
  return user;
}

describe('ChatTab url reply actions', () => {
  const ALLOWED = 'https://www.strava.com/oauth/authorize?client_id=1';
  // The open-redirect classic: a host that merely ends with a trusted name.
  const FOREIGN = 'https://strava.com.attacker.example/steal';

  beforeEach(() => {
    vi.clearAllMocks();
    getProvidersStatus.mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] });
    getConversationVerdicts.mockResolvedValue({ verdicts: [] });
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Connections', coach_id: null }],
      total: 1,
    });
    listCoaches.mockResolvedValue({ coaches: [] });
    getConversationMessages.mockResolvedValue({ messages: [] });
  });

  it('opens an allowed-domain action and refuses a foreign one', async () => {
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    answerWith([
      { type: 'prose', text: 'Pick where to connect.' },
      {
        type: 'actions',
        actions: [
          { label: 'Authorize Strava', action_type: 'url', value: ALLOWED },
          { label: 'Authorize elsewhere', action_type: 'url', value: FOREIGN },
        ],
      },
    ]);

    renderChatTab();
    const user = await send('Connect me.');

    await user.click(await screen.findByRole('button', { name: 'Authorize Strava' }));
    expect(open).toHaveBeenCalledTimes(1);
    expect(open).toHaveBeenCalledWith(ALLOWED, '_blank', 'noopener,noreferrer');

    await user.click(screen.getByRole('button', { name: 'Authorize elsewhere' }));
    // Still one: the foreign host opened nothing, and nothing was dispatched
    // back to the coach as a message either.
    expect(open).toHaveBeenCalledTimes(1);
    expect(sendTurn).toHaveBeenCalledTimes(1);

    open.mockRestore();
  });

  it('sends the message verbatim — no [Context: …] prefix is injected any more', async () => {
    // The server states the athlete's connected providers itself in
    // `build_provider_context`. The prefix was a second copy of that fact,
    // persisted verbatim into chat_messages.content and never stripped.
    answerWith([{ type: 'prose', text: 'Here is your week.' }]);

    renderChatTab();
    await send('How was my week?');

    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(1));
    expect(sendTurn.mock.calls[0][1]).toBe('How was my week?');
  });

  it('routes a turn\'s quota notice to the usage banner, with its real counters', async () => {
    answerWith([
      { type: 'prose', text: 'Here is your week.' },
      {
        type: 'notice',
        notice: {
          kind: 'quota_warning',
          level: 'approaching',
          current: 45,
          limit: 50,
          resets_at: '2026-08-26T00:00:00Z',
        },
      },
    ]);

    renderChatTab();
    await send('How was my week?');

    await waitFor(() => expect(applyNotice).toHaveBeenCalledTimes(1));
    expect(applyNotice).toHaveBeenCalledWith({
      kind: 'quota_warning',
      level: 'approaching',
      current: 45,
      limit: 50,
      resets_at: '2026-08-26T00:00:00Z',
    });
  });
});
