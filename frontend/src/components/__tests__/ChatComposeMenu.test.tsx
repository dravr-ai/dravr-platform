// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the chat "+" menu — new chat, the group-name dialog, and adding someone to a thread
// ABOUTME: Drives ChatTab so the menu's actions are asserted against the real conversation and turn calls

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
const markConversationRead = vi.fn();
const sendTurn = vi.fn();
const listCoaches = vi.fn();
const getProvidersStatus = vi.fn();

vi.mock('../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    getConversationMessages: (...a: unknown[]) => getConversationMessages(...a),
    getConversationVerdicts: (...a: unknown[]) => getConversationVerdicts(...a),
    listParticipants: (...a: unknown[]) => listParticipants(...a),
    createConversation: (...a: unknown[]) => createConversation(...a),
    markConversationRead: (...a: unknown[]) => markConversationRead(...a),
    sendTurn: (...a: unknown[]) => sendTurn(...a),
  },
  coachesApi: { list: (...a: unknown[]) => listCoaches(...a) },
  providersApi: { getProvidersStatus: (...a: unknown[]) => getProvidersStatus(...a) },
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

const CONVERSATION_ID = 'conv-1';

function renderChatTab(
  selected: string | null,
  props: { onSelectConversation?: (id: string | null) => void } = {},
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <ChatTab
          selectedConversation={selected}
          onSelectConversation={props.onSelectConversation ?? vi.fn()}
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
      conversations: [
        { id: CONVERSATION_ID, title: 'Sunday long run', coach_id: null, unread_count: 0 },
      ],
      total: 1,
    });
    getConversationMessages.mockResolvedValue({ messages: [] });
    markConversationRead.mockResolvedValue(undefined);
    listCoaches.mockResolvedValue({ coaches: [] });
    createConversation.mockResolvedValue({ id: 'conv-new', title: 'Chat' });
    sendTurn.mockResolvedValue(undefined);
  });

  it('offers exactly new chat and new group chat before a conversation is open', async () => {
    const user = userEvent.setup();
    renderChatTab(null);

    await user.click((await screen.findAllByRole('button', { name: 'New' }))[0]);

    const menu = screen.getByRole('menu', { name: 'Start a conversation' });
    const items = within(menu).getAllByRole('menuitem').map((item) => item.textContent);
    expect(items).toEqual(['New chat', 'New group chat']);
  });

  it('starts a plain conversation from "New chat"', async () => {
    const onSelectConversation = vi.fn();
    const user = userEvent.setup();
    renderChatTab(null, { onSelectConversation });

    await user.click((await screen.findAllByRole('button', { name: 'New' }))[0]);
    await user.click(screen.getByRole('menuitem', { name: 'New chat' }));

    await waitFor(() => expect(createConversation).toHaveBeenCalledTimes(1));
    expect(createConversation.mock.calls[0][0]).not.toHaveProperty('group_id');
    await waitFor(() => expect(onSelectConversation).toHaveBeenCalledWith('conv-new'));
  });

  it('asks for a name and opens a fresh thread when none is selected', async () => {
    const onSelectConversation = vi.fn();
    const user = userEvent.setup();
    renderChatTab(null, { onSelectConversation });

    await user.click((await screen.findAllByRole('button', { name: 'New' }))[0]);
    await user.click(screen.getByRole('menuitem', { name: 'New group chat' }));

    await user.type(await screen.findByTestId('group-name-input'), 'Sunday Riders');
    await user.click(screen.getByTestId('group-name-submit'));

    // A plain thread is created first — the command carries the group, so
    // nothing here posts a group. The queued command lands as its first turn.
    await waitFor(() => expect(createConversation).toHaveBeenCalledTimes(1));
    expect(createConversation.mock.calls[0][0]).not.toHaveProperty('group_id');
    await waitFor(() => expect(onSelectConversation).toHaveBeenCalledWith('conv-new'));
  });

  it('sends /group create <name> as the first turn of the fresh thread', async () => {
    const user = userEvent.setup();
    renderChatTab(CONVERSATION_ID);

    await user.click((await screen.findAllByRole('button', { name: 'New' }))[0]);
    await user.click(screen.getByRole('menuitem', { name: 'New group chat' }));

    await user.type(await screen.findByTestId('group-name-input'), 'Sunday Riders');
    await user.click(screen.getByTestId('group-name-submit'));

    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(1));
    expect(sendTurn.mock.calls[0][1]).toBe('/group create Sunday Riders');
    // No client-side group creation survives: the command is the one implementation.
    expect(createConversation).not.toHaveBeenCalled();
  });

  it('refuses an empty group name', async () => {
    const user = userEvent.setup();
    renderChatTab(CONVERSATION_ID);

    await user.click((await screen.findAllByRole('button', { name: 'New' }))[0]);
    await user.click(screen.getByRole('menuitem', { name: 'New group chat' }));

    expect(await screen.findByTestId('group-name-submit')).toBeDisabled();
    expect(sendTurn).not.toHaveBeenCalled();
  });

  it('adds "Add someone to this discussion" once a conversation is open, and it opens the participants control', async () => {
    const user = userEvent.setup();
    renderChatTab(CONVERSATION_ID);

    await user.click((await screen.findAllByRole('button', { name: 'New' }))[0]);

    const menu = screen.getByRole('menu', { name: 'Start a conversation' });
    const items = within(menu).getAllByRole('menuitem').map((item) => item.textContent);
    expect(items).toEqual(['New chat', 'New group chat', 'Add someone to this discussion']);

    await user.click(screen.getByRole('menuitem', { name: 'Add someone to this discussion' }));

    const panel = await screen.findByTestId('conversation-info-panel');
    const participants = await within(panel).findByRole('dialog', {
      name: 'Conversation participants',
    });
    expect(within(participants).getByLabelText('User id to add')).toBeInTheDocument();
    expect(within(participants).getByRole('list', { name: 'Participant list' })).toHaveTextContent(
      'owner-1 · owner',
    );
  });
});
