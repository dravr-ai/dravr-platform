// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach seat over a Groups room — the strip in place of the provider nudge and the header's state (carnet#1520)
// ABOUTME: Pins who sees it, that it says nothing pending before a request exists, and how a tap moves it to pending

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachAccessRequest, CoachingGroup, GroupMember } from '@pierre/shared-types';
import ChatTab from '../ChatTab';
import { ToastProvider } from '../ui';

const CONVERSATION_ID = 'conv-room';
const GROUP_ID = 'group-seat';
const OWNER_ID = 'user-owner';

const api = vi.hoisted(() => ({
  getConversations: vi.fn(),
  getConversationMessages: vi.fn(),
  getConversationVerdicts: vi.fn(),
  listParticipants: vi.fn(),
  getGroup: vi.fn(),
  listMembers: vi.fn(),
  readRoom: vi.fn(),
  getProvidersStatus: vi.fn(),
  getOnboardingStatus: vi.fn(),
  getCoachAccessRequest: vi.fn(),
  requestCoachAccess: vi.fn(),
  cancelCoachAccessRequest: vi.fn(),
}));

// Only the network is stubbed: ChatTab, the seat hook, the strip and the
// header run for real over it. The browser flow over mocked routes is
// `e2e/coach-seat.spec.ts`.
vi.mock('../../services/api', () => ({
  notificationsApi: { getUnreadCount: async () => ({ unread_count: 0 }) },
  chatApi: {
    getConversations: (...a: unknown[]) => api.getConversations(...a),
    getConversationMessages: (...a: unknown[]) => api.getConversationMessages(...a),
    getConversationVerdicts: (...a: unknown[]) => api.getConversationVerdicts(...a),
    listParticipants: (...a: unknown[]) => api.listParticipants(...a),
    sendTurn: vi.fn(),
    markConversationRead: vi.fn().mockResolvedValue(undefined),
    createConversation: vi.fn(),
    updateConversation: vi.fn(),
    deleteConversation: vi.fn(),
    markConversationUnread: vi.fn(),
  },
  coachesApi: { list: vi.fn().mockResolvedValue({ agents: [] }) },
  providersApi: { getProvidersStatus: (...a: unknown[]) => api.getProvidersStatus(...a) },
  groupsApi: {
    getGroup: (...a: unknown[]) => api.getGroup(...a),
    listMembers: (...a: unknown[]) => api.listMembers(...a),
    readRoom: (...a: unknown[]) => api.readRoom(...a),
  },
  userApi: {
    getOnboardingStatus: (...a: unknown[]) => api.getOnboardingStatus(...a),
    getCoachAccessRequest: (...a: unknown[]) => api.getCoachAccessRequest(...a),
    requestCoachAccess: (...a: unknown[]) => api.requestCoachAccess(...a),
    cancelCoachAccessRequest: (...a: unknown[]) => api.cancelCoachAccessRequest(...a),
  },
}));

vi.mock('../groups/GroupInfoPanel', () => ({ default: () => <div data-testid="group-info-panel" /> }));
vi.mock('../../services/analytics', () => ({ track: vi.fn() }));

const viewer = vi.hoisted(() => ({ id: 'user-owner', manages_roster: false }));
vi.mock('../../hooks/useAuth', () => ({ useAuth: () => ({ token: 'test-token', user: viewer }) }));
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
    applyNotice: vi.fn(),
  }),
}));

function groupRecord(overrides: Partial<CoachingGroup> = {}): CoachingGroup {
  return {
    id: GROUP_ID,
    tenant_id: 'tenant-a',
    name: 'Léa · Marathon',
    description: null,
    agent_id: 'agent-1',
    agent_title: 'Endurance',
    agent_handle: 'endurance',
    owner_id: OWNER_ID,
    coach_user_id: null,
    coach_display_name: null,
    peer_data_sharing: false,
    respond_mode: 'all',
    digest_mode: 'off',
    max_members: 10,
    is_active: true,
    created_at: '2026-11-10T00:00:00Z',
    updated_at: '2026-11-10T00:00:00Z',
    ...overrides,
  };
}

