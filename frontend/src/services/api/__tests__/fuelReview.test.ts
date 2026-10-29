// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for the fuel-proposal review writes — approve and reject read each outcome code as a result
// ABOUTME: A 404 or 422 that carries no outcome code (a missing route, an extractor's plain-text refusal) is thrown, never read as a result

import { describe, it, expect, vi, afterEach } from 'vitest';
import type { AxiosResponse } from 'axios';
import { pierreApi } from '../index';
import { adminApi } from '../admin';

const ID = '6f1c2b8e-0000-4000-8000-000000000001';

function answer<T>(data: T, status = 200): AxiosResponse<T> {
  return { data, status, statusText: '', headers: {}, config: {} as AxiosResponse['config'] };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('adminApi fuel-proposal review writes', () => {
  it('reads a gate refusal as an implausible result', async () => {
    const body = { outcome: 'implausible', error: 'fails the gate', violations: [{ code: 'carbs_missing', message: 'x' }] };
    const post = vi.spyOn(pierreApi.axios, 'post').mockResolvedValueOnce(answer(body, 422));

    const result = await adminApi.approveFuelProposal(ID, { carbs_g: null });

    expect(post).toHaveBeenCalledWith(
      `/api/admin/fuel-products/proposals/${ID}/approve`,
      { overrides: { carbs_g: null } },
      expect.anything(),
    );
    expect(result.outcome).toBe('implausible');
  });

  it('throws a 422 the extractor answered in plain text instead of reading it as an outcome', async () => {
    vi.spyOn(pierreApi.axios, 'post').mockResolvedValueOnce(
      answer('Failed to deserialize the JSON body: unknown field `carb_g`', 422),
    );

    await expect(adminApi.approveFuelProposal(ID, {})).rejects.toThrow(/HTTP 422.*unknown field/);
  });

  it('throws a 404 that names no outcome, and reads one that does', async () => {
    vi.spyOn(pierreApi.axios, 'post').mockResolvedValueOnce(answer({ error: 'no route' }, 404));
    await expect(adminApi.rejectFuelProposal(ID, 'illegible')).rejects.toThrow(/HTTP 404/);

    vi.spyOn(pierreApi.axios, 'post').mockResolvedValueOnce(
      answer({ outcome: 'not_found', error: 'No fuel proposal has that id' }, 404),
    );
    const result = await adminApi.rejectFuelProposal(ID, 'illegible');
    expect(result.outcome).toBe('not_found');
  });

  it('refuses an approval outcome code on a rejection', async () => {
    vi.spyOn(pierreApi.axios, 'post').mockResolvedValueOnce(answer({ outcome: 'approved', product: {} }, 200));

    await expect(adminApi.rejectFuelProposal(ID, 'duplicate')).rejects.toThrow(/HTTP 200/);
  });
});
