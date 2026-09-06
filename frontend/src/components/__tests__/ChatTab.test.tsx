// ABOUTME: Tests for ChatTab's author label, its header info drawer and its url reply actions
// ABOUTME: Covers the coach-titled bubble, the header-as-button contract, and the trusted-domain gate
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ClaimVerdict, ReplyBlock, TurnEnvelope } from '@pierre/shared-types';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ChatTab from '../ChatTab';
import { ToastProvider } from '../ui';

const CONVERSATION_ID = 'conv-1';
const COACH_ID = 'coach-1';
const COACH_TITLE = 'Marathon Coach';

const getConversations = vi.fn();
const getConversationMessages = vi.fn();
const getConversationVerdicts = vi.fn();
const listParticipants = vi.fn();
const sendTurn = vi.fn();
const listCoaches = vi.fn();
const getProvidersStatus = vi.fn();
const applyNotice = vi.fn();

vi.mock('../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    getConversationMessages: (...a: unknown[]) => getConversationMessages(...a),
    getConversationVerdicts: (...a: unknown[]) => getConversationVerdicts(...a),
    listParticipants: (...a: unknown[]) => listParticipants(...a),
    sendTurn: (...a: unknown[]) => sendTurn(...a),
    markConversationRead: vi.fn().mockResolvedValue(undefined),
    createConversation: vi.fn(),
    updateConversation: vi.fn(),
    deleteConversation: vi.fn(),
    markConversationUnread: vi.fn(),
  },
  coachesApi: { list: (...a: unknown[]) => listCoaches(...a) },
  providersApi: { getProvidersStatus: (...a: unknown[]) => getProvidersStatus(...a) },
  groupsApi: {},
}));

vi.mock('../groups/GroupInfoPanel', () => ({
  default: ({ groupId }: { groupId: string }) => (
    <div data-testid="group-info-panel">group {groupId}</div>
  ),
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

function renderChatTab(
  props: { onNavigate?: (route: string) => void; onSelectConversation?: (id: string | null) => void } = {},
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <ChatTab
          selectedConversation={CONVERSATION_ID}
          onSelectConversation={props.onSelectConversation ?? vi.fn()}
          onNavigate={props.onNavigate}
        />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe('ChatTab assistant author label', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getProvidersStatus.mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] });
    getConversationVerdicts.mockResolvedValue({ verdicts: [] });
    listParticipants.mockResolvedValue([]);
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

  // The assistant avatar's accessible name IS the author label (CoachAvatar
  // renders `role="img"` with `aria-label={assistantLabel ?? 'Dravr'}`), and
  // the conversation header's own avatar is aria-hidden. So an image-role query
  // resolves to exactly the assistant turn — the header showing the same title
  // cannot satisfy it.
  it('labels the assistant turn with the coach title, not the literal Dravr', async () => {
    renderChatTab();

    const avatar = await screen.findByRole('img', { name: COACH_TITLE });
    expect(avatar).toBeInTheDocument();
    expect(screen.queryByRole('img', { name: 'Dravr' })).toBeNull();
  });

  it('renders the coach title as the visible author name on the assistant turn', async () => {
    renderChatTab();

    const avatar = await screen.findByRole('img', { name: COACH_TITLE });
    // MessageItem: <div flex gap-3><div avatar><span role=img/></div><div body><div>LABEL</div>…
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

    await waitFor(() => expect(screen.getByRole('img', { name: 'Dravr' })).toBeInTheDocument());
    expect(screen.queryByRole('img', { name: COACH_TITLE })).toBeNull();
  });

  it('names the conversation in the header when no coach is attached', async () => {
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Sunday long run', coach_id: null }],
      total: 1,
    });

    renderChatTab();

    await waitFor(() =>
      expect(screen.getByTestId('conversation-header-title')).toHaveTextContent('Sunday long run'),
    );
  });

  it('names an untitled coach-less conversation as a new conversation', async () => {
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: null, coach_id: null }],
      total: 1,
    });

    renderChatTab();

    await waitFor(() =>
      expect(screen.getByTestId('conversation-header-title')).toHaveTextContent('New conversation'),
    );
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
    listParticipants.mockResolvedValue([]);
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

