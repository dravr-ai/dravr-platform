// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the super-admin impersonation log — the sessions it lists and one session's details
// ABOUTME: The admin API is mocked; assertions read the rendered rows and the per-session read

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ImpersonationLogTab from '../ImpersonationLogTab';

const listImpersonationSessions = vi.fn();
const getImpersonationSession = vi.fn();

vi.mock('../../services/api/admin', () => ({
  adminApi: {
    listImpersonationSessions: (...a: unknown[]) => listImpersonationSessions(...a),
    getImpersonationSession: (...a: unknown[]) => getImpersonationSession(...a),
  },
}));

const ENDED = {
  id: 'imp-1',
  impersonator_id: 'op-1',
  impersonator_email: 'op@dravr.ai',
  target_user_id: 'u-1',
  target_user_email: 'athlete@example.com',
  reason: 'Support ticket 4412',
  started_at: '2026-09-27T09:00:00Z',
  ended_at: '2026-09-27T09:12:00Z',
  is_active: false,
  duration_seconds: 720,
};

function renderTab() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <ImpersonationLogTab />
    </QueryClientProvider>,
  );
}

describe('ImpersonationLogTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('lists each session: who, as whom, how long, and whether it ended', async () => {
    listImpersonationSessions.mockResolvedValue({
      sessions: [
        { ...ENDED, id: 'imp-2', target_user_email: null, ended_at: null, is_active: true, duration_seconds: 60 },
        ENDED,
      ],
      total_count: 2,
    });
    renderTab();

    const ended = await screen.findByTestId('impersonation-session-imp-1');
    expect(ended).toHaveTextContent('op@dravr.ai as athlete@example.com');
    expect(ended).toHaveTextContent('12 min');
    expect(ended).toHaveTextContent('Ended');
    const active = screen.getByTestId('impersonation-session-imp-2');
    expect(active).toHaveTextContent('op@dravr.ai as Unknown user');
    expect(active).toHaveTextContent('Active');
  });

  it("reads one session for its details and shows the reason", async () => {
    listImpersonationSessions.mockResolvedValue({ sessions: [ENDED], total_count: 1 });
    getImpersonationSession.mockResolvedValue(ENDED);
    const user = userEvent.setup();
    renderTab();

    await user.click(await screen.findByTestId('impersonation-details-toggle-imp-1'));

    expect(getImpersonationSession).toHaveBeenCalledWith('imp-1');
    expect(await screen.findByTestId('impersonation-details-imp-1')).toHaveTextContent('Support ticket 4412');
  });

  it('says so when no one has impersonated anyone', async () => {
    listImpersonationSessions.mockResolvedValue({ sessions: [], total_count: 0 });
    renderTab();

    expect(await screen.findByTestId('impersonation-log-empty')).toHaveTextContent('No impersonation sessions yet.');
  });
});
