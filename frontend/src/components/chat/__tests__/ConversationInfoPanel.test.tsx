// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the header info drawer's three shapes — Group info, Coach info, and a plain thread
// ABOUTME: Pins that the shape is read off the conversation and that Remove sends the /coach remove command

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Conversation } from '@pierre/shared-types';
import ConversationInfoPanel from '../ConversationInfoPanel';
import { ToastProvider } from '../../ui';

const listCoaches = vi.fn();
const listParticipants = vi.fn();

vi.mock('../../../services/api', () => ({
  coachesApi: { list: (...a: unknown[]) => listCoaches(...a) },
  chatApi: {
    listParticipants: (...a: unknown[]) => listParticipants(...a),
    addParticipant: vi.fn(),
    removeParticipant: vi.fn(),
  },
  groupsApi: {},
}));

vi.mock('../../groups/GroupInfoPanel', () => ({
  default: ({ groupId }: { groupId: string }) => (
    <div data-testid="group-info-panel">group {groupId}</div>
  ),
}));

const COACH = {
  id: 'coach-1',
  title: 'Marathon Coach',
  description: 'Builds a marathon block around your long run.',
  category: 'endurance',
  handle: 'marathon-coach',
  is_system: false,
  system_prompt: '',
  tags: [],
  token_count: 0,
  is_favorite: false,
  use_count: 0,
  last_used_at: null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
};

function conversation(overrides: Partial<Conversation> = {}): Conversation {
  return {
    id: 'conv-1',
    title: 'Sunday long run',
    message_count: 2,
    created_at: '2026-08-01T00:00:00Z',
    updated_at: '2026-08-01T00:00:00Z',
    ...overrides,
  };
}

function renderPanel(conv: Conversation, extra: { openParticipants?: boolean } = {}) {
  const handlers = {
    onClose: vi.fn(),
    onSendCommand: vi.fn(),
    onEditCoach: vi.fn(),
    onRename: vi.fn(),
    onDelete: vi.fn(),
    onThreadGone: vi.fn(),
  };
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <ConversationInfoPanel conversation={conv} {...handlers} {...extra} />
      </ToastProvider>
    </QueryClientProvider>,
  );
  return handlers;
}

describe('ConversationInfoPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listCoaches.mockResolvedValue({ coaches: [COACH] });
    listParticipants.mockResolvedValue([
      { user_id: 'owner-1', role: 'owner', added_by: 'owner-1', added_at: '2026-08-01T00:00:00Z' },
    ]);
  });

  it('draws Group info for a group-scoped thread', async () => {
    renderPanel(conversation({ group_id: 'group-7', group_name: 'Sunday Riders', coach_id: 'coach-1' }));

    expect(await screen.findByTestId('group-info-panel')).toHaveTextContent('group group-7');
    expect(screen.getByRole('dialog', { name: 'Group info' })).toBeInTheDocument();
    expect(screen.queryByTestId('coach-info-panel')).toBeNull();
  });

  it('draws Coach info with the title, handle and mention hint for a coach thread', async () => {
    renderPanel(conversation({ coach_id: 'coach-1' }));

    expect(await screen.findByTestId('coach-info-panel')).toBeInTheDocument();
    expect(screen.getByRole('dialog', { name: 'Coach info' })).toBeInTheDocument();
    expect(screen.getByText('Marathon Coach')).toBeInTheDocument();
    expect(screen.getByTestId('coach-info-handle')).toHaveTextContent('@marathon-coach');
    expect(screen.getByText('endurance')).toBeInTheDocument();
    expect(screen.getByText(/Mention:/)).toBeInTheDocument();
  });

  it('sends /coach remove when the coach is removed from the chat', async () => {
    const user = userEvent.setup();
    const handlers = renderPanel(conversation({ coach_id: 'coach-1' }));

    await user.click(await screen.findByTestId('coach-info-remove'));

    expect(handlers.onSendCommand).toHaveBeenCalledWith('/coach remove');
  });

  it('routes Edit coach to the coach Discover detail, and hides it for a system coach', async () => {
    const user = userEvent.setup();
    const handlers = renderPanel(conversation({ coach_id: 'coach-1' }));

    await user.click(await screen.findByTestId('coach-info-edit'));
    expect(handlers.onEditCoach).toHaveBeenCalledWith('coach-1');

    listCoaches.mockResolvedValue({ coaches: [{ ...COACH, is_system: true }] });
    renderPanel(conversation({ id: 'conv-2', coach_id: 'coach-1' }));
    await waitFor(() => expect(screen.getAllByTestId('coach-info-panel')).toHaveLength(2));
    expect(screen.getAllByTestId('coach-info-edit')).toHaveLength(1);
  });

  it('draws rename, participants and delete for a plain thread', async () => {
    const user = userEvent.setup();
    const handlers = renderPanel(conversation());

    expect(await screen.findByTestId('plain-info-panel')).toBeInTheDocument();
    expect(screen.getByRole('dialog', { name: 'Chat info' })).toBeInTheDocument();

    const title = screen.getByTestId('conversation-info-title');
    await user.clear(title);
    await user.type(title, 'Track night');
    await user.click(screen.getByTestId('conversation-info-rename'));
    expect(handlers.onRename).toHaveBeenCalledWith('Track night');

    await user.click(screen.getByTestId('conversation-info-delete'));
    expect(handlers.onDelete).toHaveBeenCalledTimes(1);
  });

  it('refuses a rename that changes nothing', async () => {
    renderPanel(conversation());
    expect(await screen.findByTestId('conversation-info-rename')).toBeDisabled();
  });

  it('opens the participants control expanded when the "+" menu asked for it', async () => {
    renderPanel(conversation(), { openParticipants: true });

    const list = await screen.findByRole('list', { name: 'Participant list' });
    await waitFor(() => expect(list).toHaveTextContent('owner-1 · owner'));
  });

  it('closes on Escape', async () => {
    const user = userEvent.setup();
    const handlers = renderPanel(conversation());

    await screen.findByTestId('plain-info-panel');
    await user.keyboard('{Escape}');

    expect(handlers.onClose).toHaveBeenCalled();
  });
});
