// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for aborting a sciotte sign-in step — the signal reaches the request and ends the busy backoff (carnet#1300)
// ABOUTME: Red if a closed window's step keeps waiting out a 503 backoff and posts again after the abort

import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AxiosInstance } from 'axios';
import { createOAuthApi } from '../src/domains/oauth';

/** The refusal the platform answers while the sciotte scraper sheds load. */
function busy() {
  return { response: { status: 503, headers: { 'retry-after': '30' }, data: { error: 'busy' } } };
}

function stubAxios(post = vi.fn().mockResolvedValue({ data: { status: 'connected' } })) {
  return { post, api: createOAuthApi({ post } as unknown as AxiosInstance) };
}

describe('sciotte sign-in steps — abort', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('hands every step its signal, beside the long timeout', async () => {
    const { post, api } = stubAxios();
    const { signal } = new AbortController();

    await api.sciotteLogin({ email: 'a@example.test', password: 'pw', method: 'email', target: 'garmin' }, { signal });
    await api.sciotteSelect2FA('app', { signal });
    await api.sciotteSubmitOTP('123456', { signal });

    expect(post.mock.calls.map(([path, , config]) => [path, config])).toEqual([
      ['/api/providers/sciotte/login', { timeout: 1_200_000, signal }],
      ['/api/providers/sciotte/select-2fa', { timeout: 1_200_000, signal }],
      ['/api/providers/sciotte/submit-otp', { timeout: 1_200_000, signal }],
    ]);
  });

  it('ends the busy backoff at the abort instead of posting again', async () => {
    vi.useFakeTimers();
    const post = vi.fn().mockRejectedValueOnce(busy()).mockResolvedValue({ data: { status: 'connected' } });
    const { api } = stubAxios(post);
    const controller = new AbortController();

    const step = api.sciotteSubmitOTP('123456', { signal: controller.signal });
    const settled = step.then(
      () => 'resolved',
      (err: unknown) => err,
    );
    // Let the first post fail and the backoff wait begin.
    await vi.advanceTimersByTimeAsync(0);
    expect(post).toHaveBeenCalledTimes(1);

    const reason = new Error('window closed');
    controller.abort(reason);

    expect(await settled).toBe(reason);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(post).toHaveBeenCalledTimes(1);
  });

  it('still retries a busy step nobody aborted', async () => {
    vi.useFakeTimers();
    const post = vi.fn().mockRejectedValueOnce(busy()).mockResolvedValue({ data: { status: 'connected' } });
    const { api } = stubAxios(post);

    const step = api.sciotteSubmitOTP('123456', { signal: new AbortController().signal });
    await vi.advanceTimersByTimeAsync(31_000);

    expect(await step).toEqual({ status: 'connected' });
    expect(post).toHaveBeenCalledTimes(2);
  });
});
