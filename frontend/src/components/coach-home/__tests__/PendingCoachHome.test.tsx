// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The pending coach Home (carnet#1529) — « Préparer ton coaching » ticked from the server, the seat request, the invite and the platforms
// ABOUTME: Pins that each row says what the server holds, the group is made the onboarding way, and only a coach who trains has the switch

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachSetupResponse } from '@pierre/shared-types';
import { ToastProvider } from '../../ui';
import PendingCoachHome from '../PendingCoachHome';

const getCoachSetup = vi.fn();
const getCoachAccessRequest = vi.fn();
const requestCoachAccess = vi.fn();
const getProvidersStatus = vi.fn();
const listAgents = vi.fn();
const createGroup = vi.fn();
const createInvite = vi.fn();
const listInvites = vi.fn();
const createConversation = vi.fn();
const getConversations = vi.fn();

vi.mock('../../../services/api', () => ({
  userApi: {
    getCoachSetup: (...a: unknown[]) => getCoachSetup(...a),
    getCoachAccessRequest: (...a: unknown[]) => getCoachAccessRequest(...a),
    requestCoachAccess: (...a: unknown[]) => requestCoachAccess(...a),
    cancelCoachAccessRequest: vi.fn(),
  },
  authApi: { getSession: vi.fn() },
  providersApi: { getProvidersStatus: (...a: unknown[]) => getProvidersStatus(...a) },
  coachesApi: { list: (...a: unknown[]) => listAgents(...a) },
  groupsApi: {
    createGroup: (...a: unknown[]) => createGroup(...a),
    createInvite: (...a: unknown[]) => createInvite(...a),
    listInvites: (...a: unknown[]) => listInvites(...a),
    getGroup: vi.fn(),
  },
  chatApi: {
    createConversation: (...a: unknown[]) => createConversation(...a),
    getConversations: (...a: unknown[]) => getConversations(...a),
  },
}));

vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { id: 'coach-1', role: 'user', manages_roster: false }, updateUser: vi.fn() }),
}));

// The platform rows are Group info's own component, tested there.
vi.mock('../../ProviderConnectionCards', () => ({
  default: ({ onlyProviders }: { onlyProviders?: readonly string[] }) => (
    <div data-testid="provider-cards">{(onlyProviders ?? []).join(',')}</div>
  ),
}));

// The human coach's own access and Home, in English.
const SEAT_TITLE = 'Get your coach access'; // humanCoach.seat.panelTitle
const SCOPE_PENDING = "Coach access pending · your coach Home opens as soon as it's on"; // humanCoach.pendingHome.scopePending

function setup(overrides: Partial<CoachSetupResponse> = {}): CoachSetupResponse {
  return {
    rows: [
      { id: 'seat', order: 1, done: false, current: true },
      { id: 'invite', order: 2, done: false, current: false },
      { id: 'platform', order: 3, done: false, current: false },
    ],
    done_count: 0,
    total: 3,
    groups: [],
    athlete: null,
    platform: null,
    ...overrides,
  };
}

function renderHome(trains = false) {
  const navigate = vi.fn();
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <PendingCoachHome onNavigate={navigate} trains={trains} />
      </ToastProvider>
    </QueryClientProvider>,
  );
  return { navigate };
}

beforeEach(() => {
  vi.clearAllMocks();
  getCoachSetup.mockResolvedValue(setup());
  getCoachAccessRequest.mockResolvedValue({ request: null });
  getProvidersStatus.mockResolvedValue({ providers: [] });
  listAgents.mockResolvedValue({
    agents: [
      { id: 'agent-coach', title: 'Coach desk', tags: ['coach-tool'] },
      { id: 'agent-endurance', title: 'Endurance', tags: ['endurance'] },
    ],
  });
  listInvites.mockResolvedValue({ invites: [] });
  getConversations.mockResolvedValue({ conversations: [], total: 0 });
});

