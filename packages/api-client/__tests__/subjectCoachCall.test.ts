// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for the coach's call on a session of their draft — the route, the body it puts, the answer it parses (carnet#1814)
// ABOUTME: Runs the real chat domain over a stub axios, so web and mobile share what is pinned here

import { describe, expect, it, vi } from 'vitest';
import type { AxiosInstance } from 'axios';
import { createChatApi } from '../src/domains/chat';
import type { PlatformAdapter } from '../src/types/platform';

const EXPECT = { sport: 'run', workout: 'Tempo 3 × 10' };

/** The subject plan the route answers: the coach's draft opened, no card worth reading here. */
const ANSWERED = {
  athlete: { user_id: 'athlete-1', display_name: 'Nadia' },
  today: '2026-11-24',
  live: null,
  draft: null,
};

/** The chat domain over a stub `put`; the call reads nothing of the platform adapter. */
function stubChat(data: unknown) {
  const put = vi.fn().mockResolvedValue({ data });
  const api = createChatApi({ put } as unknown as AxiosInstance, {} as PlatformAdapter);
  return { put, api };
}

describe('setSubjectDraftDayCoachCall', () => {
  it('puts the call and the session the card showed to that session of the date', async () => {
    const { put, api } = stubChat(ANSWERED);
    const answered = await api.setSubjectDraftDayCoachCall('conv-1', '2026-11-26', 1, true, EXPECT, 'fr');
    expect(put).toHaveBeenCalledWith(
      '/api/chat/conversations/conv-1/subject-plan/draft/days/2026-11-26/1/coach-call',
      { on: true, expect: EXPECT },
      { params: { locale: 'fr' } },
    );
    expect(answered.athlete.display_name).toBe('Nadia');
  });

  it('takes the call off with on: false and sends no locale it was not given', async () => {
    const { put, api } = stubChat(ANSWERED);
    await api.setSubjectDraftDayCoachCall('conv-1', '2026-11-26', 0, false, EXPECT);
    expect(put).toHaveBeenCalledWith(
      '/api/chat/conversations/conv-1/subject-plan/draft/days/2026-11-26/0/coach-call',
      { on: false, expect: EXPECT },
      { params: undefined },
    );
  });

  it('refuses an answer that is not the subject plan', async () => {
    const { api } = stubChat({ ok: true });
    await expect(api.setSubjectDraftDayCoachCall('conv-1', '2026-11-26', 0, true, EXPECT)).rejects.toThrow(
      'does not match its contract',
    );
  });
});
