// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Home's personal surface — Today beside the thread, its folded line, the history and where a Today tap drafts
// ABOUTME: Drives ChatTab's personal layout against mocked chat reads; Today is a stand-in the test hands it

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor, within, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ChatTab, { type PersonalToday } from '../ChatTab';
import { ToastProvider } from '../ui';

const getConversations = vi.fn();
const getConversationMessages = vi.fn();
const getConversationVerdicts = vi.fn();
const createConversation = vi.fn();
const markConversationRead = vi.fn();
const sendTurn = vi.fn();
const listCoaches = vi.fn();
const getProvidersStatus = vi.fn();

vi.mock('../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    getConversationMessages: (...a: unknown[]) => getConversationMessages(...a),
    getConversationVerdicts: (...a: unknown[]) => getConversationVerdicts(...a),
    createConversation: (...a: unknown[]) => createConversation(...a),
    markConversationRead: (...a: unknown[]) => markConversationRead(...a),
    sendTurn: (...a: unknown[]) => sendTurn(...a),
    listParticipants: vi.fn().mockResolvedValue([]),
  },
  coachesApi: { list: (...a: unknown[]) => listCoaches(...a) },
  providersApi: { getProvidersStatus: (...a: unknown[]) => getProvidersStatus(...a) },
  // Read by the shared hook bindings at import; this spec asserts nothing they fetch.
  groupsApi: {},
}));

vi.mock('../../services/analytics', () => ({ track: vi.fn() }));
vi.mock('../../hooks/useUsageStatus', () => ({
  useUsageStatus: () => ({
    level: 'none',
    sendDisabled: false,
    message: '',
    invalidate: vi.fn(),
    applyNotice: vi.fn(),
  }),
}));

const DRAFT = 'Walk me through my session on Monday 5 October: Easy trail';

/** Today as the host draws it, reduced to what the surface places: a day to tap and the folded line. */
const TODAY: PersonalToday = {
  panel: (onOpenChatDraft) => (
    <button type="button" onClick={() => onOpenChatDraft(DRAFT)}>
      tap day
    </button>
  ),
  peek: (onOpen, hidden) => (
    <button type="button" data-testid="peek" data-hidden={String(hidden)} onClick={onOpen}>
      today line
    </button>
  ),
};

function renderHome(selected: string | null, props: { onSelectConversation?: (id: string | null) => void } = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <ChatTab
          layout="personal"
          selectedConversation={selected}
          onSelectConversation={props.onSelectConversation ?? vi.fn()}
          today={TODAY}
        />
      </ToastProvider>
    </QueryClientProvider>,
  );
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
});
