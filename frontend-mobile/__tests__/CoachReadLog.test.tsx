// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the member's record of their human coach's reads on the phone — who, which kind, through what, when
// ABOUTME: Lists the server's rows as they come, names a coach who left Dravr, and says so when there are none or the read failed

import React from 'react';
import { render, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachReads } from '@pierre/shared-types';

const mockGetCoachReads = jest.fn();
jest.mock('../src/services/api', () => ({
  groupsApi: {
    getCoachReads: (...args: unknown[]) => mockGetCoachReads(...args),
  },
}));

import { CoachReadLog } from '../src/screens/groups/CoachReadLog';

const GROUP_ID = 'group-1';

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

function renderLog() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <CoachReadLog groupId={GROUP_ID} />
    </QueryClientProvider>,
  );
}

/** The text a node renders, nested `Text` children included. */
function textOf(node: { props: { children?: unknown } }): string {
  const flatten = (children: unknown): string => {
    if (children === null || children === undefined || typeof children === 'boolean') return '';
    if (typeof children === 'string' || typeof children === 'number') return String(children);
    if (Array.isArray(children)) return children.map(flatten).join('');
    if (typeof children === 'object' && 'props' in (children as object)) {
      return flatten((children as { props: { children?: unknown } }).props.children);
    }
    return '';
  };
  return flatten(node.props.children);
}

describe('CoachReadLog', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("asks the server for the member's own reads in this group", async () => {
    mockGetCoachReads.mockResolvedValue(TWO_READS);
    const { findAllByTestId } = renderLog();

    await findAllByTestId('coach-read-row');
    expect(mockGetCoachReads).toHaveBeenCalledWith(GROUP_ID);
  });

  it('lists each read in the order the server gave, with who, which tool and which kind', async () => {
    mockGetCoachReads.mockResolvedValue(TWO_READS);
    const { findAllByTestId } = renderLog();

    const rows = await findAllByTestId('coach-read-row');
    expect(rows).toHaveLength(2);
    const [newest, older] = rows;

    expect(textOf(within(newest).getByTestId('coach-read-coach'))).toBe('Casey Coach');
    expect(textOf(within(newest).getByTestId('coach-read-tool'))).toBe('get_sleep_sessions');
    expect(textOf(within(older).getByTestId('coach-read-tool'))).toBe('get_activities');
    expect(textOf(within(newest).getByTestId('coach-read-at'))).not.toBe('');
    // Wellness and training are told apart, each in its own words.
    expect(textOf(within(newest).getByTestId('coach-read-class'))).not.toBe(
      textOf(within(older).getByTestId('coach-read-class')),
    );
  });

  it('words a read made outside a tool call instead of showing its label', async () => {
    mockGetCoachReads.mockResolvedValue({
      retention_months: 12,
      reads: [
        { coach_name: 'Casey Coach', tool: 'coach_thread', data_class: 'training', read_at: '2026-11-12T14:05:00Z' },
        { coach_name: 'Casey Coach', tool: 'roster', data_class: 'training', read_at: '2026-11-11T09:00:00Z' },
      ],
    });
    const { findAllByTestId } = renderLog();

    const rows = await findAllByTestId('coach-read-row');
    const thread = textOf(within(rows[0]).getByTestId('coach-read-tool'));
    const roster = textOf(within(rows[1]).getByTestId('coach-read-tool'));
    // The server's label is not a tool name: each reads in its own words.
    expect(thread).not.toBe('coach_thread');
    expect(roster).not.toBe('roster');
    expect(thread).not.toBe(roster);
  });

  it('names a coach whose account is gone without leaving the row blank', async () => {
    mockGetCoachReads.mockResolvedValue(TWO_READS);
    const { findAllByTestId } = renderLog();

    const rows = await findAllByTestId('coach-read-row');
    const formerCoach = textOf(within(rows[1]).getByTestId('coach-read-coach'));
    expect(formerCoach).not.toBe('');
    expect(formerCoach).not.toBe('Casey Coach');
  });

  it('says so when the coach has read nothing', async () => {
    mockGetCoachReads.mockResolvedValue({ reads: [], retention_months: 12 });
    const { findByTestId, queryByTestId } = renderLog();

    expect(await findByTestId('coach-read-log-empty')).toBeTruthy();
    expect(queryByTestId('coach-read-row')).toBeNull();
  });

  it('says the record could not be read rather than showing it empty', async () => {
    mockGetCoachReads.mockRejectedValue(new Error('Request failed'));
    const { findByTestId, queryByTestId } = renderLog();

    expect(await findByTestId('coach-read-log-error')).toBeTruthy();
    expect(queryByTestId('coach-read-log-empty')).toBeNull();
  });
});
