// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for SciotteLoginModal's server-driven timeout copy and its busy-scraper retry wait
// ABOUTME: Covers formatTimeout (minutes vs seconds, in the reader's language) and the retry honouring the refusal's details.retry_after_secs

import { describe, it, expect, vi, afterEach } from 'vitest'
import type { AxiosInstance } from 'axios'
import { createOAuthApi } from '@pierre/api-client'
import { i18n } from '@pierre/i18n'
import { formatTimeout } from '../sciotteLoginCopy'

describe('formatTimeout', () => {
  afterEach(async () => {
    await i18n.changeLanguage('en')
  })

  const en = (secs: number | null) => formatTimeout(secs, i18n.t, 'en')

  it('renders short budgets in seconds', () => {
    expect(en(30)).toBe('30 seconds')
    expect(en(89)).toBe('89 seconds')
    expect(en(1)).toBe('1 second')
  })

  it('collapses budgets >= 90s into whole minutes', () => {
    expect(en(240)).toBe('4 minutes')
    expect(en(210)).toBe('4 minutes')
    expect(en(180)).toBe('3 minutes')
  })

  it('switches units at the 90s boundary', () => {
    expect(en(90)).toBe('2 minutes')
    expect(en(60)).toBe('60 seconds')
  })

  it('names an unknown budget without a number', () => {
    expect(en(null)).toBe('a few minutes')
  })

  it('speaks the reader language, never English inside a French sentence', async () => {
    await i18n.changeLanguage('fr')
    expect(formatTimeout(180, i18n.t, 'fr')).toBe('3 minutes')
    expect(formatTimeout(45, i18n.t, 'fr')).toBe('45 secondes')
    expect(formatTimeout(0, i18n.t, 'fr')).toBe('0 seconde')
    expect(formatTimeout(null, i18n.t, 'fr')).toBe('quelques minutes')
  })
})

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
