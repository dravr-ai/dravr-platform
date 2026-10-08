// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the admin console's coach-access queue (carnet#738)
// ABOUTME: Asserts the queue lists who asked and for which group, and that grant/decline call the API and refresh

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import CoachAccessRequests from '../CoachAccessRequests';

vi.mock('../../services/api', () => ({
  coachAccessAdminApi: {
    list: vi.fn(),
    grant: vi.fn(),
    decline: vi.fn(),
  },
}));

const { coachAccessAdminApi } = await import('../../services/api');

const base = {
  user_id: 'u-1',
  group_tenant_id: 't-1',
  status: 'pending' as const,
  created_at: '2026-10-07T12:00:00Z',
  decided_at: null,
  decided_by: null,
};

const WITH_GROUP = {
  ...base,
  id: 'r-1',
  group_id: 'g-1',
  email: 'coach@example.com',
  display_name: 'Phil',
  group_name: 'Les Rouleurs',
};

const WITHOUT_GROUP = {
  ...base,
  id: 'r-2',
  user_id: 'u-2',
  group_id: null,
  group_tenant_id: null,
  email: 'other@example.com',
  display_name: null,
  group_name: null,
};

function renderView() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <CoachAccessRequests />
    </QueryClientProvider>,
  );
}

describe('CoachAccessRequests', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('lists each pending request with who asked and for which group', async () => {
    vi.mocked(coachAccessAdminApi.list).mockResolvedValue([WITH_GROUP, WITHOUT_GROUP]);

    renderView();

    expect(await screen.findByText('Phil · coach@example.com')).toBeTruthy();
    expect(screen.getByText('Group: Les Rouleurs')).toBeTruthy();
    expect(screen.getByText('other@example.com')).toBeTruthy();
    expect(screen.getByText('No group named')).toBeTruthy();
    expect(screen.getByText('Coach access requests (2)')).toBeTruthy();
    expect(coachAccessAdminApi.list).toHaveBeenCalledWith('pending');
  });

  it('says so when no coach is waiting', async () => {
    vi.mocked(coachAccessAdminApi.list).mockResolvedValue([]);
    renderView();
    expect(await screen.findByText('No coach is waiting for access.')).toBeTruthy();
  });

  it('grants a request and reloads the queue', async () => {
    vi.mocked(coachAccessAdminApi.list)
      .mockResolvedValueOnce([WITH_GROUP])
      .mockResolvedValue([]);
    vi.mocked(coachAccessAdminApi.grant).mockResolvedValue({
      message: 'Coach access granted; the coach now coaches their group',
      request: { ...WITH_GROUP, status: 'granted' },
      attachedGroupId: 'g-1',
    });

    renderView();
    fireEvent.click(await screen.findByRole('button', { name: 'Grant' }));

    await waitFor(() => expect(coachAccessAdminApi.grant).toHaveBeenCalledWith('r-1'));
    expect(
      await screen.findByText('Coach access granted; the coach now coaches their group'),
    ).toBeTruthy();
    expect(await screen.findByText('No coach is waiting for access.')).toBeTruthy();
  });

  it('declines a request', async () => {
    vi.mocked(coachAccessAdminApi.list).mockResolvedValue([WITH_GROUP]);
    vi.mocked(coachAccessAdminApi.decline).mockResolvedValue({
      message: 'Coach access request declined',
      request: { ...WITH_GROUP, status: 'declined' },
    });

    renderView();
    fireEvent.click(await screen.findByRole('button', { name: 'Decline' }));

    await waitFor(() => expect(coachAccessAdminApi.decline).toHaveBeenCalledWith('r-1'));
    expect(await screen.findByText('Coach access request declined')).toBeTruthy();
  });

  it("shows the server's refusal when a request was already decided", async () => {
    vi.mocked(coachAccessAdminApi.list).mockResolvedValue([WITH_GROUP]);
    vi.mocked(coachAccessAdminApi.grant).mockRejectedValue({
      response: { data: { message: 'This coach access request was already decided' } },
    });

    renderView();
    fireEvent.click(await screen.findByRole('button', { name: 'Grant' }));

    expect(
      await screen.findByText('This coach access request was already decided'),
    ).toBeTruthy();
  });
});
