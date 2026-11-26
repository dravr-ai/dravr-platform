// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the scoped conversation list — Home's history or the Groups tab's rooms: rows, search, empty state, Load more, row actions
// ABOUTME: Mocks chatApi and asserts what the list draws and which calls its actions make; a coach's trifecta athlete reads as one row (carnet#1817)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Conversation } from '@pierre/shared-types';
import type { ConversationScope } from '@pierre/chat-utils';
import type { CoachAccessViewer } from '@pierre/ui-logic';
import ConversationList from '../ConversationList';
import { CONVERSATION_PAGE_SIZE } from '../../../hooks/useConversationList';

const getConversations = vi.fn();
const updateConversation = vi.fn();
const deleteConversation = vi.fn();
const markConversationUnread = vi.fn();

vi.mock('../../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    updateConversation: (...a: unknown[]) => updateConversation(...a),
    deleteConversation: (...a: unknown[]) => deleteConversation(...a),
    markConversationUnread: (...a: unknown[]) => markConversationUnread(...a),
  },
  // The coach-seat lines read where the viewer's seat stands; it stands still here.
  userApi: { getCoachAccessRequest: () => Promise.resolve({ request: null }) },
}));

function conversation(overrides: Partial<Conversation> = {}): Conversation {
  return {
    id: 'conv-1',
    title: 'Marathon plan',
    agent_id: 'coach-running',
    agent_title: 'Running Coach',
    agent_handle: 'running-coach',
    message_count: 3,
    unread_count: 0,
    created_at: '2026-04-13T18:00:00Z',
    updated_at: '2026-04-13T18:00:00Z',
    last_message: { preview: 'Easy 10k tomorrow', role: 'assistant', created_at: '2026-04-13T18:00:00Z' },
    ...overrides,
  };
}

function page(conversations: Conversation[], total = conversations.length) {
  return { conversations, total, limit: CONVERSATION_PAGE_SIZE, offset: 0 };
}

function renderList(
  props: Partial<{
    scope: ConversationScope;
    selectedConversation: string | null;
    onSelectConversation: (id: string | null) => void;
    seatViewer: CoachAccessViewer | null;
    onOpenWorkspace: (athleteId: string) => void;
  }> = {},
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ConversationList
        scope={props.scope ?? 'personal'}
        selectedConversation={props.selectedConversation ?? null}
        onSelectConversation={props.onSelectConversation ?? vi.fn()}
        seatViewer={props.seatViewer ?? null}
        onOpenWorkspace={props.onOpenWorkspace}
      />
    </QueryClientProvider>,
  );
}

