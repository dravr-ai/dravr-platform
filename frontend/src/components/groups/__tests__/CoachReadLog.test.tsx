// ABOUTME: Tests for the member's record of their human coach's reads in Group info — who, which kind, through what, when
// ABOUTME: Lists the server's rows as they come, names a coach who left Dravr, and says so when there are none or the read failed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachReads } from '@pierre/shared-types';
import CoachReadLog from '../CoachReadLog';

const GROUP_ID = 'group-1';

vi.mock('../../../services/api', () => ({
  groupsApi: {
    getCoachReads: vi.fn(),
  },
}));

const { groupsApi } = await import('../../../services/api');

function renderLog() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <CoachReadLog groupId={GROUP_ID} />
    </QueryClientProvider>,
  );
}

const TWO_READS: CoachReads = {
  retention_months: 12,
  reads: [
    {
      coach_name: 'Casey Coach',
      tool: 'get_sleep_sessions',
      data_class: 'wellness',
      read_at: '2026-11-12T14:05:00Z',
    },
    {
      coach_name: null,
      tool: 'get_activities',
      data_class: 'training',
      read_at: '2026-11-10T08:30:00Z',
    },
  ],
};

describe('CoachReadLog', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("asks the server for the member's own reads in this group", async () => {
    vi.mocked(groupsApi.getCoachReads).mockResolvedValue(TWO_READS);
    renderLog();

    await screen.findAllByTestId('coach-read-row');
    expect(groupsApi.getCoachReads).toHaveBeenCalledWith(GROUP_ID);
  });

  it('lists each read in the order the server gave, with who, which tool and when', async () => {
    vi.mocked(groupsApi.getCoachReads).mockResolvedValue(TWO_READS);
    renderLog();

    const rows = await screen.findAllByTestId('coach-read-row');
    expect(rows).toHaveLength(2);
    const [newest, older] = rows;

    expect(within(newest).getByTestId('coach-read-coach')).toHaveTextContent('Casey Coach');
    expect(within(newest).getByTestId('coach-read-tool')).toHaveTextContent('get_sleep_sessions');
    expect(within(newest).getByTestId('coach-read-at')).toHaveAttribute('datetime', '2026-11-12T14:05:00Z');
    expect(within(older).getByTestId('coach-read-tool')).toHaveTextContent('get_activities');
    expect(within(older).getByTestId('coach-read-at')).toHaveAttribute('datetime', '2026-11-10T08:30:00Z');

    // Wellness and training are told apart, each in its own words.
    expect(within(newest).getByTestId('coach-read-class').textContent).not.toBe(
      within(older).getByTestId('coach-read-class').textContent,
    );
  });

  it('words a read made outside a tool call instead of showing its label', async () => {
    vi.mocked(groupsApi.getCoachReads).mockResolvedValue({
      retention_months: 12,
      reads: [
        { coach_name: 'Casey Coach', tool: 'coach_thread', data_class: 'training', read_at: '2026-11-12T14:05:00Z' },
        { coach_name: 'Casey Coach', tool: 'roster', data_class: 'training', read_at: '2026-11-11T09:00:00Z' },
      ],
    });
    renderLog();

    const rows = await screen.findAllByTestId('coach-read-row');
    const thread = within(rows[0]).getByTestId('coach-read-tool');
    const roster = within(rows[1]).getByTestId('coach-read-tool');
    // The server's label is not a tool name: it is never shown as code.
    expect(thread.tagName).not.toBe('CODE');
    expect(thread).not.toHaveTextContent(/^coach_thread$/);
    expect(roster).not.toHaveTextContent(/^roster$/);
    expect(thread.textContent).not.toBe(roster.textContent);
  });

  it('names a coach whose account is gone without leaving the row blank', async () => {
    vi.mocked(groupsApi.getCoachReads).mockResolvedValue(TWO_READS);
    renderLog();

    const rows = await screen.findAllByTestId('coach-read-row');
    const formerCoach = within(rows[1]).getByTestId('coach-read-coach');
    expect(formerCoach.textContent).not.toBe('');
    expect(formerCoach).not.toHaveTextContent('Casey Coach');
  });

  it('says so when the coach has read nothing', async () => {
    vi.mocked(groupsApi.getCoachReads).mockResolvedValue({ reads: [], retention_months: 12 });
    renderLog();

    expect(await screen.findByTestId('coach-read-log-empty')).toBeInTheDocument();
    expect(screen.queryByTestId('coach-read-row')).toBeNull();
  });

  it('says the record could not be read rather than showing it empty', async () => {
    vi.mocked(groupsApi.getCoachReads).mockRejectedValue(new Error('Request failed with status code 500'));
    renderLog();

    expect(await screen.findByTestId('coach-read-log-error')).toBeInTheDocument();
    expect(screen.queryByTestId('coach-read-log-empty')).toBeNull();
  });
});
