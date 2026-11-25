// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for the morning brief calls on the coach roster domain — the read, the change and the open ping (carnet#1791)
// ABOUTME: Runs the real domain over a stub axios, so web and mobile share the paths and bodies pinned here

import { describe, expect, it, vi } from 'vitest';
import type { AxiosInstance } from 'axios';
import { createCoachRosterApi } from '../src/domains/coachRoster';

const BRIEF = { state: 'active', hour: 7, reasons_off: [], muted: [], last_run: null, next_run: null };

function stubAxios() {
  const get = vi.fn().mockResolvedValue({ data: BRIEF });
  const put = vi.fn().mockResolvedValue({ data: { ...BRIEF, hour: 6 } });
  const post = vi.fn().mockResolvedValue({ status: 204, data: '' });
  return { get, put, post, api: createCoachRosterApi({ get, put, post } as unknown as AxiosInstance) };
}

describe('coach brief calls', () => {
  it('reads the brief', async () => {
    const { get, api } = stubAxios();
    await expect(api.getBrief()).resolves.toEqual(BRIEF);
    expect(get).toHaveBeenCalledWith('/api/coach/brief');
  });

  it('puts only the fields it is given, and answers with the stored brief', async () => {
    const { put, api } = stubAxios();
    await expect(api.updateBrief({ hour: 6 })).resolves.toMatchObject({ hour: 6 });
    expect(put).toHaveBeenCalledWith('/api/coach/brief', { hour: 6 });
    await api.updateBrief({ muted: ['u-a', 'u-b'], reasons_off: ['plan_ended'] });
    expect(put).toHaveBeenLastCalledWith('/api/coach/brief', { muted: ['u-a', 'u-b'], reasons_off: ['plan_ended'] });
  });

  it('posts the athlete the coach opened', async () => {
    const { post, api } = stubAxios();
    await expect(api.markBriefOpened('u-a')).resolves.toBeUndefined();
    expect(post).toHaveBeenCalledWith('/api/coach/brief/opened', { athlete_user_id: 'u-a' });
  });
});
