// ABOUTME: Tests the admin console's pre-approval allow-list view
// ABOUTME: Asserts the list renders operator + registration state, and that allow/remove call the API
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import PreApprovedEmails from '../PreApprovedEmails';

vi.mock('../../services/api', () => ({
  adminApi: {
    getPreApprovedEmails: vi.fn(),
    allowEmail: vi.fn(),
    disallowEmail: vi.fn(),
  },
}));

const { adminApi } = await import('../../services/api');

const WAITING = {
  email: 'alpha@example.com',
  note: 'alpha cohort',
  created_at: '2026-08-20T12:00:00Z',
  allowed_by: 'operator-1',
  allowed_by_email: 'admin@example.com',
  account_status: null,
};

const REGISTERED = {
  email: 'already@example.com',
  note: null,
  created_at: '2026-08-21T12:00:00Z',
  allowed_by: null,
  allowed_by_email: null,
  account_status: 'active' as const,
};

function renderView() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <PreApprovedEmails />
    </QueryClientProvider>,
  );
}

describe('PreApprovedEmails', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('lists each allow with its note, operator, and registration state', async () => {
    vi.mocked(adminApi.getPreApprovedEmails).mockResolvedValue([WAITING, REGISTERED]);

    renderView();

    expect(await screen.findByText('alpha@example.com')).toBeTruthy();
    expect(screen.getByText('alpha cohort')).toBeTruthy();
    expect(screen.getByText('By: admin@example.com')).toBeTruthy();
    // A standing allow nobody has registered against is the normal state, and
    // must read as such rather than as a blank cell.
    expect(screen.getByText('not registered')).toBeTruthy();

    expect(screen.getByText('already@example.com')).toBeTruthy();
    expect(screen.getByText('active')).toBeTruthy();
    expect(screen.getByText('By: unattributed')).toBeTruthy();
  });

  it('allows an address with its note and reports the server message', async () => {
    vi.mocked(adminApi.getPreApprovedEmails).mockResolvedValue([]);
    vi.mocked(adminApi.allowEmail).mockResolvedValue({
      message: 'newcomer@example.com pre-approved — their registration will land active',
      outcome: 'recorded',
      email: 'newcomer@example.com',
      approved_user_id: null,
    });

    renderView();

    fireEvent.change(await screen.findByLabelText('Email address to pre-approve'), {
      target: { value: 'newcomer@example.com' },
    });
    fireEvent.change(screen.getByLabelText('Note recorded with the pre-approval'), {
      target: { value: 'beta cohort' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Allow' }));

    await waitFor(() =>
      expect(adminApi.allowEmail).toHaveBeenCalledWith('newcomer@example.com', 'beta cohort'),
    );
    expect(
      await screen.findByText(
        'newcomer@example.com pre-approved — their registration will land active',
      ),
    ).toBeTruthy();
  });

  it('removes an allow through the API', async () => {
    vi.mocked(adminApi.getPreApprovedEmails).mockResolvedValue([WAITING]);
    vi.mocked(adminApi.disallowEmail).mockResolvedValue({
      message: 'alpha@example.com removed from the pre-approved list',
      removed: true,
    });

    renderView();

    fireEvent.click(await screen.findByRole('button', { name: 'Remove' }));

    await waitFor(() => expect(adminApi.disallowEmail).toHaveBeenCalledWith('alpha@example.com'));
    expect(
      await screen.findByText('alpha@example.com removed from the pre-approved list'),
    ).toBeTruthy();
  });

  // The browser's own `type="email"` check catches the obvious typos, so the
  // rejection that reaches this path is the server's stricter one — it has to
  // be shown verbatim, or an operator sees a form that silently did nothing.
  it('surfaces the server rejection instead of a generic failure', async () => {
    vi.mocked(adminApi.getPreApprovedEmails).mockResolvedValue([]);
    vi.mocked(adminApi.allowEmail).mockRejectedValue({
      response: { data: { message: "'athlete@example' is not a valid email address" } },
    });

    renderView();

    fireEvent.change(await screen.findByLabelText('Email address to pre-approve'), {
      target: { value: 'athlete@example' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Allow' }));

    expect(
      await screen.findByText("'athlete@example' is not a valid email address"),
    ).toBeTruthy();
  });
});
