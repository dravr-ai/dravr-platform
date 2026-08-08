// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests that the social feed client paginates with the offset parameter the server reads
// ABOUTME: A cursor parameter is silently dropped by the endpoint and re-serves page one forever

import { describe, it, expect, vi } from 'vitest'
import type { AxiosInstance } from 'axios'
import type { FeedResponse } from '@pierre/shared-types'
import { createSocialApi } from '@pierre/api-client'

const metadata = { timestamp: '2024-06-01T10:00:00Z', api_version: 'v1' }

/**
 * Builds a social API bound to an axios stub that records every GET url and
 * replays `pages` in call order, so a request sequence can be asserted end to end.
 */
function socialApiOver(pages: FeedResponse[]) {
  const urls: string[] = []
  let call = 0
  const axiosStub = {
    get: vi.fn((url: string) => {
      urls.push(url)
      const data = pages[Math.min(call, pages.length - 1)]
      call += 1
      return Promise.resolve({ data })
    }),
  } as unknown as AxiosInstance

  return { api: createSocialApi(axiosStub), urls }
}

const emptyPage: FeedResponse = {
  items: [],
  next_cursor: null,
  has_more: false,
  metadata,
}

describe('social feed pagination', () => {
  it('sends the cursor as the offset parameter the feed endpoint reads', async () => {
    const { api, urls } = socialApiOver([emptyPage])

    await api.getFeed({ cursor: '20', limit: 20 })

    expect(urls).toEqual(['/api/social/feed?offset=20&limit=20'])
  })

  it('never sends a cursor parameter, which the endpoint would drop', async () => {
    const { api, urls } = socialApiOver([emptyPage])

    await api.getFeed({ cursor: '40', limit: 20 })

    expect(urls[0]).not.toContain('cursor=')
  })

  it('omits offset entirely on the first page', async () => {
    const { api, urls } = socialApiOver([emptyPage])

    await api.getFeed({ limit: 20 })

    expect(urls).toEqual(['/api/social/feed?limit=20'])
  })

  it('advances past page one when the next_cursor is followed', async () => {
    const pageOne: FeedResponse = {
      items: [],
      next_cursor: '2',
      has_more: true,
      metadata,
    }
    const pageTwo: FeedResponse = {
      items: [],
      next_cursor: '4',
      has_more: true,
      metadata,
    }
    const { api, urls } = socialApiOver([pageOne, pageTwo])

    const first = await api.getFeed({ limit: 2 })
    const second = await api.getFeed({ cursor: first.next_cursor ?? undefined, limit: 2 })

    expect(urls).toEqual(['/api/social/feed?limit=2', '/api/social/feed?offset=2&limit=2'])
    expect(urls[0]).not.toEqual(urls[1])
    expect(second.next_cursor).toBe('4')
  })
})