function membership(role: GroupMember['role']): GroupMember {
  return {
    id: 'm-1',
    group_id: GROUP_ID,
    user_id: OWNER_ID,
    role,
    peer_sharing_consent: false,
    consent_given_at: null,
    joined_at: '2026-11-10T00:00:00Z',
    display_name: 'Philippe',
  } as GroupMember;
}

const pending: CoachAccessRequest = {
  id: 'req-1',
  user_id: OWNER_ID,
  group_id: GROUP_ID,
  group_tenant_id: 'tenant-a',
  status: 'pending',
  created_at: '2026-11-12T19:32:00Z',
  decided_at: null,
  decided_by: null,
};

function status(coachesOthers: boolean) {
  return {
    needs_provider_connection: false,
    pillars_covered: 0,
    pillars_total: 7,
    onboarding_complete: false,
    steps: [{ step_id: 'coach_proposal', status: 'complete' }],
    chosen_channel: null,
    coaches_others: coachesOthers,
  };
}

function renderRoom() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <ChatTab selectedConversation={CONVERSATION_ID} onSelectConversation={vi.fn()} onNavigate={vi.fn()} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe('the coach seat over a Groups room', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    viewer.manages_roster = false;
    api.getConversations.mockResolvedValue({
      conversations: [
        { id: CONVERSATION_ID, title: 'Léa · Marathon', agent_id: 'agent-1', group_id: GROUP_ID, group_name: 'Léa · Marathon' },
      ],
      total: 1,
    });
    api.getConversationMessages.mockResolvedValue({ messages: [] });
    api.getConversationVerdicts.mockResolvedValue({ verdicts: [] });
    api.listParticipants.mockResolvedValue([]);
    api.readRoom.mockResolvedValue([]);
    // No provider connected: without the seat, the room would carry the connect nudge.
    api.getProvidersStatus.mockResolvedValue({ providers: [] });
    api.getGroup.mockResolvedValue(groupRecord());
    api.listMembers.mockResolvedValue({ members: [membership('owner')] });
    api.getOnboardingStatus.mockResolvedValue(status(true));
    api.getCoachAccessRequest.mockResolvedValue({ request: null });
  });

  it('puts the filled request in the room, in place of the provider nudge, and says the state in the header', async () => {
    renderRoom();

    const strip = await screen.findByTestId('coach-seat-strip');
    expect(strip).toHaveAttribute('data-state', 'to_request');
    expect(within(strip).getByRole('heading', { name: "You're not this group's coach yet" })).toBeInTheDocument();
    expect(within(strip).getByTestId('coach-access-request')).toHaveClass('btn-primary');
    expect(screen.queryByTestId('connect-provider-banner')).toBeNull();
    const state = await screen.findByTestId('thread-status');
    expect(state).toHaveTextContent('coach access to request');
    // The state sits inside the title button, which opens Group info.
    expect(screen.getByTestId('conversation-header-title')).toContainElement(state);
    // Nothing is pending before a request exists.
    expect(strip).not.toHaveTextContent(/pending/i);
  });

  it('moves strip and header to pending, dated, once the coach asks', async () => {
    api.requestCoachAccess.mockResolvedValue({ request: pending });
    renderRoom();

    const strip = await screen.findByTestId('coach-seat-strip');
    await userEvent.click(within(strip).getByTestId('coach-access-request'));

    await waitFor(() => expect(api.requestCoachAccess).toHaveBeenCalledWith(GROUP_ID));
    await waitFor(() => expect(strip).toHaveAttribute('data-state', 'pending'));
    expect(within(strip).getByRole('heading')).toHaveTextContent(/^Coach access requested$/);
    // Dated, no wait time promised: the answer comes here and by email.
    expect(within(strip).getByTestId('coach-access-pending')).toHaveTextContent(
      /^Request sent .*2026\. We'll let you know here and by email\.$/,
    );
    expect(strip.textContent).not.toMatch(/24/);
    expect(within(strip).queryByTestId('coach-access-request')).toBeNull();
    expect(within(strip).getByTestId('coach-access-cancel')).toHaveTextContent('Cancel request');
    // Focus follows the request to its new state.
    expect(within(strip).getByRole('heading')).toHaveFocus();
    await waitFor(() => expect(screen.getByTestId('thread-status')).toHaveTextContent('coach access pending'));
    expect(screen.getByTestId('thread-status')).toHaveAttribute('data-state', 'pending');
  });

  it('opens Group info from the strip, where the coaching platforms are', async () => {
    renderRoom();

    const strip = await screen.findByTestId('coach-seat-strip');
    await userEvent.click(within(strip).getByRole('button', { name: 'Link a coach account' }));

    expect(await screen.findByTestId('group-info-panel')).toBeInTheDocument();
  });

  it('says the request failed and keeps the button when the server refuses it', async () => {
    api.requestCoachAccess.mockRejectedValue(new Error('boom'));
    renderRoom();

    const strip = await screen.findByTestId('coach-seat-strip');
    await userEvent.click(within(strip).getByTestId('coach-access-request'));

    expect(await within(strip).findByRole('alert')).toHaveTextContent("We couldn't send your request. Please try again.");
    expect(within(strip).getByTestId('coach-access-request')).toBeEnabled();
  });

  it('withdraws the request from the strip, which offers it again at once', async () => {
    api.getCoachAccessRequest.mockResolvedValue({ request: pending });
    api.cancelCoachAccessRequest.mockResolvedValue({ request: null });
    renderRoom();

    const strip = await screen.findByTestId('coach-seat-strip');
    await waitFor(() => expect(strip).toHaveAttribute('data-state', 'pending'));
    await userEvent.click(within(strip).getByTestId('coach-access-cancel'));

    await waitFor(() => expect(api.cancelCoachAccessRequest).toHaveBeenCalled());
    await waitFor(() => expect(strip).toHaveAttribute('data-state', 'to_request'));
    expect(within(strip).getByTestId('coach-access-cancelled')).toHaveTextContent('Request cancelled');
    await waitFor(() => expect(screen.getByTestId('thread-status')).toHaveTextContent('coach access to request'));
  });

  it('offers the strip to an owner who never said they coach others (carnet#1522)', async () => {
    api.getOnboardingStatus.mockResolvedValue(status(false));
    renderRoom();

    const strip = await screen.findByTestId('coach-seat-strip');
    expect(strip).toHaveAttribute('data-state', 'to_request');
  });

  it('shows neither strip nor state to a member who does not own the group', async () => {
    api.getGroup.mockResolvedValue(groupRecord({ owner_id: 'someone-else' }));
    renderRoom();

    await screen.findByTestId('thread-header');
    await waitFor(() => expect(api.getGroup).toHaveBeenCalled());
    expect(screen.queryByTestId('coach-seat-strip')).toBeNull();
    expect(screen.queryByTestId('thread-status')).toBeNull();
    expect(api.getCoachAccessRequest).not.toHaveBeenCalled();
  });

  it('shows neither to an owner who already holds coach access, and reads no roster for it', async () => {
    viewer.manages_roster = true;
    renderRoom();

    await screen.findByTestId('thread-header');
    await waitFor(() => expect(api.getGroup).toHaveBeenCalled());
    expect(screen.queryByTestId('coach-seat-strip')).toBeNull();
    expect(api.listMembers).not.toHaveBeenCalled();
  });

  it('shows neither in a group that already has its human coach', async () => {
    api.getGroup.mockResolvedValue(groupRecord({ coach_user_id: 'user-coach', coach_display_name: 'Casey' }));
    renderRoom();

    await screen.findByTestId('thread-header');
    await waitFor(() => expect(api.getGroup).toHaveBeenCalled());
    expect(screen.queryByTestId('coach-seat-strip')).toBeNull();
    expect(api.listMembers).not.toHaveBeenCalled();
  });
});
