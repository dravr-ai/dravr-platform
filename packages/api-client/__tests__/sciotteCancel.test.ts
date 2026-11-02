// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for the explicit sciotte sign-in cancel — every step names its attempt, the cancel names it too (carnet#1300)
// ABOUTME: Red if a step drops attempt_id, a step without one sends it, or the best-effort cancel retries a busy answer

import { describe, expect, it, vi } from 'vitest';
import type { AxiosInstance } from 'axios';
import { createOAuthApi } from '../src/domains/oauth';

function stubAxios(post = vi.fn().mockResolvedValue({ data: { status: 'connected' } })) {
  return { post, api: createOAuthApi({ post } as unknown as AxiosInstance) };
}

const ATTEMPT = '0b7e3c1a-5f2d-4e8b-9a61-2c4d8e0f1a3b';

describe('sciotte sign-in — explicit cancel', () => {
  it('sends the attempt id on all three steps', async () => {
    const { post, api } = stubAxios();

    await api.sciotteLogin(
      { email: 'a@example.test', password: 'pw', method: 'email', target: 'garmin' },
      { attemptId: ATTEMPT },
    );
    await api.sciotteSelect2FA('app', { attemptId: ATTEMPT });
    await api.sciotteSubmitOTP('123456', { attemptId: ATTEMPT });

    expect(post.mock.calls.map(([path, body]) => [path, body])).toEqual([
      [
        '/api/providers/sciotte/login',
        { email: 'a@example.test', password: 'pw', method: 'email', target: 'garmin', attempt_id: ATTEMPT },
      ],
      ['/api/providers/sciotte/select-2fa', { option_id: 'app', attempt_id: ATTEMPT }],
      ['/api/providers/sciotte/submit-otp', { code: '123456', attempt_id: ATTEMPT }],
    ]);
  });

  it('sends no attempt id from a step that names none', async () => {
    const { post, api } = stubAxios();

    await api.sciotteSelect2FA('app');
    await api.sciotteSubmitOTP('123456', {});

    expect(post.mock.calls.map(([, body]) => body)).toEqual([{ option_id: 'app' }, { code: '123456' }]);
  });

  it('posts the cancel with the attempt id and reads its answer', async () => {
    const { post, api } = stubAxios(vi.fn().mockResolvedValue({ data: { cancelled: true } }));

    expect(await api.sciotteCancel(ATTEMPT)).toEqual({ cancelled: true });
    expect(post).toHaveBeenCalledTimes(1);
    expect(post.mock.calls[0].slice(0, 2)).toEqual(['/api/providers/sciotte/cancel', { attempt_id: ATTEMPT }]);
  });

  it('never retries a busy cancel', async () => {
    vi.useFakeTimers();
    try {
      const busy = { response: { status: 503, headers: { 'retry-after': '1' }, data: { error: 'busy' } } };
      const post = vi.fn().mockRejectedValue(busy);
      const { api } = stubAxios(post);

      const settled = api.sciotteCancel(ATTEMPT).then(
        () => 'resolved',
        (err: unknown) => err,
      );
      await vi.advanceTimersByTimeAsync(60_000);

      expect(await settled).toBe(busy);
      expect(post).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
