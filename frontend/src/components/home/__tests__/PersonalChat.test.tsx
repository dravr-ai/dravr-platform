// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Home's personal surface — Today beside the thread, its folded line, the history, where a Today tap drafts, and the empty thread's starters
// ABOUTME: Drives the PersonalChat host against mocked chat reads; Today is a stand-in the test hands it

import { useState } from 'react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor, within, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { UseCaseStarter } from '@pierre/shared-types';
import PersonalChat from '../PersonalChat';
import type { PersonalToday } from '../personalToday';
import { ToastProvider } from '../../ui';

const getConversations = vi.fn();
const getConversationMessages = vi.fn();
const getConversationVerdicts = vi.fn();
const createConversation = vi.fn();
const openPersonalTopic = vi.fn();
const markConversationRead = vi.fn();
const sendTurn = vi.fn();
const listStarters = vi.fn();
const listCoaches = vi.fn();
const getProvidersStatus = vi.fn();

vi.mock('../../../services/api', () => ({
  // The header's notifications bell reads the unread count.
  notificationsApi: { getUnreadCount: async () => ({ unread_count: 0 }) },
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    getConversationMessages: (...a: unknown[]) => getConversationMessages(...a),
    getConversationVerdicts: (...a: unknown[]) => getConversationVerdicts(...a),
    createConversation: (...a: unknown[]) => createConversation(...a),
    openPersonalTopic: (...a: unknown[]) => openPersonalTopic(...a),
    markConversationRead: (...a: unknown[]) => markConversationRead(...a),
    sendTurn: (...a: unknown[]) => sendTurn(...a),
    listStarters: (...a: unknown[]) => listStarters(...a),
    listParticipants: vi.fn().mockResolvedValue([]),
  },
  coachesApi: { list: (...a: unknown[]) => listCoaches(...a) },
  providersApi: { getProvidersStatus: (...a: unknown[]) => getProvidersStatus(...a) },
  // Read by the shared hook bindings at import; this spec asserts nothing they fetch.
  groupsApi: {},
}));

vi.mock('../../../services/analytics', () => ({ track: vi.fn() }));
vi.mock('../../../hooks/useAuth', () => ({ useAuth: () => ({ user: { id: 'athlete-1', role: 'user' } }) }));
vi.mock('../../../hooks/useUsageStatus', () => ({
  useUsageStatus: () => ({
    level: 'none',
    sendDisabled: false,
    message: '',
    invalidate: vi.fn(),
    applyNotice: vi.fn(),
  }),
}));

const DRAFT = 'Walk me through my session on Monday 5 October: Easy trail';

/** Today as the host draws it, reduced to what the surface places: a day to tap, a first step, and the folded line. */
const TODAY: PersonalToday = {
  panel: (actions) => (
    <>
      <button type="button" onClick={() => actions.draft(DRAFT)}>
        tap day
      </button>
      <button type="button" onClick={() => actions.send('gs:1:meet_team', 'Meet your team')}>
        tap first step
      </button>
    </>
  ),
  peek: (onOpen, hidden) => (
    <button type="button" data-testid="peek" data-hidden={String(hidden)} onClick={onOpen}>
      today line
    </button>
  ),
};

