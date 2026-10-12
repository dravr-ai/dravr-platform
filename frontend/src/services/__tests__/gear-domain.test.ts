// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the shared @pierre/api-client gear domain the Shoes pane and the activity picker call
// ABOUTME: Pins each route and verb, the bodies sent, a declared "none" as null, and a malformed answer as an error

import { describe, it, expect, vi } from 'vitest'
import type { AxiosInstance } from 'axios'
import { createGearApi } from '@pierre/api-client'
import type { GearItem } from '@pierre/shared-types'

const PAIR: GearItem = {
  id: '6f1c2a5e-3b1d-4c6a-9a7e-2f4b5c6d7e8f',
  kind: 'shoe',
  name: 'Pegasus 41',
  sport: 'run',
  initial_distance_meters: 0,
  warn_distance_meters: 500_000,
  distance_meters: 42_000,
  activity_count: 4,
  warn_reached: false,
  pinned: false,
  is_default: true,
  added_at: '2026-09-01T07:00:00Z',
  retired_at: null,
  last_used_at: '2026-10-08T06:30:00Z',
}

function axiosStub(answer: unknown) {
  const respond = () => Promise.resolve({ data: answer })
  const stub = {
    get: vi.fn(respond),
    post: vi.fn(respond),
    patch: vi.fn(respond),
    put: vi.fn(respond),
    delete: vi.fn(respond),
  }
  return { stub, api: createGearApi(stub as unknown as AxiosInstance) }
}

describe('shared gear domain', () => {
  it('reads the ledger from /api/me/gear', async () => {
    const { stub, api } = axiosStub({ gear: [PAIR] })

    await expect(api.listGear()).resolves.toEqual({ gear: [PAIR] })
    expect(stub.get).toHaveBeenCalledWith('/api/me/gear')
  })

  it('declares a pair with only the fields given', async () => {
    const { stub, api } = axiosStub(PAIR)

    await api.createGear({ name: 'Pegasus 41', sport: 'run', warn_distance_meters: 600_000 })

    expect(stub.post).toHaveBeenCalledWith('/api/me/gear', {
      name: 'Pegasus 41',
      sport: 'run',
      warn_distance_meters: 600_000,
    })
  })

  it('changes, retires, restores and deletes one pair at its own address', async () => {
    const { stub, api } = axiosStub(PAIR)

    await api.updateGear(PAIR.id, { pinned: true })
    await api.retireGear(PAIR.id)
    await api.restoreGear(PAIR.id)
    await api.deleteGear(PAIR.id)

    expect(stub.patch).toHaveBeenCalledWith(`/api/me/gear/${PAIR.id}`, { pinned: true })
    expect(stub.post).toHaveBeenNthCalledWith(1, `/api/me/gear/${PAIR.id}/retire`)
    expect(stub.post).toHaveBeenNthCalledWith(2, `/api/me/gear/${PAIR.id}/restore`)
    expect(stub.delete).toHaveBeenCalledWith(`/api/me/gear/${PAIR.id}`)
  })

  it('reads and records an activity pair at the activity address, none as null', async () => {
    const { stub, api } = axiosStub({ gear_id: null, source: 'declared' })

    await api.getActivityGear('intervals_icu', 'i/42')
    const answer = await api.setActivityGear('intervals_icu', 'i/42', null)

    expect(stub.get).toHaveBeenCalledWith('/api/me/activities/intervals_icu/i%2F42/gear')
    expect(stub.put).toHaveBeenCalledWith('/api/me/activities/intervals_icu/i%2F42/gear', { gear_id: null })
    expect(answer).toEqual({ gear_id: null, source: 'declared' })
  })

  it('rejects an answer that does not match the contract', async () => {
    const { api } = axiosStub({ gear: [{ ...PAIR, sport: 'ride' }] })

    await expect(api.listGear()).rejects.toThrow('/api/me/gear answered with a body that does not match its contract')
  })
})