describe('PendingCoachHome', () => {
  it('lists the three steps in the server’s order, the seat current with the one filled request', async () => {
    renderHome();

    const checklist = await screen.findByTestId('coach-setup');
    expect(within(checklist).getByRole('heading', { name: 'Get ready to coach' })).toBeInTheDocument();
    expect(screen.getByTestId('coach-setup-count')).toHaveTextContent('0 of 3');
    const rows = within(checklist).getAllByRole('listitem');
    expect(rows.map((row) => row.getAttribute('data-testid'))).toEqual([
      'coach-setup-row-seat',
      'coach-setup-row-invite',
      'coach-setup-row-platform',
    ]);
    const seat = screen.getByTestId('coach-setup-row-seat');
    expect(seat).toHaveAttribute('data-state', 'current');
    expect(seat).toHaveAttribute('aria-current', 'step');
    expect(within(seat).getByText(SEAT_TITLE)).toBeInTheDocument();
    expect(await within(seat).findByRole('button', { name: 'Request coach access' })).toBeInTheDocument();
    // TrainingPeaks and Intervals.icu, the same rows Group info shows (D3).
    expect(screen.getByTestId('provider-cards')).toHaveTextContent('sciotte_trainingpeaks,intervals_icu');
  });

  it('asks in one tap, then dates the request with no wait time and offers to withdraw it', async () => {
    requestCoachAccess.mockResolvedValue({
      request: {
        id: 'req-1',
        user_id: 'coach-1',
        group_id: null,
        group_tenant_id: null,
        status: 'pending',
        created_at: '2026-11-12T19:32:00Z',
        decided_at: null,
        decided_by: null,
      },
    });
    renderHome();

    const seat = await screen.findByTestId('coach-setup-row-seat');
    await userEvent.click(await within(seat).findByRole('button', { name: 'Request coach access' }));

    expect(requestCoachAccess).toHaveBeenCalledWith(undefined);
    expect(await within(seat).findByText(/^Request sent .+\. We'll let you know here and by email\.$/)).toBeInTheDocument();
    expect(within(seat).getByRole('button', { name: 'Cancel request' })).toBeInTheDocument();
    expect(within(seat).queryByText(/24/)).toBeNull();
    await waitFor(() =>
      expect(screen.getByTestId('pending-coach-scope')).toHaveTextContent(
        SCOPE_PENDING,
      ),
    );
  });

  it('makes the first group the onboarding way, named after the athlete, on the endurance agent', async () => {
    createGroup.mockResolvedValue({ id: 'group-1', agent_id: 'agent-endurance', name: 'Léa' });
    createConversation.mockResolvedValue({ id: 'conv-1' });
    createInvite.mockResolvedValue({ code: 'JOIN-1' });
    renderHome();

    const invite = await screen.findByTestId('coach-setup-row-invite');
    await userEvent.type(within(invite).getByLabelText('Your athlete'), 'Léa');
    await userEvent.click(within(invite).getByRole('button', { name: 'Create the group' }));

    await waitFor(() =>
      expect(createGroup).toHaveBeenCalledWith({ name: 'Léa', agent_id: 'agent-endurance', coach_is_me: true }),
    );
    expect(createConversation).toHaveBeenCalledWith({ group_id: 'group-1', agent_id: 'agent-endurance' });
    // The checklist reads the group the server now holds.
    await waitFor(() => expect(getCoachSetup).toHaveBeenCalledTimes(2));
  });

  it('says a usable invite was sent but not joined, neutrally, beside the link to copy again', async () => {
    getCoachSetup.mockResolvedValue(setup({ groups: [{ id: 'group-1', name: 'Léa · Marathon' }] }));
    listInvites.mockResolvedValue({
      invites: [
        {
          id: 'inv-1',
          code: 'JOIN-1',
          kind: 'member',
          is_active: true,
          expires_at: null,
          max_uses: null,
          use_count: 0,
          created_at: '2026-11-12T10:00:00Z',
        },
      ],
    });
    renderHome();

    expect(await screen.findByTestId('coach-setup-invite-status')).toHaveTextContent(
      'Léa · Marathon · Invited · hasn’t joined yet',
    );
    expect(screen.getByTestId('coach-setup-copy-link')).toHaveTextContent('Copy the link again');
  });

  it('ticks what the server says is done: who joined and when, and the platform linked', async () => {
    getCoachSetup.mockResolvedValue(
      setup({
        rows: [
          { id: 'seat', order: 1, done: false, current: true },
          { id: 'invite', order: 2, done: true, current: false },
          { id: 'platform', order: 3, done: true, current: false },
        ],
        done_count: 2,
        groups: [{ id: 'group-1', name: 'Léa · Marathon' }],
        athlete: { name: 'Léa Gagnon', group_id: 'group-1', joined_at: '2026-11-10T09:00:00Z' },
        platform: 'sciotte_trainingpeaks',
      }),
    );
    renderHome();

    const invite = await screen.findByTestId('coach-setup-row-invite');
    expect(invite).toHaveAttribute('data-state', 'done');
    expect(within(invite).getByText(/· done/)).toBeInTheDocument();
    expect(screen.getByTestId('coach-setup-athlete-joined')).toHaveTextContent('Léa Gagnon joined your group on Nov 10.');
    expect(screen.getByTestId('coach-setup-platform-linked')).toHaveTextContent('TrainingPeaks is linked.');
    expect(screen.queryByTestId('provider-cards')).toBeNull();
    expect(screen.getByTestId('coach-setup-count')).toHaveTextContent('2 of 3');
  });

  it('gives only a coach who trains the switch to their own Home', async () => {
    const { navigate } = renderHome(true);
    await userEvent.click(await screen.findByRole('button', { name: 'My training' }));
    expect(navigate).toHaveBeenCalledWith('home/me');
  });

  it('titles the page « Mes athlètes » for a coach who does not train, with no switch', async () => {
    renderHome(false);
    expect(await screen.findByRole('heading', { level: 1, name: 'My athletes' })).toBeInTheDocument();
    expect(screen.queryByTestId('coach-home-switch')).toBeNull();
  });

  it('says the checklist could not be read, with a retry', async () => {
    getCoachSetup.mockRejectedValue(new Error('down'));
    renderHome();
    const failed = await screen.findByTestId('pending-coach-home-failed');
    expect(within(failed).getByText("We couldn't load your checklist. Please try again.")).toBeInTheDocument();
  });
});
