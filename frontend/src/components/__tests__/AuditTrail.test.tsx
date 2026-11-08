// ABOUTME: Tests the admin console audit trail — rows, outcome pills, actor fallbacks, filters and the older-page cursor
// ABOUTME: Mocks adminApi.getAuditEvents and asserts the query each control sends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { formatDateTime } from '@pierre/chat-utils';
import AuditTrail from '../AuditTrail';
import type { AuditEvent } from '../../services/api/admin';

vi.mock('../../services/api', () => ({
  adminApi: { getAuditEvents: vi.fn() },
}));

const { adminApi } = await import('../../services/api');

const DELETE_PRE_APPROVAL: AuditEvent = {
  id: 'ev-1',
  occurred_at: '2026-11-04T20:31:45.123Z',
  action: 'DELETE /api/admin/pre-approved-emails/{email}',
  summary: 'Removed pre-approval for x@y.z',
  outcome: 'success',
  source: 'console',
  actor_user_id: 'op-1',
  actor_email: 'admin@dravr.ai',
  tenant_id: null,
  metadata: {
    method: 'DELETE',
    route: '/api/admin/pre-approved-emails/{email}',
    status: 200,
    params: { email: 'x@y.z' },
  },
};

const SIGNUP_CHANGE: AuditEvent = {
  id: 'ev-2',
  occurred_at: '2026-11-04T19:00:00.000Z',
  action: 'settings.signup.update',
  summary: 'Sign-up settings: daily limit 100 → 1000',
  outcome: 'refused',
  source: 'cli_database',
  actor_user_id: null,
  actor_email: null,
  tenant_id: null,
  metadata: { before: { daily_limit: 100 }, after: { daily_limit: 1000 } },
};

const FAILED_BY_GONE_OPERATOR: AuditEvent = {
  id: 'ev-3',
  occurred_at: '2026-11-03T08:00:00.000Z',
  action: 'user.delete',
  summary: 'Delete user',
  outcome: 'failed',
  source: 'console',
  actor_user_id: 'op-gone',
  actor_email: null,
  tenant_id: null,
  metadata: null,
};

function renderView() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <AuditTrail />
    </QueryClientProvider>,
  );
}

describe('AuditTrail', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('lists each event with its time, outcome pill, actor, source, summary and target', async () => {
    vi.mocked(adminApi.getAuditEvents).mockResolvedValue({
      events: [DELETE_PRE_APPROVAL, SIGNUP_CHANGE, FAILED_BY_GONE_OPERATOR],
      next_before: null,
      next_before_id: null,
    });

    renderView();

    expect(await screen.findByText('Removed pre-approval for x@y.z')).toBeTruthy();
    expect(adminApi.getAuditEvents).toHaveBeenCalledWith({ limit: 50 });

    const first = screen.getByTestId('audit-event-ev-1');
    expect(first.textContent).toContain(formatDateTime(DELETE_PRE_APPROVAL.occurred_at, 'en'));
    expect(first.textContent).toContain('By: admin@dravr.ai');
    expect(first.textContent).toContain('Via: console');
    expect(screen.getByTestId('audit-target-ev-1').textContent).toBe('email: x@y.z');

    // Each outcome is a pill with its own semantic colour.
    expect(screen.getByText('success').className).toContain('badge-success');
    expect(screen.getByText('refused').className).toContain('badge-warning');
    expect(screen.getByText('failed').className).toContain('badge-error');

    // An event with no email says which kind of nobody it was.
    expect(screen.getByTestId('audit-event-ev-2').textContent).toContain('By: pierre-cli (database)');
    expect(screen.getByTestId('audit-event-ev-3').textContent).toContain('By: deleted account');

    expect(screen.queryByRole('button', { name: 'Load older' })).toBeNull();
  });

  it('opens a row onto before → after and the raw metadata', async () => {
    vi.mocked(adminApi.getAuditEvents).mockResolvedValue({ events: [SIGNUP_CHANGE], next_before: null, next_before_id: null });

    renderView();

    fireEvent.click(await screen.findByTestId('audit-details-toggle-ev-2'));

    const details = screen.getByTestId('audit-details-ev-2');
    expect(details.textContent).toContain('Before');
    expect(details.textContent).toContain('"daily_limit": 100');
    expect(details.textContent).toContain('After');
    expect(details.textContent).toContain('"daily_limit": 1000');
    expect(details.textContent).toContain('Recorded details');
  });

  it('sends the outcome and action-prefix filters to the server', async () => {
    vi.mocked(adminApi.getAuditEvents).mockResolvedValue({ events: [], next_before: null, next_before_id: null });

    renderView();
    await screen.findByTestId('audit-empty');

    fireEvent.change(screen.getByLabelText('Outcome'), { target: { value: 'refused' } });
    await waitFor(() =>
      expect(adminApi.getAuditEvents).toHaveBeenLastCalledWith({ limit: 50, outcome: 'refused' }),
    );

    fireEvent.change(screen.getByLabelText('Action starts with'), { target: { value: ' user. ' } });
    fireEvent.click(screen.getByRole('button', { name: 'Filter' }));
    await waitFor(() =>
      expect(adminApi.getAuditEvents).toHaveBeenLastCalledWith({
        limit: 50,
        outcome: 'refused',
        action: 'user.',
      }),
    );
    expect(await screen.findByText('No recorded actions match these filters.')).toBeTruthy();
  });

  it('loads the older page with the instant and id the server returned, so a tied instant is not skipped', async () => {
    vi.mocked(adminApi.getAuditEvents)
      .mockResolvedValueOnce({
        events: [DELETE_PRE_APPROVAL],
        next_before: '2026-11-04T20:31:45.123Z',
        next_before_id: 'ev-1',
      })
      .mockResolvedValueOnce({ events: [FAILED_BY_GONE_OPERATOR], next_before: null, next_before_id: null });

    renderView();

    fireEvent.click(await screen.findByRole('button', { name: 'Load older' }));

    await waitFor(() =>
      expect(adminApi.getAuditEvents).toHaveBeenLastCalledWith({
        limit: 50,
        before: '2026-11-04T20:31:45.123Z',
        before_id: 'ev-1',
      }),
    );
    // The older page is appended under the newer one, and the last page offers no more.
    expect(await screen.findByTestId('audit-event-ev-3')).toBeTruthy();
    expect(screen.getByTestId('audit-event-ev-1')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Load older' })).toBeNull();
  });

  it('says what will appear when nothing has been recorded', async () => {
    vi.mocked(adminApi.getAuditEvents).mockResolvedValue({ events: [], next_before: null, next_before_id: null });

    renderView();

    expect((await screen.findByTestId('audit-empty')).textContent).toMatch(
      /No operator actions recorded yet.*removing a pre-approval.*refused/,
    );
  });
});
