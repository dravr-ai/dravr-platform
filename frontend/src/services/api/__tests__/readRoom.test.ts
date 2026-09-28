// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for readRoom — how a group thread pages the room back to its own oldest row
// ABOUTME: Drives the shared groups client over a stubbed axios get, asserting the cursor it sends and the order it returns

import { describe, it, expect, vi, afterEach } from 'vitest';
import type { AxiosResponse } from 'axios';
import type { GroupTranscriptEntry, GroupTranscriptResponse } from '@pierre/shared-types';

import { groupsApi, pierreApi } from '../index';

const GROUP_ID = 'group-7';
/** The most one page of the transcript route holds. */
const PAGE = 200;

/** `count` entries, one a minute, ending at `endMinute` minutes past 08:00 — oldest first, as a page arrives. */
function page(prefix: string, endMinute: number, count: number): GroupTranscriptEntry[] {
  return Array.from({ length: count }, (_, i) => {
    const minute = endMinute - (count - 1 - i);
    return {
      id: `${prefix}-${i}`,
      speaker: 'member' as const,
      withheld: false,
      own: false,
      author_user_id: 'user-bob',
      author_display_name: 'Bob',
      content: `${prefix} line ${i}`,
      message_id: null,
      created_at: new Date(Date.UTC(2026, 8, 14, 8, minute)).toISOString(),
    };
  });
}

function reply(entries: GroupTranscriptEntry[]): AxiosResponse<GroupTranscriptResponse> {
  return {
    data: { group_id: GROUP_ID, members: [], entries },
    status: 200,
    statusText: 'OK',
    headers: {},
    config: {} as AxiosResponse['config'],
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('groupsApi.readRoom', () => {
  it('reads only the newest page when the thread has no rows of its own', async () => {
    const get = vi.spyOn(pierreApi.axios, 'get').mockResolvedValueOnce(reply(page('new', 500, PAGE)));

    const room = await groupsApi.readRoom(GROUP_ID, null);

    expect(room).toHaveLength(PAGE);
    expect(get).toHaveBeenCalledTimes(1);
    expect(get).toHaveBeenCalledWith(`/api/chat/groups/${GROUP_ID}/transcript`, {
      params: { limit: PAGE },
    });
  });

  it('pages back from the oldest entry held until a page reaches the thread\'s own oldest row', async () => {
    const newest = page('new', 500, PAGE);
    const older = page('old', 300, PAGE);
    const get = vi
      .spyOn(pierreApi.axios, 'get')
      .mockResolvedValueOnce(reply(newest))
      .mockResolvedValueOnce(reply(older));
    // The thread's own oldest row sits inside the second page.
    const since = new Date(Date.UTC(2026, 8, 14, 8, 250)).toISOString();

    const room = await groupsApi.readRoom(GROUP_ID, since);

    expect(get).toHaveBeenCalledTimes(2);
    expect(get).toHaveBeenNthCalledWith(2, `/api/chat/groups/${GROUP_ID}/transcript`, {
      params: { limit: PAGE, before: 'new-0' },
    });
    // Oldest first across pages: the older page, then the newest.
    expect(room).toHaveLength(PAGE * 2);
    expect(room[0].id).toBe('old-0');
    expect(room[PAGE - 1].id).toBe(`old-${PAGE - 1}`);
    expect(room[PAGE].id).toBe('new-0');
    expect(room[PAGE * 2 - 1].content).toBe(`new line ${PAGE - 1}`);
  });

  it('stops when the room runs out, however far back the thread reaches', async () => {
    const get = vi
      .spyOn(pierreApi.axios, 'get')
      .mockResolvedValueOnce(reply(page('new', 500, PAGE)))
      .mockResolvedValueOnce(reply(page('first', 300, 3)));
    const since = new Date(Date.UTC(2026, 8, 1, 0, 0)).toISOString();

    const room = await groupsApi.readRoom(GROUP_ID, since);

    expect(get).toHaveBeenCalledTimes(2);
    expect(room.map((entry) => entry.id).slice(0, 3)).toEqual(['first-0', 'first-1', 'first-2']);
    expect(room).toHaveLength(PAGE + 3);
  });

  it('hands a withheld entry back as the placeholder the server sent, never dropped', async () => {
    const withheld: GroupTranscriptEntry = {
      id: 'hidden-1',
      speaker: 'member',
      withheld: true,
      own: false,
      author_user_id: null,
      author_display_name: null,
      content: null,
      message_id: null,
      created_at: '2026-09-14T09:00:00Z',
    };
    vi.spyOn(pierreApi.axios, 'get').mockResolvedValueOnce(reply([withheld]));

    const room = await groupsApi.readRoom(GROUP_ID, '2026-09-14T08:00:00Z');

    expect(room).toEqual([withheld]);
  });
});
