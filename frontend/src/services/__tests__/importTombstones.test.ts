// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the import-tombstone clear call both clients make to start fresh with a deleted account's identity
// ABOUTME: The clear posts to /api/user/import-tombstones/clear, naming the tombstone a refusal named or none

import { describe, it, expect, vi } from 'vitest'
import type { AxiosInstance } from 'axios'
import { createUserApi } from '@pierre/api-client'

function axiosWith(post: ReturnType<typeof vi.fn>): AxiosInstance {
  return { post } as unknown as AxiosInstance
}

describe('clearImportTombstone', () => {
  it('clears the tombstone a refusal named', async () => {
    const post = vi.fn().mockResolvedValue({ data: { cleared: [], user_status: 'active' } })

    const result = await createUserApi(axiosWith(post)).clearImportTombstone('tomb-1')

    expect(post).toHaveBeenCalledWith('/api/user/import-tombstones/clear', { tombstone_id: 'tomb-1' })
    expect(result.user_status).toBe('active')
  })

  it('without an id, clears the one naming the account email', async () => {
    const post = vi.fn().mockResolvedValue({ data: { cleared: [], user_status: 'pending' } })

    await createUserApi(axiosWith(post)).clearImportTombstone()

    expect(post).toHaveBeenCalledWith('/api/user/import-tombstones/clear', {})
  })
})