describe('ConversationList', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('says each side is empty in its own words', async () => {
    // A room is the Groups tab's, so Home's history holds nothing here.
    getConversations.mockResolvedValue(page([conversation({ id: 'g1', group_id: 'group-1' })]));
    const { unmount } = renderList({ scope: 'personal' });

    expect(await screen.findByTestId('conversation-list-empty')).toHaveTextContent('No other conversations yet.');
    // The list has no "+" of its own: its host owns the one compose control.
    expect(screen.queryByRole('button', { name: 'New' })).toBeNull();
    unmount();

    getConversations.mockResolvedValue(page([conversation({ id: 'c1' })]));
    renderList({ scope: 'groups' });
    expect(await screen.findByTestId('conversation-list-empty')).toHaveTextContent('No groups yet');
  });

  it('keeps one side of the app, newest activity first, with preview and unread count', async () => {
    getConversations.mockResolvedValue(
      page([
        conversation({ id: 'c-old', title: 'Track workout', updated_at: '2026-04-01T10:00:00Z', last_message: null }),
        conversation({ id: 'c-new', title: 'Marathon plan', unread_count: 2 }),
        conversation({
          id: 'c-group',
          title: 'Sunday Riders',
          group_id: 'group-1',
          group_name: 'Sunday Riders',
          agent_title: 'Tempo Coach',
          updated_at: '2026-04-10T10:00:00Z',
          last_message: { preview: 'Ride at 8', role: 'assistant', created_at: '2026-04-10T10:00:00Z' },
        }),
      ]),
    );
    const { unmount } = renderList({ scope: 'personal' });

    // Home's history: the athlete's own threads, the room left out.
    const list = await screen.findByRole('list', { name: 'Conversations' });
    const rows = within(list).getAllByTestId('conversation-row');
    expect(rows.map((row) => row.getAttribute('data-conversation-id'))).toEqual(['c-new', 'c-old']);
    // No coach grouping headers, no "Without a coach" bucket — one flat list.
    expect(screen.queryByText(/Without a coach/)).toBeNull();
    expect(screen.queryByText('Running Coach')).toBeNull();
    expect(within(rows[0]).getByTestId('conversation-preview')).toHaveTextContent('Easy 10k tomorrow');
    expect(within(rows[0]).getByTestId('conversation-unread-count')).toHaveTextContent('2');
    unmount();

    // The Groups tab: the room alone.
    renderList({ scope: 'groups' });
    const groups = await screen.findByRole('list', { name: 'Conversations' });
    const groupRows = within(groups).getAllByTestId('conversation-row');
    expect(groupRows.map((row) => row.getAttribute('data-conversation-id'))).toEqual(['c-group']);
    expect(within(groupRows[0]).getByTestId('conversation-kind-glyph')).toHaveAttribute('data-kind', 'group');
    expect(within(groupRows[0]).getByTestId('conversation-preview')).toHaveTextContent('Tempo Coach: Ride at 8');
  });

  it('opens a row without touching the read marker', async () => {
    getConversations.mockResolvedValue(page([conversation({ id: 'c1', unread_count: 2 })]));
    const onSelectConversation = vi.fn();
    renderList({ onSelectConversation });

    fireEvent.click(await screen.findByRole('button', { name: /Marathon plan/ }));

    expect(onSelectConversation).toHaveBeenCalledWith('c1');
    expect(markConversationUnread).not.toHaveBeenCalled();
  });

  it('filters the rows by title, handle or preview and reports when nothing matches', async () => {
    getConversations.mockResolvedValue(
      page([
        conversation({ id: 'c1', title: 'Marathon plan' }),
        conversation({ id: 'c2', title: 'Deadlift form', agent_handle: 'strength-coach', last_message: null }),
      ]),
    );
    renderList();
    await screen.findByText('Deadlift form');

    const search = screen.getByLabelText('Search our conversations');
    fireEvent.change(search, { target: { value: 'strength' } });
    await waitFor(() => expect(screen.queryByText('Marathon plan')).toBeNull());
    expect(screen.getByText('Deadlift form')).toBeInTheDocument();

    fireEvent.change(search, { target: { value: '10k' } });
    await waitFor(() => expect(screen.getByText('Marathon plan')).toBeInTheDocument());
    expect(screen.queryByText('Deadlift form')).toBeNull();

    fireEvent.change(search, { target: { value: 'kettlebell' } });
    expect(await screen.findByText('No chats match')).toBeInTheDocument();
    // The search narrows what is cached: no keystroke asks the server again.
    expect(getConversations).toHaveBeenCalledTimes(1);
  });

  it('draws its filters as text tabs sized by the pointer, the underline under the chosen word alone', async () => {
    getConversations.mockResolvedValue(page([conversation({ id: 'c1', unread_count: 1 })]));
    renderList({ scope: 'personal' });
    await screen.findByText('Marathon plan');

    const tabs = within(screen.getByRole('radiogroup')).getAllByRole('radio');
    expect(tabs.map((tab) => tab.textContent)).toEqual(['All', 'Unread', 'Agents']);
    const label = (tab: HTMLElement) => tab.querySelector('span') as HTMLElement;
    for (const tab of tabs) {
      // 44x44 on a coarse pointer and under lg; with a fine pointer the
      // padding keeps even a three-letter word clear of the 32px floor and
      // the height at the 36px ceiling (6 + 18 + 10 + 2). The ring is for
      // the keyboard only: a click on a tab moves on.
      expect(tab).toHaveClass('touch-target', 'px-3.5', 'pt-1.5', 'focus-ring-visible');
      expect(label(tab)).toHaveClass('pb-2.5', 'border-b-2');
      expect(tab).not.toHaveClass('focus-ring');
    }
    expect(tabs[0]).toHaveAttribute('aria-checked', 'true');
    expect(label(tabs[0])).toHaveClass('border-primary');
    expect(label(tabs[1])).toHaveClass('border-transparent');

    fireEvent.click(tabs[1]);
    expect(tabs[1]).toHaveAttribute('aria-checked', 'true');
    expect(label(tabs[1])).toHaveClass('border-primary');
    expect(label(tabs[0])).toHaveClass('border-transparent');
  });

  it('offers Load more while the total exceeds the rows and asks for the next page', async () => {
    const first = Array.from({ length: CONVERSATION_PAGE_SIZE }, (_, i) =>
      conversation({ id: `c${i}`, title: `Chat ${i}` }),
    );
    getConversations.mockResolvedValueOnce(page(first, CONVERSATION_PAGE_SIZE + 1));
    getConversations.mockResolvedValueOnce({
      conversations: [conversation({ id: 'c-last', title: 'The last one' })],
      total: CONVERSATION_PAGE_SIZE + 1,
      limit: CONVERSATION_PAGE_SIZE,
      offset: CONVERSATION_PAGE_SIZE,
    });
    renderList();

    fireEvent.click(await screen.findByTestId('conversation-list-load-more'));

    expect(await screen.findByText('The last one')).toBeInTheDocument();
    expect(getConversations).toHaveBeenLastCalledWith(CONVERSATION_PAGE_SIZE, CONVERSATION_PAGE_SIZE);
    await waitFor(() => expect(screen.queryByTestId('conversation-list-load-more')).toBeNull());
  });

  it('offers the next page, not "No groups yet", when the loaded page holds none of its side', async () => {
    // Fifty newer one-to-one threads fill the first page; the room is on the second.
    const first = Array.from({ length: CONVERSATION_PAGE_SIZE }, (_, i) =>
      conversation({ id: `c${i}`, title: `Chat ${i}` }),
    );
    getConversations.mockResolvedValueOnce(page(first, CONVERSATION_PAGE_SIZE + 1));
    getConversations.mockResolvedValueOnce({
      conversations: [conversation({ id: 'g1', title: 'Sunday Riders', group_id: 'group-1' })],
      total: CONVERSATION_PAGE_SIZE + 1,
      limit: CONVERSATION_PAGE_SIZE,
      offset: CONVERSATION_PAGE_SIZE,
    });
    renderList({ scope: 'groups' });

    fireEvent.click(await screen.findByTestId('conversation-list-load-more'));
    expect(screen.queryByTestId('conversation-list-empty')).toBeNull();

    const rows = await screen.findAllByTestId('conversation-row');
    expect(rows.map((row) => row.getAttribute('data-conversation-id'))).toEqual(['g1']);
  });

  it('marks a conversation unread from the row action without selecting it', async () => {
    getConversations.mockResolvedValue(page([conversation({ id: 'c1' })]));
    markConversationUnread.mockResolvedValue(undefined);
    const onSelectConversation = vi.fn();
    renderList({ onSelectConversation });
    await screen.findByText('Marathon plan');

    fireEvent.click(screen.getByTestId('conversation-actions-trigger'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Mark conversation unread' }));

    await waitFor(() => expect(markConversationUnread).toHaveBeenCalledWith('c1'));
    expect(onSelectConversation).not.toHaveBeenCalled();
  });

  it('renames inline and saves the trimmed title on Enter', async () => {
    getConversations.mockResolvedValue(page([conversation({ id: 'c1' })]));
    updateConversation.mockResolvedValue(conversation({ id: 'c1', title: 'Fall marathon' }));
    renderList();
    await screen.findByText('Marathon plan');

    fireEvent.click(screen.getByTestId('conversation-actions-trigger'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Rename conversation' }));
    const input = screen.getByLabelText('Conversation title');
    expect(input).toHaveValue('Marathon plan');
    fireEvent.change(input, { target: { value: '  Fall marathon  ' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    await waitFor(() => expect(updateConversation).toHaveBeenCalledWith('c1', { title: 'Fall marathon' }));
  });

  it('deletes after confirmation and deselects the open thread', async () => {
    getConversations.mockResolvedValue(page([conversation({ id: 'c1' })]));
    deleteConversation.mockResolvedValue(undefined);
    const onSelectConversation = vi.fn();
    renderList({ selectedConversation: 'c1', onSelectConversation });
    await screen.findByText('Marathon plan');

    fireEvent.click(screen.getByTestId('conversation-actions-trigger'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete conversation' }));
    expect(await screen.findByText('Delete Conversation')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

    await waitFor(() => expect(deleteConversation).toHaveBeenCalledWith('c1'));
    await waitFor(() => expect(onSelectConversation).toHaveBeenCalledWith(null));
  });

  describe("a coach's trifecta athlete (carnet#1817)", () => {
    const COACH: CoachAccessViewer = { id: 'user-coach', manages_roster: true };
    const HALINA = { user_id: 'user-halina', display_name: 'Halina Torresan' };
    const room = conversation({
      id: 'c-room',
      title: 'Traka 200',
      agent_id: null,
      group_id: 'g-traka',
      group_name: 'Traka 200',
      solo_athlete: HALINA,
      unread_count: 1,
      last_message: { preview: 'See you Saturday', role: 'user', created_at: '2026-04-13T09:00:00Z' },
      updated_at: '2026-04-13T09:00:00Z',
    });
    const plan = conversation({
      id: 'c-plan',
      title: 'Plan · Halina Torresan',
      subject_user_id: HALINA.user_id,
      subject_group_id: 'g-traka',
      subject_display_name: HALINA.display_name,
      solo_athlete: HALINA,
      unread_count: 2,
      last_message: { preview: 'Draft saved', role: 'assistant', created_at: '2026-04-13T11:00:00Z' },
      updated_at: '2026-04-13T11:00:00Z',
    });
    const club = conversation({
      id: 'c-club',
      title: 'Club',
      agent_id: null,
      group_id: 'g-club',
      last_message: { preview: 'Ride at 7', role: 'user', created_at: '2026-04-13T10:00:00Z' },
      updated_at: '2026-04-13T10:00:00Z',
    });

    beforeEach(() => {
      getConversations.mockResolvedValue(page([room, plan, club]));
    });

    it('reads their room and plan thread as one row that opens their workspace, with no row menu', async () => {
      const onOpenWorkspace = vi.fn();
      const onSelectConversation = vi.fn();
      renderList({ scope: 'groups', seatViewer: COACH, onOpenWorkspace, onSelectConversation });

      await waitFor(() => expect(screen.getAllByTestId('conversation-row')).toHaveLength(2));
      const [athlete, other] = screen.getAllByTestId('conversation-row');
      expect(athlete).toHaveAttribute('data-opens-workspace', 'true');
      expect(within(athlete as HTMLElement).getByTestId('conversation-title')).toHaveTextContent('Halina Torresan');
      expect(within(athlete as HTMLElement).getByTestId('conversation-preview')).toHaveTextContent('Draft saved');
      expect(within(athlete as HTMLElement).getByTestId('conversation-unread-count')).toHaveTextContent('3');
      expect(within(athlete as HTMLElement).queryByTestId('conversation-actions-trigger')).toBeNull();
      expect(other).not.toHaveAttribute('data-opens-workspace');

      fireEvent.click(within(athlete as HTMLElement).getByRole('button'));
      expect(onOpenWorkspace).toHaveBeenCalledWith(HALINA.user_id);
      expect(onSelectConversation).not.toHaveBeenCalled();
    });

    it('marks the row the open thread is part of, the plan thread included', async () => {
      renderList({ scope: 'groups', seatViewer: COACH, onOpenWorkspace: vi.fn(), selectedConversation: 'c-plan' });
      await waitFor(() => expect(screen.getAllByTestId('conversation-row')).toHaveLength(2));
      expect(within(screen.getAllByTestId('conversation-row')[0] as HTMLElement).getByRole('button')).toHaveAttribute(
        'aria-current',
        'true',
      );
    });

    it('finds the row by the athlete it is named after', async () => {
      renderList({ scope: 'groups', seatViewer: COACH, onOpenWorkspace: vi.fn() });
      await waitFor(() => expect(screen.getAllByTestId('conversation-row')).toHaveLength(2));
      fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'halina' } });
      await waitFor(() => expect(screen.getAllByTestId('conversation-row')).toHaveLength(1));
      expect(screen.getByTestId('conversation-title')).toHaveTextContent('Halina Torresan');
    });

    it('keeps every conversation its own row for a reader without the coach Home the workspace sits in', async () => {
      renderList({ scope: 'groups', seatViewer: { id: COACH.id }, onOpenWorkspace: vi.fn() });
      await waitFor(() => expect(screen.getAllByTestId('conversation-row')).toHaveLength(3));
      expect(document.querySelector('[data-opens-workspace]')).toBeNull();
    });

    it('keeps every conversation its own row where the host opens no workspace', async () => {
      renderList({ scope: 'groups', seatViewer: COACH });
      await waitFor(() => expect(screen.getAllByTestId('conversation-row')).toHaveLength(3));
    });
  });
});
