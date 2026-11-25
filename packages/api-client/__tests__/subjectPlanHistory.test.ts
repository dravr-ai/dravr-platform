// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for a date of the coach's draft put back to the live plan, and the plan's change history (carnet#1815)
// ABOUTME: Runs the real chat domain over a stub axios: the routes, what each sends, and the answers each parses or refuses

import { describe, expect, it, vi } from 'vitest';
import type { AxiosInstance } from 'axios';
import { createChatApi } from '../src/domains/chat';
import type { PlatformAdapter } from '../src/types/platform';

/** The subject plan the revert answers: no card worth reading here. */
const ANSWERED = {
  athlete: { user_id: 'athlete-1', display_name: 'Nadia' },
  today: '2026-11-25',
  live: null,
  draft: null,
};

/** One page of the history, newest first. */
const HISTORY = {
  events: [
    { kind: 'date_revert', source: 'coach', dates: ['2026-11-26'], created_at: '2026-11-25T14:05:00.482913Z' },
    { kind: 'agent_draft', source: 'agent', dates: ['2026-11-24', '2026-11-26'], created_at: '2026-11-25T13:00:00Z' },
  ],
};

/** The chat domain over a stub `post` and `get`; neither call reads the platform adapter. */
function stubChat(data: unknown) {
  const post = vi.fn().mockResolvedValue({ data });
  const get = vi.fn().mockResolvedValue({ data });
  const api = createChatApi({ post, get } as unknown as AxiosInstance, {} as PlatformAdapter);
  return { post, get, api };
}

describe('revertSubjectDraftDate', () => {
  it('posts no body to the date of the draft, in the locale it was given', async () => {
    const { post, api } = stubChat(ANSWERED);
    const answered = await api.revertSubjectDraftDate('conv-1', '2026-11-26', 'fr');
    expect(post).toHaveBeenCalledWith(
      '/api/chat/conversations/conv-1/subject-plan/draft/dates/2026-11-26/revert',
      undefined,
      { params: { locale: 'fr' } },
    );
    expect(answered.athlete.display_name).toBe('Nadia');
  });

  it('sends no locale it was not given, and refuses an answer that is not the subject plan', async () => {
    const { post, api } = stubChat(ANSWERED);
    await api.revertSubjectDraftDate('conv-1', '2026-11-26');
    expect(post).toHaveBeenCalledWith(
      '/api/chat/conversations/conv-1/subject-plan/draft/dates/2026-11-26/revert',
      undefined,
      { params: undefined },
    );
    const refused = stubChat({ ok: true });
    await expect(refused.api.revertSubjectDraftDate('conv-1', '2026-11-26')).rejects.toThrow(
      'does not match its contract',
    );
  });
});

describe('getSubjectPlanHistory', () => {
  it('reads the newest page with no query, and the page before a change with one', async () => {
    const { get, api } = stubChat(HISTORY);
    const newest = await api.getSubjectPlanHistory('conv-1');
    expect(get).toHaveBeenCalledWith('/api/chat/conversations/conv-1/subject-plan/history', { params: undefined });
    expect(newest.events.map((event) => event.kind)).toEqual(['date_revert', 'agent_draft']);
    expect(newest.events[1].dates).toEqual(['2026-11-24', '2026-11-26']);

    await api.getSubjectPlanHistory('conv-1', { before: '2026-11-25T13:00:00Z', limit: 20 });
    expect(get).toHaveBeenLastCalledWith('/api/chat/conversations/conv-1/subject-plan/history', {
      params: { before: '2026-11-25T13:00:00Z', limit: 20 },
    });
  });

  it('refuses a body that is not the history', async () => {
    const { api } = stubChat({ events: [{ kind: 'day_moved', source: 'coach', dates: [], created_at: '2026-11-25T13:00:00Z' }] });
    await expect(api.getSubjectPlanHistory('conv-1')).rejects.toThrow('does not match its contract');
  });
});