function renderHome(
  selected: string | null,
  props: { onSelectConversation?: (id: string | null) => void; resolving?: boolean } = {},
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <PersonalChat
          selectedConversation={selected}
          onSelectConversation={props.onSelectConversation ?? vi.fn()}
          today={TODAY}
          resolving={props.resolving}
        />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

/** The three starters the server ranked for a new athlete, as it words them. */
const STARTERS: UseCaseStarter[] = [
  { id: 'connect', label: 'Connect my watch', postback: 'uc:0:connect' },
  { id: 'about_me_more', label: 'Finish my profile', postback: 'uc:1:about_me_more' },
  { id: 'can_see', label: 'What can you see?', postback: 'uc:2:can_see' },
];

/** Home as the shell hosts it: the thread a created conversation selects is the one drawn. */
function StatefulHome() {
  const [selected, setSelected] = useState<string | null>(null);
  return <PersonalChat selectedConversation={selected} onSelectConversation={setSelected} today={TODAY} />;
}

function composer(): HTMLTextAreaElement {
  const field = document.querySelector<HTMLTextAreaElement>('textarea[data-composer="true"]');
  if (!field) throw new Error('no composer on screen');
  return field;
}

/** Make the matchMedia polyfill answer the desktop query, as a ≥1024px window would. */
function wideScreen() {
  vi.spyOn(window, 'matchMedia').mockImplementation(
    (query: string) =>
      ({
        matches: query === '(min-width: 1024px)',
        media: query,
        onchange: null,
        addEventListener: () => {},
        removeEventListener: () => {},
        addListener: () => {},
        removeListener: () => {},
        dispatchEvent: () => false,
      }) as MediaQueryList,
  );
}

describe('Home — the personal surface', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    getProvidersStatus.mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] });
    getConversationVerdicts.mockResolvedValue({ verdicts: [] });
    getConversations.mockResolvedValue({
      conversations: [
        { id: 'conv-1', title: 'Bromont analysis', agent_id: null, channel_type: 'web', unread_count: 0, updated_at: '2026-10-05T09:52:00Z' },
        { id: 'conv-2', title: 'Night-before fuelling', agent_id: null, channel_type: 'telegram', unread_count: 1, updated_at: '2026-10-03T21:14:00Z' },
        { id: 'room-1', title: 'Sunday Riders', agent_id: null, group_id: 'group-1', unread_count: 2, updated_at: '2026-10-05T09:46:00Z' },
      ],
      total: 3,
    });
    getConversationMessages.mockResolvedValue({ messages: [] });
    markConversationRead.mockResolvedValue(undefined);
    listCoaches.mockResolvedValue({ agents: [] });
    createConversation.mockResolvedValue({ id: 'conv-new', title: 'Chat' });
    openPersonalTopic.mockResolvedValue({
      conversation: { id: 'conv-new', title: 'Dravr', personal: true },
      divider: null,
    });
    listStarters.mockResolvedValue(STARTERS);
    sendTurn.mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('drafts a Today tap into the open thread’s composer, without starting another thread', async () => {
    const user = userEvent.setup();
    renderHome('conv-1');

    await user.click(await screen.findByTestId('peek'));
    const sheet = await screen.findByRole('dialog', { name: 'Today' });
    await user.click(within(sheet).getByRole('button', { name: 'tap day' }));

    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Today' })).toBeNull());
    expect(composer()).toHaveValue(DRAFT);
    expect(createConversation).not.toHaveBeenCalled();
    expect(sendTurn).not.toHaveBeenCalled();
  });

  // carnet#1098: a Getting started row sends its starter, as the empty
  // thread's starters do, after the Today sheet steps aside.
  it('sends a Today first step into the open thread as the athlete’s turn, the sheet closed first', async () => {
    const user = userEvent.setup();
    renderHome('conv-1');

    await user.click(await screen.findByTestId('peek'));
    const sheet = await screen.findByRole('dialog', { name: 'Today' });
    await user.click(within(sheet).getByRole('button', { name: 'tap first step' }));

    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Today' })).toBeNull());
    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(1));
    const [conversationId, content] = sendTurn.mock.calls[0];
    expect(conversationId).toBe('conv-1');
    expect(content).toBe('gs:1:meet_team');
    expect(createConversation).not.toHaveBeenCalled();
    // The bubble shows the row's words, never the postback.
    expect(await screen.findByText('Meet your team')).toBeInTheDocument();
    expect(screen.queryByText('gs:1:meet_team')).toBeNull();
  });

  it('lists the athlete’s own threads behind History, never a room, and opens the one picked', async () => {
    const onSelectConversation = vi.fn();
    const user = userEvent.setup();
    renderHome('conv-1', { onSelectConversation });

    await user.click(await screen.findByTestId('home-history-button'));
    const history = await screen.findByRole('dialog', { name: 'History' });
    const rows = await within(history).findAllByTestId('conversation-row');
    expect(rows.map((row) => row.getAttribute('data-conversation-id'))).toEqual(['conv-1', 'conv-2']);

    await user.click(within(history).getByRole('button', { name: /Night-before fuelling/ }));

    expect(onSelectConversation).toHaveBeenCalledWith('conv-2');
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'History' })).toBeNull());
  });

  it('folds the Today line away as the athlete reads up the thread and brings it back at the latest message', async () => {
    renderHome('conv-1');
    const peek = await screen.findByTestId('peek');
    expect(peek).toHaveAttribute('data-hidden', 'false');

    const scroller = composer().closest('[data-testid="home-page"]')?.querySelector<HTMLElement>('.overflow-y-auto');
    if (!scroller) throw new Error('no transcript scroller');
    Object.defineProperty(scroller, 'scrollHeight', { configurable: true, value: 2000 });
    Object.defineProperty(scroller, 'clientHeight', { configurable: true, value: 500 });
    const scrollTo = (top: number) => {
      scroller.scrollTop = top;
      fireEvent.scroll(scroller);
    };

    // The thread's own scroll to its end on open, stopping a little short.
    scrollTo(1450);
    expect(screen.getByTestId('peek')).toHaveAttribute('data-hidden', 'false');
    // Reading back up the thread.
    scrollTo(900);
    expect(screen.getByTestId('peek')).toHaveAttribute('data-hidden', 'true');
    // Coming back down, but not yet at the end.
    scrollTo(1200);
    expect(screen.getByTestId('peek')).toHaveAttribute('data-hidden', 'true');
    // At the latest message.
    scrollTo(1480);
    expect(screen.getByTestId('peek')).toHaveAttribute('data-hidden', 'false');
  });

  it('docks Today beside the thread on a wide screen and remembers when it is folded away', async () => {
    wideScreen();
    const user = userEvent.setup();
    const first = renderHome('conv-1');

    const panel = await screen.findByTestId('home-today-panel');
    expect(within(panel).getByRole('button', { name: 'tap day' })).toBeInTheDocument();
    // A wide screen has the panel, not the folded line.
    expect(screen.queryByTestId('peek')).toBeNull();

    await user.click(screen.getByRole('button', { name: 'Hide Today' }));
    expect(screen.queryByTestId('home-today-panel')).toBeNull();
    expect(localStorage.getItem('dravr.home.todayPanel')).toBe('closed');

    first.unmount();
    renderHome('conv-1');
    await screen.findByRole('button', { name: 'Show Today' });
    expect(screen.queryByTestId('home-today-panel')).toBeNull();
  });

  it('offers the starters ranked for the athlete with no thread open, and a tap opens a topic on its postback', async () => {
    const user = userEvent.setup();
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    render(
      <QueryClientProvider client={queryClient}>
        <ToastProvider>
          <StatefulHome />
        </ToastProvider>
      </QueryClientProvider>,
    );

    const empty = await screen.findByTestId('chat-empty-state');
    await within(empty).findByTestId('chat-empty-starter-connect');
    expect(within(empty).getAllByTestId(/^chat-empty-starter-/).map((link) => link.textContent)).toEqual(
      STARTERS.map((starter) => starter.label),
    );
    expect(listStarters).toHaveBeenCalledTimes(1);

    await user.click(within(empty).getByTestId('chat-empty-starter-about_me_more'));

    // The athlete speaks first: the topic opens in the personal thread without
    // a welcome, so the same starters are not offered again over the answer.
    await waitFor(() => expect(openPersonalTopic).toHaveBeenCalledTimes(1));
    expect(openPersonalTopic.mock.calls[0][0]).toMatchObject({ athlete_speaks_first: true });
    expect(createConversation).not.toHaveBeenCalled();
    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(1));
    const [conversationId, content] = sendTurn.mock.calls[0];
    expect(conversationId).toBe('conv-new');
    expect(content).toBe('uc:1:about_me_more');
    // The bubble shows the label, never the postback.
    expect(await screen.findByText('Finish my profile')).toBeInTheDocument();
    expect(screen.queryByText('uc:1:about_me_more')).toBeNull();
  });

  it('asks for no starters while a thread is open', async () => {
    renderHome('conv-1');
    await screen.findByTestId('peek');
    expect(listStarters).not.toHaveBeenCalled();
  });

  it('asks for no starters while Home is still choosing its thread: no one sees them behind the spinner', async () => {
    renderHome(null, { resolving: true });
    await screen.findByRole('status', { name: 'Loading...' });
    expect(screen.queryByTestId('chat-empty-state')).toBeNull();
    expect(listStarters).not.toHaveBeenCalled();
  });

  // carnet#1146: the server stamps a rebound thread with the agent's
  // canonical English title; the replies are signed with the localized name
  // the agent list serves. Home's header names the agent the way its
  // replies do, and a stored title or rename shows only in History.
  it('names the thread after its agent as the agent list localizes it, as its replies are signed', async () => {
    getConversations.mockResolvedValue({
      conversations: [
        {
          id: 'conv-fuel',
          title: 'Fuelling Agent',
          agent_id: 'agent-fuel',
          agent_handle: 'fuelling-agent',
          agent_title: 'Fuelling Agent',
          personal: true,
          channel_type: 'web',
          unread_count: 0,
          updated_at: '2026-10-23T09:00:00Z',
        },
      ],
      total: 1,
    });
    listCoaches.mockResolvedValue({ agents: [{ id: 'agent-fuel', title: 'Agent Ravitaillement' }] });
    getConversationMessages.mockResolvedValue({
      messages: [
        { id: 'a1', role: 'assistant', content: 'Bonjour, parlons ravitaillement.', created_at: '2026-10-23T09:00:01Z' },
      ],
    });
    const user = userEvent.setup();
    renderHome('conv-fuel');

    await waitFor(() => expect(screen.getByTestId('thread-title')).toHaveTextContent('Agent Ravitaillement'));
    expect(screen.getByTestId('thread-subtitle')).toHaveTextContent('@fuelling-agent');
    // The reply is signed with the same name the header prints.
    await screen.findByText('Bonjour, parlons ravitaillement.');
    expect(screen.getAllByText('Agent Ravitaillement').length).toBeGreaterThan(1);
    expect(screen.getByTestId('thread-header')).not.toHaveTextContent('Fuelling Agent');

    // The stored title is the list's: History still prints it.
    await user.click(screen.getByTestId('home-history-button'));
    const history = await screen.findByRole('dialog', { name: 'History' });
    expect(await within(history).findByRole('button', { name: /Fuelling Agent/ })).toBeInTheDocument();
  });

  it('keeps the stored title for a thread no agent is bound to', async () => {
    renderHome('conv-1');
    await waitFor(() => expect(screen.getByTestId('thread-title')).toHaveTextContent('Bromont analysis'));
  });

  // Home's "+" opens a topic in the personal thread, which no cap refuses —
  // the cap's wording is pinned where a room is created (ChatComposeMenu).
  describe('when a new chat cannot be opened', () => {
    async function startNewChat() {
      const user = userEvent.setup();
      renderHome('conv-1');
      const [menuButton] = await screen.findAllByRole('button', { name: 'New' });
      await user.click(menuButton);
      await user.click(await screen.findByRole('menuitem', { name: 'New chat' }));
    }

    function refusedWith(details: Record<string, unknown>, code: string) {
      return { response: { status: 429, data: { code, message: 'refused', details } } };
    }

    it('reads a spent request budget as the usage quota, not as the conversation cap', async () => {
      openPersonalTopic.mockRejectedValue(
        refusedWith(
          { limit_type: 'requests', current: 10000, limit: 10000, retry_after_secs: 3600 },
          'RateLimitExceeded',
        ),
      );

      await startNewChat();

      expect(
        await screen.findByText('Usage quota reached (10,000/10,000). Please try again later.'),
      ).toBeInTheDocument();
      expect(screen.getByText('Could not start chat')).toBeInTheDocument();
      expect(screen.queryByText('Conversation limit reached')).toBeNull();
      expect(
        screen.queryByText('Something went wrong creating the conversation. Please try again.'),
      ).toBeNull();
      expect(createConversation).not.toHaveBeenCalled();
    });

    it('falls back to the create-failed sentence for any other failure', async () => {
      openPersonalTopic.mockRejectedValue({
        response: { status: 409, data: { code: 'ResourceAlreadyExists' } },
      });

      await startNewChat();

      expect(
        await screen.findByText('Something went wrong creating the conversation. Please try again.'),
      ).toBeInTheDocument();
      expect(screen.getByText('Could not start chat')).toBeInTheDocument();
    });
  });
});
