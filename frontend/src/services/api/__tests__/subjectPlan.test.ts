// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for the plan-thread chat calls — open an athlete's thread, read the subject plan, publish, discard
// ABOUTME: Drives the shared chat client over stubbed axios calls, asserting the routes, the bodies, and a 409 read as a result

import { describe, it, expect, vi, afterEach } from 'vitest';
import type { AxiosResponse } from 'axios';
import { chatApi, pierreApi } from '../index';

const CONVERSATION_ID = 'conv-plan-alice';

function answer<T>(data: T, status = 200): AxiosResponse<T> {
  return { data, status, statusText: '', headers: {}, config: {} as AxiosResponse['config'] };
}

const DRAFT_STATE = {
  draft_id: 'draft-7',
  updated_at: '2026-10-16T14:05:00Z',
  conflicts: [],
  season_relaid: false,
};

afterEach(() => {
  vi.restoreAllMocks();
});

describe('chatApi plan-thread calls', () => {
  it('opens an athlete thread with the group it is scoped to', async () => {
    const post = vi
      .spyOn(pierreApi.axios, 'post')
      .mockResolvedValueOnce(answer({ id: CONVERSATION_ID, subject_user_id: 'user-alice' }));

    const thread = await chatApi.openAthleteThread('user-alice', 'group-1');

    expect(post).toHaveBeenCalledWith('/api/chat/athletes/user-alice/thread', { group_id: 'group-1' });
    expect(thread.id).toBe(CONVERSATION_ID);
  });

  it("reads a coached athlete's plan in the group named, and refuses a body that breaks its contract", async () => {
    const body = {
      athlete: { user_id: 'user-alice', display_name: 'Alice Runner' },
      today: '2026-10-28',
      live: null,
    };
    const get = vi.spyOn(pierreApi.axios, 'get').mockResolvedValueOnce(answer(body));

    await expect(chatApi.getAthletePlan('user-alice', 'group-1')).resolves.toEqual(body);
    expect(get).toHaveBeenCalledWith('/api/chat/athletes/user-alice/plan', { params: { group_id: 'group-1' } });

    get.mockResolvedValueOnce(answer({ athlete: { user_id: 'user-alice', display_name: 'Alice' }, today: '28/10' }));
    await expect(chatApi.getAthletePlan('user-alice', 'group-1')).rejects.toThrow('does not match its contract');
  });

  it('reads the subject plan and refuses a body that breaks its contract', async () => {
    const body = {
      athlete: { user_id: 'user-alice', display_name: 'Alice Runner' },
      today: '2026-10-16',
      live: null,
      draft: null,
      draft_state: DRAFT_STATE,
    };
    const get = vi.spyOn(pierreApi.axios, 'get').mockResolvedValueOnce(answer(body));

    await expect(chatApi.getSubjectPlan(CONVERSATION_ID)).resolves.toEqual(body);
    expect(get).toHaveBeenCalledWith(`/api/chat/conversations/${CONVERSATION_ID}/subject-plan`);

    get.mockResolvedValueOnce(answer({ athlete: null }));
    await expect(chatApi.getSubjectPlan(CONVERSATION_ID)).rejects.toThrow('does not match its contract');
  });

  it('publishes with the overwrite flag and reads the 200 as published', async () => {
    const post = vi
      .spyOn(pierreApi.axios, 'post')
      .mockResolvedValueOnce(answer({ plan_id: 'plan-2', superseded_plan_id: 'plan-1', athlete_notified: true }));

    const result = await chatApi.publishSubjectPlan(CONVERSATION_ID, { overwrite_athlete_changes: true });

    expect(result).toEqual({
      status: 'published',
      published: { plan_id: 'plan-2', superseded_plan_id: 'plan-1', athlete_notified: true },
    });
    const [url, body, config] = post.mock.calls[0];
    expect(url).toBe(`/api/chat/conversations/${CONVERSATION_ID}/subject-plan/publish`);
    expect(body).toEqual({ overwrite_athlete_changes: true });
    // The conflict is admitted as an answer, every other refusal still rejects.
    expect(config?.validateStatus?.(409)).toBe(true);
    expect(config?.validateStatus?.(403)).toBe(false);
  });

  it('resolves the publish conflict as a result rather than a thrown error', async () => {
    vi.spyOn(pierreApi.axios, 'post').mockResolvedValueOnce(
      answer(
        {
          error_code: 'plan_publish_conflict',
          conflicts: [{ week_start: '2026-10-19', reason: 'athlete_changed_week' }],
          season_relaid: false,
        },
        409,
      ),
    );

    await expect(chatApi.publishSubjectPlan(CONVERSATION_ID, { overwrite_athlete_changes: false })).resolves.toEqual({
      status: 'conflict',
      conflict: {
        error_code: 'plan_publish_conflict',
        conflicts: [{ week_start: '2026-10-19', reason: 'athlete_changed_week' }],
        season_relaid: false,
      },
    });
  });

  it('rejects a 409 that is not the conflict, carrying the response the classifier reads', async () => {
    vi.spyOn(pierreApi.axios, 'post').mockResolvedValueOnce(answer({ error_code: 'several_tenants' }, 409));

    await expect(
      chatApi.publishSubjectPlan(CONVERSATION_ID, { overwrite_athlete_changes: false }),
    ).rejects.toMatchObject({ response: { status: 409, data: { error_code: 'several_tenants' } } });
  });

  it('discards the draft on its own route', async () => {
    const del = vi.spyOn(pierreApi.axios, 'delete').mockResolvedValueOnce(answer(undefined, 204));

    await chatApi.discardSubjectPlanDraft(CONVERSATION_ID);

    expect(del).toHaveBeenCalledWith(`/api/chat/conversations/${CONVERSATION_ID}/subject-plan/draft`);
  });
});
