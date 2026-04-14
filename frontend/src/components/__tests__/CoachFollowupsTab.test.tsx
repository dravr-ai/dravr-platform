// ABOUTME: Sprint C7 tests — CoachFollowupsTab renders pending followups and cancels via dialog
// ABOUTME: Mocks adminApi.listPendingFollowups/cancelFollowup and asserts list + cancel UX
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import CoachFollowupsTab from '../CoachFollowupsTab';
import type { FollowupRow } from '../../services/api/admin';

vi.mock('../../services/api/admin', async () => ({
  adminApi: {
    listPendingFollowups: vi.fn(),
    cancelFollowup: vi.fn(),
  },
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: {
      id: 'user-1',
      email: 'admin@example.com',
      tenant_id: 'tenant-a',
      is_admin: true,
      role: 'admin',
      tier: 'professional',
      created_at: '2026-01-01T00:00:00Z',
    },
  }),
}));

const { adminApi } = await import('../../services/api/admin');

function sampleFollowup(overrides: Partial<FollowupRow> = {}): FollowupRow {
  return {
    id: 'f1',
    tenant_id: 'tenant-a',
    user_id: 'user-42',
    coach_id: 'coach-strength',
    conversation_id: 'conv-7',
    content: 'Check on Achilles pain tomorrow',
    due_at: '2026-04-14T08:00:00Z',
    status: 'pending',
    created_at: '2026-04-13T18:00:00Z',
    updated_at: '2026-04-13T18:00:00Z',
    delivered_at: null,
    ...overrides,
  };
}

function renderTab() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <CoachFollowupsTab />
    </QueryClientProvider>,
  );
}

describe('CoachFollowupsTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the empty state when no followups are pending', async () => {
    vi.mocked(adminApi.listPendingFollowups).mockResolvedValueOnce({
      followups: [],
      total: 0,
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/No pending followups/i)).toBeInTheDocument();
    });
  });

  it('renders followup rows with content and coach id', async () => {
    vi.mocked(adminApi.listPendingFollowups).mockResolvedValueOnce({
      followups: [
        sampleFollowup(),
        sampleFollowup({
          id: 'f2',
          content: 'Ask about taper week',
          coach_id: 'coach-endurance',
          user_id: 'user-7',
        }),
      ],
      total: 2,
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/Check on Achilles pain tomorrow/i)).toBeInTheDocument();
    });
    expect(screen.getByText(/Ask about taper week/i)).toBeInTheDocument();
    expect(screen.getByText(/coach-strength/)).toBeInTheDocument();
    expect(screen.getByText(/coach-endurance/)).toBeInTheDocument();
  });

  it('opens the confirm dialog and calls cancelFollowup on confirm', async () => {
    vi.mocked(adminApi.listPendingFollowups).mockResolvedValue({
      followups: [sampleFollowup()],
      total: 1,
    });
    vi.mocked(adminApi.cancelFollowup).mockResolvedValueOnce({ cancelled: true });
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/Check on Achilles pain tomorrow/i)).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: /^Cancel$/i }));
    expect(await screen.findByText(/Cancel this followup/i)).toBeInTheDocument();
    const confirmButtons = await screen.findAllByRole('button', { name: /Cancel followup/i });
    fireEvent.click(confirmButtons[confirmButtons.length - 1]);
    await waitFor(() => {
      expect(adminApi.cancelFollowup).toHaveBeenCalledWith('f1', 'tenant-a');
    });
  });

  it('filters rows by coach id', async () => {
    vi.mocked(adminApi.listPendingFollowups).mockResolvedValueOnce({
      followups: [
        sampleFollowup({ id: 'f1', coach_id: 'coach-strength' }),
        sampleFollowup({
          id: 'f2',
          content: 'Taper week reminder',
          coach_id: 'coach-endurance',
        }),
      ],
      total: 2,
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/Check on Achilles/)).toBeInTheDocument();
    });
    const coachInput = screen.getByPlaceholderText(/filter by coach id/i);
    fireEvent.change(coachInput, { target: { value: 'endurance' } });
    await waitFor(() => {
      expect(screen.queryByText(/Check on Achilles/)).not.toBeInTheDocument();
    });
    expect(screen.getByText(/Taper week reminder/)).toBeInTheDocument();
  });
});
