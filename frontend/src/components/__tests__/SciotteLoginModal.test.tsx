// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit test for the sciotte login's busy-scraper retry wait
// ABOUTME: The retry honours the refusal's details.retry_after_secs when no Retry-After header reaches the client

import { describe, it, expect, vi, afterEach } from 'vitest'
import type { AxiosInstance } from 'axios'
import { createOAuthApi } from '@pierre/api-client'

describe('sciotte login retry on a busy scraper', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.restoreAllMocks()
  })

  it('waits the retry window the refusal names in details before retrying', async () => {
    vi.useFakeTimers()
    // No jitter, so the wait is exactly the larger of the server's window and
    // the first exponential step (1 s).
    vi.spyOn(Math, 'random').mockReturnValue(0)

    const busy = {
      response: {
        status: 503,
        // No Retry-After header reached the client: the body is the only
        // place the wait is named.
        headers: {},
        data: {
          code: 'ExternalRateLimited',
          message: 'Strava login is busy, try again shortly',
          details: { retry_after_secs: 2 },
        },
      },
    }
    const loggedIn = { data: { status: 'success', provider: 'strava' } }
    const post = vi.fn().mockRejectedValueOnce(busy).mockResolvedValueOnce(loggedIn)
    const oauth = createOAuthApi({ post } as unknown as AxiosInstance)

    const login = oauth.sciotteLogin({
      email: 'athlete@example.com',
      password: 'secret',
      method: 'email',
      target: 'strava',
    })

    await vi.advanceTimersByTimeAsync(0)
    expect(post).toHaveBeenCalledTimes(1)

    await vi.advanceTimersByTimeAsync(1_999)
    expect(post).toHaveBeenCalledTimes(1)

    await vi.advanceTimersByTimeAsync(1)
    expect(post).toHaveBeenCalledTimes(2)
    await expect(login).resolves.toEqual(loggedIn.data)
  })
})
