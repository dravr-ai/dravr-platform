// ABOUTME: Tests for the Room section of Group info — the shared room read, a withheld entry kept as a placeholder
// ABOUTME: Pins that an unconsented member's entry shows as a line naming no one, never as a gap or as their words
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { GroupTranscriptResponse } from '@pierre/shared-types';
import GroupTranscriptPanel from '../GroupTranscriptPanel';

const getTranscript = vi.fn();

vi.mock('../../../services/api', () => ({
  groupsApi: { getTranscript: (...a: unknown[]) => getTranscript(...a) },
}));

function renderPanel() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <GroupTranscriptPanel groupId="group-1" />
    </QueryClientProvider>,
  );
}

describe('GroupTranscriptPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('keeps a withheld entry in the room as a placeholder, between the lines around it', async () => {
    const transcript: GroupTranscriptResponse = {
      group_id: 'group-1',
      members: [],
      entries: [
        {
          id: 'e1',
          speaker: 'member',
          withheld: false,
          own: false,
          author_user_id: 'user-bob',
          author_display_name: 'Bob',
          content: 'Long run at 7?',
          message_id: null,
          created_at: '2026-09-14T07:00:00Z',
        },
        {
          id: 'e2',
          speaker: 'coach',
          withheld: true,
          own: false,
          author_user_id: null,
          author_display_name: null,
          content: null,
          message_id: null,
          created_at: '2026-09-14T07:01:00Z',
        },
        {
          id: 'e3',
          speaker: 'member',
          withheld: true,
          own: false,
          author_user_id: null,
          author_display_name: null,
          content: null,
          message_id: null,
          created_at: '2026-09-14T07:02:00Z',
        },
      ],
    };
    getTranscript.mockResolvedValue(transcript);

    renderPanel();

    expect(await screen.findByText('Long run at 7?')).toBeInTheDocument();
    const placeholders = screen.getAllByTestId('room-entry-withheld').map((p) => p.textContent);
    expect(placeholders).toEqual([
      "The agent's reply to a member is hidden: sharing not enabled",
      "A member's message is hidden: sharing not enabled",
    ]);
    expect(getTranscript).toHaveBeenCalledWith('group-1');
  });
});