describe('ChatTab verdict drawer', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getProvidersStatus.mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] });
    listParticipants.mockResolvedValue([]);
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Claims', coach_id: null }],
      total: 1,
    });
    listCoaches.mockResolvedValue({ coaches: [] });
    getConversationMessages.mockResolvedValue({ messages: [] });
  });

  it('re-reads the verdicts when a chip is clicked before its rows landed, then draws the card', async () => {
    // The verdict rows are written right after the reply row, so the chip on
    // a live turn can be clicked while the conversation's read still says
    // "none". The drawer must open on a refetch and its loading line — never
    // on an empty list read as "no verdicts".
    const row: ClaimVerdict = {
      id: 'verdict-1',
      conversation_id: CONVERSATION_ID,
      message_id: 'm4',
      coach_id: null,
      claim_text: 'Your VO2max is 82.',
      category: 'physiological',
      status: 'contradicted',
      evidence_strength: 'none',
      confidence: 0.91,
      layer_fired: 'deterministic',
      explanation: null,
      evidence_refs: null,
      created_at: '2026-08-23T10:01:03Z',
    };
    let deliverRows: (value: { verdicts: ClaimVerdict[] }) => void = () => undefined;
    getConversationVerdicts
      .mockResolvedValueOnce({ verdicts: [] })
      .mockImplementationOnce(
        () =>
          new Promise<{ verdicts: ClaimVerdict[] }>((resolve) => {
            deliverRows = resolve;
          }),
      );
    answerWith([
      { type: 'prose', text: 'Your VO2max is 82.' },
      {
        type: 'verdicts',
        chips: [
          { claim: 'Ton VO2max est de 82.', contradicted: true },
          { claim: 'Six heures de sommeil suffisent.', contradicted: false },
        ],
      },
    ]);

    renderChatTab();
    const user = await send('What is my VO2max?');

    // The chip previews the turn's two chips while the read has no row for them.
    const chip = await screen.findByTestId('verdict-chip');
    expect(chip).toHaveTextContent('2 verdicts · contradicted');
    await user.click(chip);

    expect(await screen.findByTestId('verdict-drawer')).toBeInTheDocument();
    expect(screen.getAllByText('Loading verdicts…').length).toBeGreaterThan(0);
    expect(screen.queryByTestId('verdict-card')).toBeNull();
    expect(getConversationVerdicts).toHaveBeenCalledTimes(2);

    deliverRows({ verdicts: [row] });

    const card = await screen.findByTestId('verdict-card');
    expect(card).toHaveTextContent('Your VO2max is 82.');
    expect(screen.queryByText('Loading verdicts…')).toBeNull();
    // And the chip now counts the one row, not the two chips beside it.
    expect(screen.getByTestId('verdict-chip')).toHaveTextContent('1 verdict · contradicted');
  });
});

describe('ChatTab conversation rotation', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getProvidersStatus.mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] });
    getConversationVerdicts.mockResolvedValue({ verdicts: [] });
    listParticipants.mockResolvedValue([]);
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Long thread', coach_id: null }],
      total: 1,
    });
    listCoaches.mockResolvedValue({ coaches: [] });
    getConversationMessages.mockResolvedValue({ messages: [] });
  });

  /** Answer the next send with a command turn that rotated the thread. */
  function answerWithRotation(rotatedTo: string | undefined) {
    sendTurn.mockImplementation(
      async (
        _conversationId: string,
        _content: string,
        options: {
          onBlock?: (block: ReplyBlock) => void;
          onDone?: (turn: TurnEnvelope) => void;
        },
      ) => {
        options.onBlock?.({ type: 'prose', text: 'New conversation started.' });
        const turn = turnEnvelope();
        turn.assistant.finish_reason = 'command';
        turn.rotated_to_conversation_id = rotatedTo;
        options.onDone?.(turn);
      },
    );
  }

  // `/reset` archives the thread server-side; the client's only job is to open
  // the one the turn names. Asserted on the selection callback rather than on
  // rendered text: what matters is that the surface MOVED, not what it drew.
  it('opens the conversation a rotating turn names', async () => {
    const onSelectConversation = vi.fn();
    answerWithRotation('conv-fresh');

    renderChatTab({ onSelectConversation });
    await send('/reset');

    await waitFor(() => expect(onSelectConversation).toHaveBeenCalledWith('conv-fresh'));
    // The list is re-read before the switch, so the fresh row is there to open.
    expect(getConversations).toHaveBeenCalled();
  });

  it('stays put when the turn names no other conversation', async () => {
    const onSelectConversation = vi.fn();
    answerWithRotation(undefined);

    renderChatTab({ onSelectConversation });
    await send('What is my VO2max?');

    await waitFor(() => expect(sendTurn).toHaveBeenCalled());
    expect(onSelectConversation).not.toHaveBeenCalled();
  });
});

