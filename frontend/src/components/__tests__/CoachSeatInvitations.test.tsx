// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the admin console's coach-seat invitations (carnet#1524) mounted in the coach-access queue
// ABOUTME: Asserts the invited rows and their expiry, issuing, re-sending, revoking, and the direct grant offered for an existing account

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import CoachSeatInvitations from '../CoachSeatInvitations';

vi.mock('../../services/api', () => ({
  coachSeatInvitationsApi: {
    list: vi.fn(),
    invite: vi.fn(),
    revoke: vi.fn(),
  },
  adminApi: {
    setUserManagesRoster: vi.fn(),
  },
}));

const { coachSeatInvitationsApi, adminApi } = await import('../../services/api');

const INVITED = {
  email: 'philippe@example.com',
  invited_by: 'op-1',
  invited_by_email: 'team@dravr.ai',
  created_at: '2026-11-13T14:40:00Z',
  expires_at: '2026-11-20T14:40:00Z',
  expired: false,
};

const EXPIRED = {
  ...INVITED,
  email: 'late@example.com',
  expires_at: '2026-11-01T10:00:00Z',
  expired: true,
};

function renderView() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <CoachSeatInvitations />
    </QueryClientProvider>,
  );
}

describe('CoachSeatInvitations', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('lists each invited coach as an invited row with its expiry and who sent it', async () => {
    vi.mocked(coachSeatInvitationsApi.list).mockResolvedValue([INVITED, EXPIRED]);

    renderView();

    const row = await screen.findByTestId('coach-seat-invitation-philippe@example.com');
    expect(within(row).getByText('invited')).toBeTruthy();
    expect(row.textContent).toContain('Expires Nov 20, 2026');
    expect(row.textContent).toContain('team@dravr.ai');
    const late = screen.getByTestId('coach-seat-invitation-late@example.com');
    expect(within(late).getByText('expired')).toBeTruthy();
    expect(late.textContent).toContain('Expired Nov 1, 2026');
    expect(screen.getByText('Invited (2)')).toBeTruthy();
  });

  it('says so when no coach is invited', async () => {
    vi.mocked(coachSeatInvitationsApi.list).mockResolvedValue([]);
    renderView();
    expect(await screen.findByText('No coach is invited right now.')).toBeTruthy();
  });

  it('sends an invitation to the typed address and shows what the server said', async () => {
    vi.mocked(coachSeatInvitationsApi.list).mockResolvedValue([]);
    vi.mocked(coachSeatInvitationsApi.invite).mockResolvedValue({
      outcome: 'invited',
      message: 'Coach-seat invitation sent to coach@example.com: its link works once, until 2026-11-20 14:40 UTC',
      email: 'coach@example.com',
      expiresAt: '2026-11-20T14:40:00Z',
    });
    renderView();

    fireEvent.change(await screen.findByLabelText("Coach's email address"), { target: { value: 'coach@example.com' } });
    fireEvent.click(screen.getByTestId('coach-seat-invite-send'));

    await waitFor(() => expect(coachSeatInvitationsApi.invite).toHaveBeenCalledWith('coach@example.com'));
    expect((await screen.findByTestId('coach-seat-invite-result')).textContent).toContain(
      'Coach-seat invitation sent to coach@example.com',
    );
    await waitFor(() => expect(coachSeatInvitationsApi.list).toHaveBeenCalledTimes(2));
  });

  it('offers the existing direct grant for an address that already has an account', async () => {
    vi.mocked(coachSeatInvitationsApi.list).mockResolvedValue([]);
    vi.mocked(coachSeatInvitationsApi.invite).mockResolvedValue({
      outcome: 'account_exists',
      message: 'This address already has an account: grant it coach access directly instead',
      email: 'member@example.com',
      userId: 'u-9',
      displayName: null,
      managesRoster: false,
    });
    vi.mocked(adminApi.setUserManagesRoster).mockResolvedValue({
      message: 'ok',
    } as Awaited<ReturnType<typeof adminApi.setUserManagesRoster>>);
    renderView();

    fireEvent.change(await screen.findByLabelText("Coach's email address"), { target: { value: 'member@example.com' } });
    fireEvent.click(screen.getByTestId('coach-seat-invite-send'));

    const grant = await screen.findByTestId('coach-seat-invite-grant');
    expect(grant.textContent).toBe('Grant coach access to member@example.com');
    fireEvent.click(grant);
    await waitFor(() => expect(adminApi.setUserManagesRoster).toHaveBeenCalledWith('u-9', true));
    expect((await screen.findByTestId('coach-seat-invite-result')).textContent).toBe(
      'member@example.com now has coach access',
    );
  });

  it('re-sends and revokes from the row', async () => {
    vi.mocked(coachSeatInvitationsApi.list).mockResolvedValue([INVITED]);
    vi.mocked(coachSeatInvitationsApi.invite).mockResolvedValue({
      outcome: 'invited',
      message: 'sent again',
      email: INVITED.email,
      expiresAt: INVITED.expires_at,
    });
    vi.mocked(coachSeatInvitationsApi.revoke).mockResolvedValue({ message: 'revoked', revoked: true });
    renderView();

    const row = await screen.findByTestId('coach-seat-invitation-philippe@example.com');
    fireEvent.click(within(row).getByRole('button', { name: 'Send again' }));
    await waitFor(() => expect(coachSeatInvitationsApi.invite).toHaveBeenCalledWith('philippe@example.com'));
    await screen.findByText('sent again');

    fireEvent.click(within(row).getByRole('button', { name: 'Revoke' }));
    await waitFor(() => expect(coachSeatInvitationsApi.revoke).toHaveBeenCalledWith('philippe@example.com'));
    expect(await screen.findByText('revoked')).toBeTruthy();
  });

  it('shows the server refusal when the invitation cannot be sent', async () => {
    vi.mocked(coachSeatInvitationsApi.list).mockResolvedValue([]);
    vi.mocked(coachSeatInvitationsApi.invite).mockRejectedValue({
      response: { data: { message: 'No invitation sent: 5 already went to this address in the last hour' } },
    });
    renderView();

    fireEvent.change(await screen.findByLabelText("Coach's email address"), { target: { value: 'busy@example.com' } });
    fireEvent.click(screen.getByTestId('coach-seat-invite-send'));

    const result = await screen.findByTestId('coach-seat-invite-result');
    expect(result.textContent).toBe('No invitation sent: 5 already went to this address in the last hour');
    expect(result.className).toContain('text-error');
  });
});
