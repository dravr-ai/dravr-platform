// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for the coach replies domain — the page query it sends, the flag note it trims, the ids it encodes (carnet#1304)
// ABOUTME: Runs the real domain over a stub axios, so web and mobile share what is pinned here

import { describe, expect, it, vi } from 'vitest';
import type { AxiosInstance } from 'axios';
import { createCoachRepliesApi } from '../src/domains/coachReplies';

function stubAxios() {
  const get = vi.fn().mockResolvedValue({ data: { ok: 'get' } });
  const post = vi.fn().mockResolvedValue({ data: { ok: 'post' } });
  return { get, post, api: createCoachRepliesApi({ get, post } as unknown as AxiosInstance) };
}

describe('coachReplies domain', () => {
  it('sends only the filters that are set on a page read', async () => {
    const { get, api } = stubAxios();
    await api.list({ athlete_id: 'a1', before: '', limit: 20 });
    expect(get).toHaveBeenCalledWith('/api/coach/replies', { params: { athlete_id: 'a1', limit: 20 } });
  });

  it('reads the first page with no params at all', async () => {
    const { get, api } = stubAxios();
    await api.list();
    expect(get).toHaveBeenCalledWith('/api/coach/replies', { params: {} });
  });

  it('trims a flag note and drops a blank one', async () => {
    const { post, api } = stubAxios();
    await api.flag('r 1', 'wrong', '  too sure  ');
    expect(post).toHaveBeenLastCalledWith('/api/coach/replies/r%201/flags', { reason: 'wrong', note: 'too sure' });
    await api.flag('r1', 'unsafe', '   ');
    expect(post).toHaveBeenLastCalledWith('/api/coach/replies/r1/flags', { reason: 'unsafe', note: undefined });
  });

  it('posts a correction to the reply it corrects', async () => {
    const { post, api } = stubAxios();
    await api.correct('r1', 'Ran 2:51, not 2:41');
    expect(post).toHaveBeenCalledWith('/api/coach/replies/r1/corrections', { content: 'Ran 2:51, not 2:41' });
  });

  it('asks for the weekly counts, with the week span when given', async () => {
    const { get, api } = stubAxios();
    await api.weekly(1);
    expect(get).toHaveBeenLastCalledWith('/api/coach/replies/weekly', { params: { weeks: 1 } });
    await api.weekly();
    expect(get).toHaveBeenLastCalledWith('/api/coach/replies/weekly', { params: undefined });
  });
});
