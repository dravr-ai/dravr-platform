// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the chat "+" menu — new chat, new group chat, and adding someone to the open thread
// ABOUTME: Drives ChatTab so the menu's actions are asserted against the real conversation calls

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ChatTab from '../ChatTab';
import { ToastProvider } from '../ui';

const getConversations = vi.fn();
const getConversationMessages = vi.fn();
const getConversationVerdicts = vi.fn();
const listParticipants = vi.fn();
const createConversation = vi.fn();
const listCoaches = vi.fn();
const getProvidersStatus = vi.fn();
const listMyGroups = vi.fn();

vi.mock('../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    getConversationMessages: (...a: unknown[]) => getConversationMessages(...a),
    getConversationVerdicts: (...a: unknown[]) => getConversationVerdicts(...a),
    listParticipants: (...a: unknown[]) => listParticipants(...a),
    createConversation: (...a: unknown[]) => createConversation(...a),
  },
  groupsApi: { listMyGroups: (...a: unknown[]) => listMyGroups(...a) },
  coachesApi: { list: (...a: unknown[]) => listCoaches(...a) },
  providersApi: { getProvidersStatus: (...a: unknown[]) => getProvidersStatus(...a) },
  oauthApi: {},
}));

vi.mock('../../services/analytics', () => ({ track: vi.fn() }));
vi.mock('../PromptSuggestions', () => ({ default: () => <div data-testid="prompt-suggestions" /> }));
vi.mock('../../hooks/useUsageStatus', () => ({
  useUsageStatus: () => ({
    level: 'none',
    sendDisabled: false,
    message: '',
    invalidate: vi.fn(),
    applyNotice: vi.fn(),
  }),
}));

const CONVERSATION_ID = 'conv-1';

function renderChatTab(selected: string | null, props: { onSelectConversation?: (id: string | null) => void; onNavigate?: (route: string) => void } = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <ChatTab
          selectedConversation={selected}
          onSelectConversation={props.onSelectConversation ?? vi.fn()}
          onNavigate={props.onNavigate}
        />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe('chat "+" menu', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getProvidersStatus.mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] });
    getConversationVerdicts.mockResolvedValue({ verdicts: [] });
    listParticipants.mockResolvedValue([
      { user_id: 'owner-1', role: 'owner', added_by: 'owner-1', added_at: '2026-08-26T10:00:00Z' },
    ]);
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Sunday long run', coach_id: null }],
      total: 1,
    });
    getConversationMessages.mockResolvedValue({ messages: [] });
    listCoaches.mockResolvedValue({ coaches: [] });
    listMyGroups.mockResolvedValue({
      groups: [
        {
          id: 'group-1',
          name: 'Sunday Riders',
          description: null,
          coach_id: 'coach-tempo',
          member_count: 4,
          is_active: true,
          peer_data_sharing: true,
          my_role: 'member',
          created_at: '2026-08-01T00:00:00Z',
        },
      ],
    });
    createConversation.mockResolvedValue({ id: 'conv-new', title: 'Chat' });
  });

  it('offers exactly new chat and new group chat before a conversation is open', async () => {
    const user = userEvent.setup();
    renderChatTab(null);

    await user.click(await screen.findByRole('button', { name: 'New' }));

    const menu = screen.getByRole('menu', { name: 'Start a conversation' });
    const items = within(menu).getAllByRole('menuitem').map((item) => item.textContent);
    expect(items).toEqual(['New chat', 'New group chat']);
  });

  it('starts a plain conversation from "New chat"', async () => {
    const onSelectConversation = vi.fn();
    const user = userEvent.setup();
    renderChatTab(null, { onSelectConversation });

    await user.click(await screen.findByRole('button', { name: 'New' }));
    await user.click(screen.getByRole('menuitem', { name: 'New chat' }));

    await waitFor(() => expect(createConversation).toHaveBeenCalledTimes(1));
    expect(createConversation.mock.calls[0][0]).not.toHaveProperty('group_id');
    await waitFor(() => expect(onSelectConversation).toHaveBeenCalledWith('conv-new'));
  });

  it('starts a group-scoped conversation from the group picker', async () => {
    const onSelectConversation = vi.fn();
    const user = userEvent.setup();
    renderChatTab(null, { onSelectConversation });

    await user.click(await screen.findByRole('button', { name: 'New' }));
    await user.click(screen.getByRole('menuitem', { name: 'New group chat' }));

    const picker = await screen.findByRole('dialog');
    expect(within(picker).getByText('New group chat')).toBeInTheDocument();
    await user.click(await within(picker).findByRole('button', { name: /Sunday Riders/ }));

    await waitFor(() => expect(createConversation).toHaveBeenCalledTimes(1));
    // `group_id` is what turns on the roster and group context server-side.
    expect(createConversation).toHaveBeenCalledWith({
      title: 'Sunday Riders',
      coach_id: 'coach-tempo',
      group_id: 'group-1',
    });
    await waitFor(() => expect(onSelectConversation).toHaveBeenCalledWith('conv-new'));
  });

  it('sends an athlete in no group to the Groups surface', async () => {
    listMyGroups.mockResolvedValue({ groups: [] });
    const onNavigate = vi.fn();
    const user = userEvent.setup();
    renderChatTab(null, { onNavigate });

    await user.click(await screen.findByRole('button', { name: 'New' }));
    await user.click(screen.getByRole('menuitem', { name: 'New group chat' }));

    expect(await screen.findByText('You are not in a coaching group yet.')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Go to Groups' }));
    expect(onNavigate).toHaveBeenCalledWith('groups');
    expect(createConversation).not.toHaveBeenCalled();
  });

  it('adds "Add someone to this discussion" once a conversation is open, and it opens the participants control', async () => {
    const user = userEvent.setup();
    renderChatTab(CONVERSATION_ID);

    await screen.findByText('Participants (1)');
    await user.click(screen.getByRole('button', { name: 'New' }));

    const menu = screen.getByRole('menu', { name: 'Start a conversation' });
    const items = within(menu).getAllByRole('menuitem').map((item) => item.textContent);
    expect(items).toEqual(['New chat', 'New group chat', 'Add someone to this discussion']);

    await user.click(screen.getByRole('menuitem', { name: 'Add someone to this discussion' }));

    const participants = await screen.findByRole('dialog', { name: 'Conversation participants' });
    expect(within(participants).getByLabelText('User id to add')).toBeInTheDocument();
    expect(within(participants).getByRole('list', { name: 'Participant list' })).toHaveTextContent('owner-1 · owner');
  });
});