describe('ChatTab header info drawer', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getProvidersStatus.mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] });
    getConversationVerdicts.mockResolvedValue({ verdicts: [] });
    listParticipants.mockResolvedValue([]);
    getConversationMessages.mockResolvedValue({ messages: [] });
    listCoaches.mockResolvedValue({
      coaches: [
        {
          id: COACH_ID,
          title: COACH_TITLE,
          description: 'Builds a marathon block.',
          category: 'endurance',
          handle: 'marathon-coach',
          is_system: false,
        },
      ],
    });
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Sunday long run', coach_id: COACH_ID }],
      total: 1,
    });
  });

  it('makes the header title the button that opens the drawer', async () => {
    const user = userEvent.setup();
    renderChatTab();

    const title = await screen.findByTestId('conversation-header-title');
    expect(title.tagName).toBe('BUTTON');
    expect(title).toHaveAttribute('aria-haspopup', 'dialog');
    expect(screen.queryByTestId('conversation-info-panel')).toBeNull();

    await user.click(title);

    expect(await screen.findByTestId('conversation-info-panel')).toBeInTheDocument();
    expect(await screen.findByTestId('coach-info-panel')).toBeInTheDocument();
  });

  it('opens Group info for a group-scoped thread, named by the group', async () => {
    getConversations.mockResolvedValue({
      conversations: [
        {
          id: CONVERSATION_ID,
          title: 'Sunday long run',
          coach_id: COACH_ID,
          group_id: 'group-7',
          group_name: 'Sunday Riders',
        },
      ],
      total: 1,
    });
    const user = userEvent.setup();
    renderChatTab();

    await waitFor(() =>
      expect(screen.getByTestId('conversation-header-title')).toHaveTextContent('Sunday Riders'),
    );
    await user.click(screen.getByTestId('conversation-header-title'));

    expect(await screen.findByTestId('group-info-panel')).toHaveTextContent('group group-7');
  });

  it('routes Edit coach to the coach Discover detail', async () => {
    const onNavigate = vi.fn();
    const user = userEvent.setup();
    renderChatTab({ onNavigate });

    await user.click(await screen.findByTestId('conversation-header-title'));
    await user.click(await screen.findByTestId('coach-info-edit'));

    expect(onNavigate).toHaveBeenCalledWith('discover/coach-1');
  });

  it('sends /agent remove as a turn when the agent is removed from the chat', async () => {
    sendTurn.mockResolvedValue(undefined);
    const user = userEvent.setup();
    renderChatTab();

    await user.click(await screen.findByTestId('conversation-header-title'));
    await user.click(await screen.findByTestId('coach-info-remove'));

    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(1));
    expect(sendTurn.mock.calls[0][1]).toBe('/agent remove');
  });

  it('offers no agent creation or agent form anywhere in the chat surface', async () => {
    renderChatTab();

    await screen.findByTestId('conversation-header-title');
    expect(screen.queryByRole('button', { name: /Create Agent/i })).toBeNull();
    expect(screen.queryByTestId('prompt-suggestions')).toBeNull();
    expect(screen.queryByLabelText('Agent Name')).toBeNull();
  });
});

describe('ChatTab copy and share', () => {
  const BEFORE = 'Voici ton vélo d’août — surtout du VTT à Prévost 🚴';
  const AFTER = 'Neuf sorties, environ 128 km au total.';
  const REPLY = `${BEFORE}\n\n⟦viz:0⟧\n\n${AFTER}`;
  const READABLE = `${BEFORE}\n\n[Chart: Volume hebdomadaire]\n\n${AFTER}`;
  const SCENE_BLOCKS = JSON.stringify([
    {
      kind: 'chart',
      view_box: { x: 0, y: 0, width: 320, height: 180 },
      nodes: [],
      legend: [],
      title: 'Volume hebdomadaire',
      source_tool: 'get_activities',
    },
  ]);

  const writeText = vi.fn();

  /**
   * `userEvent.setup()` installs a clipboard stub of its own, so the spy the
   * assertions read has to land after it.
   */
  function setupWithClipboard() {
    const user = userEvent.setup();
    Object.defineProperty(window.navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    });
    return user;
  }

  beforeEach(() => {
    vi.clearAllMocks();
    getProvidersStatus.mockResolvedValue({ providers: [] });
    getConversationVerdicts.mockResolvedValue({ verdicts: [] });
    listParticipants.mockResolvedValue([]);
    listCoaches.mockResolvedValue({ coaches: [] });
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Août à vélo', coach_id: null }],
      total: 1,
    });
    getConversationMessages.mockResolvedValue({
      messages: [
        {
          id: 'm1',
          role: 'assistant',
          content: REPLY,
          scene_blocks: SCENE_BLOCKS,
          created_at: '2026-09-02T10:00:00Z',
        },
      ],
    });
  });

  it('puts the chart’s caption on the clipboard, never the ⟦viz:0⟧ marker', async () => {
    const user = setupWithClipboard();
    renderChatTab();

    await user.click(await screen.findByTitle('Copy message'));

    expect(writeText).toHaveBeenCalledTimes(1);
    const copied = writeText.mock.calls[0][0] as string;
    // Paste this into a message to a training partner and they read a caption,
    // not a token that means nothing outside the app.
    expect(copied).not.toContain('⟦');
    expect(copied).not.toContain('⟧');
    expect(copied).toBe(READABLE);
  });

  it('shares the same readable text when the browser has no share sheet', async () => {
    const user = setupWithClipboard();
    renderChatTab();

    await user.click(await screen.findByTitle('Share'));

    // No navigator.share in jsdom, so the share path falls to the clipboard —
    // and it carries the same text the copy button does.
    expect(writeText).toHaveBeenCalledWith(READABLE);
  });
});
